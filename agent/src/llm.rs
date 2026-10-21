use std::io::{BufRead, BufReader};
use std::time::Duration;

use serde_json::{json, Value};

use crate::config::Config;

/// A single tool invocation requested by the model.
#[derive(Default, Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON string of the arguments (may be empty).
    pub arguments: String,
}

/// Token counts reported by the API for one request.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// Extract `usage` from a chat-completions payload (absent fields count as 0).
pub fn usage_from_value(v: &Value) -> Usage {
    let u = v.get("usage");
    let get = |k: &str| {
        u.and_then(|u| u.get(k))
            .and_then(|x| x.as_u64())
            .unwrap_or(0)
    };
    Usage {
        prompt_tokens: get("prompt_tokens"),
        completion_tokens: get("completion_tokens"),
    }
}

/// One assistant response: optional text plus zero or more tool calls.
#[derive(Default, Debug)]
pub struct AssistantTurn {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
}

/// Call `/chat/completions`. When `cfg.stream` is true, text deltas are handed to
/// `on_text` as they arrive. Returns the fully assembled assistant turn.
/// If a `fallback_model` is configured and the primary model returns a 429 or
/// 5xx status, the request is retried once with the fallback model.
pub fn chat<F: FnMut(&str)>(
    cfg: &Config,
    messages: &[Value],
    tools: &Value,
    mut on_text: F,
) -> Result<AssistantTurn, String> {
    let has_tools = tools.as_array().map(|a| !a.is_empty()).unwrap_or(false);

    let build_body = |model: &str| {
        let mut body = json!({
            "model": model,
            "messages": messages,
            "temperature": cfg.temperature,
            "max_tokens": cfg.max_tokens,
            "stream": cfg.stream,
        });
        if has_tools {
            body["tools"] = tools.clone();
            body["tool_choice"] = "auto".into();
        }
        if cfg.thinking {
            body["reasoning_effort"] = "high".into();
        }
        if cfg.stream {
            // Ask for a trailing usage frame so token counts work while streaming.
            body["stream_options"] = json!({ "include_usage": true });
        }
        body
    };

    // Retry transient failures with exponential backoff; once retries are
    // exhausted, fall back to `fallback_model` (if configured).
    let mut attempt: u32 = 0;
    loop {
        match chat_with_model(cfg, build_body(&cfg.model), &mut on_text) {
            Ok(turn) => return Ok(turn),
            Err(e) => {
                if is_retryable_error(&e) && attempt < cfg.max_retries {
                    attempt += 1;
                    let delay = backoff_ms(cfg.retry_base_ms, attempt);
                    eprintln!(
                        "note: request failed ({e}); retrying in {delay}ms (attempt {attempt}/{})",
                        cfg.max_retries
                    );
                    std::thread::sleep(Duration::from_millis(delay));
                    continue;
                }
                if let Some(fallback) = &cfg.fallback_model {
                    if is_retryable_error(&e) {
                        eprintln!("note: primary model failed ({e}); retrying with fallback model `{fallback}`");
                        return chat_with_model(cfg, build_body(fallback), &mut on_text);
                    }
                }
                return Err(e);
            }
        }
    }
}

/// Delay before retry `attempt` (1-based): `base * 2^(attempt-1)`, capped at 30s.
fn backoff_ms(base_ms: u64, attempt: u32) -> u64 {
    base_ms
        .saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)))
        .min(30_000)
}

/// Whether an error string indicates a transient server-side failure worth
/// retrying (HTTP 429 or 5xx).
fn is_retryable_error(e: &str) -> bool {
    e.contains("HTTP 429")
        || e.contains("HTTP 500")
        || e.contains("HTTP 502")
        || e.contains("HTTP 503")
        || e.contains("HTTP 504")
}

fn chat_with_model<F: FnMut(&str)>(
    cfg: &Config,
    body: Value,
    on_text: &mut F,
) -> Result<AssistantTurn, String> {
    let mut builder = ureq::AgentBuilder::new().timeout(Duration::from_secs(cfg.timeout_secs));
    if let Some(p) = proxy_for(&cfg.endpoint()) {
        builder = builder.proxy(p);
    }
    let agent = builder.build();

    let req = agent
        .post(&cfg.endpoint())
        .set("Content-Type", "application/json")
        .set("Authorization", &format!("Bearer {}", cfg.api_key));

    let resp = match req.send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_string().unwrap_or_default();
            return Err(format!("HTTP {code} from API: {}", truncate(&detail, 800)));
        }
        Err(e) => return Err(format!("request failed: {e}")),
    };

    if cfg.stream {
        read_stream(resp, on_text)
    } else {
        read_json(resp, on_text)
    }
}

fn read_json(
    resp: ureq::Response,
    on_text: &mut impl FnMut(&str),
) -> Result<AssistantTurn, String> {
    let v: Value = resp
        .into_json()
        .map_err(|e| format!("invalid JSON response: {e}"))?;

    if let Some(err) = v.get("error") {
        return Err(format!("API error: {err}"));
    }

    let msg = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .ok_or_else(|| {
            format!(
                "unexpected response shape: {}",
                truncate(&v.to_string(), 400)
            )
        })?;

    let mut turn = AssistantTurn::default();
    if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
        turn.content.push_str(text);
        on_text(text);
    }
    if let Some(tcs) = msg.get("tool_calls").and_then(|t| t.as_array()) {
        for tc in tcs {
            turn.tool_calls.push(ToolCall {
                id: tc
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string(),
                arguments: tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    turn.usage = usage_from_value(&v);
    Ok(turn)
}

fn read_stream(
    resp: ureq::Response,
    on_text: &mut impl FnMut(&str),
) -> Result<AssistantTurn, String> {
    let mut turn = AssistantTurn::default();
    let reader = BufReader::new(resp.into_reader());
    let mut line = String::new();

    for l in reader.lines() {
        line.clear();
        let l = match l {
            Ok(l) => l,
            Err(e) => return Err(format!("stream read error: {e}")),
        };
        let l = l.trim_end_matches(['\r', '\n']);
        if l.is_empty() || l.starts_with(':') {
            continue; // keep-alive / comment
        }
        let data = match l.strip_prefix("data:") {
            Some(d) => d.trim(),
            None => continue, // event/id/retry fields
        };
        if data == "[DONE]" {
            break;
        }
        let v: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue, // tolerate partial / unknown frames
        };
        if let Some(err) = v.get("error") {
            return Err(format!("API error: {err}"));
        }
        if v.get("usage").is_some() {
            turn.usage = usage_from_value(&v);
        }
        let Some(delta) = v
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("delta"))
        else {
            continue;
        };
        if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
            if !text.is_empty() {
                turn.content.push_str(text);
                on_text(text);
            }
        }
        accumulate_tool_calls(delta.get("tool_calls"), &mut turn.tool_calls);
    }

    Ok(turn)
}

/// OpenAI streams tool calls incrementally: each delta carries an `index` plus a
/// fragment of `function.arguments`. We merge fragments into a slot per index.
fn accumulate_tool_calls(tcs: Option<&Value>, out: &mut Vec<ToolCall>) {
    let Some(arr) = tcs.and_then(|t| t.as_array()) else {
        return;
    };
    for tc in arr {
        let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
        if out.len() <= idx {
            out.resize(idx + 1, ToolCall::default());
        }
        let slot = &mut out[idx];
        if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
            if !id.is_empty() {
                slot.id = id.to_string();
            }
        }
        if let Some(f) = tc.get("function") {
            if let Some(name) = f.get("name").and_then(|n| n.as_str()) {
                if !name.is_empty() {
                    slot.name = name.to_string();
                }
            }
            if let Some(args) = f.get("arguments").and_then(|a| a.as_str()) {
                slot.arguments.push_str(args);
            }
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… ({} bytes truncated)", &s[..end], s.len() - end)
}

/// Honour standard proxy env vars for public endpoints, while bypassing them for
/// hosts listed in NO_PROXY (and localhost). Sandboxes often expose an egress
/// proxy this way, so the agent works out of the box without extra configuration.
pub fn proxy_for(url: &str) -> Option<ureq::Proxy> {
    let host = url.split("://").nth(1)?.split(['/', ':']).next()?;
    if bypass_proxy(host) {
        return None;
    }
    let raw = [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .iter()
    .find_map(|k| std::env::var(k).ok())
    .filter(|v| !v.trim().is_empty())?;
    ureq::Proxy::new(raw.trim()).ok()
}

fn bypass_proxy(host: &str) -> bool {
    if host == "localhost" || host == "::1" || host.starts_with("127.") {
        return true;
    }
    let list = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();
    list.split(',')
        .map(|s| s.trim().trim_start_matches('.'))
        .any(|e| !e.is_empty() && (host == e || host.ends_with(&format!(".{e}")) || e == "*"))
}

/// Summarize a chunk of conversation into a short paragraph. Used to compact
/// long-running sessions without losing important state. Returns the plain
/// summary text (no tools, streaming is disabled for this internal call).
pub fn summarize(cfg: &Config, text: &str) -> Result<String, String> {
    let mut summarizer_cfg = cfg.clone();
    summarizer_cfg.stream = false;
    let system = "You are a conversation summarizer for a coding agent. Write a concise summary of the excerpt below, preserving: the user's goal, key decisions, file paths changed, and current task state. Use at most 300 words. Reply with the summary only.";
    let messages = vec![
        json!({ "role": "system", "content": system }),
        json!({ "role": "user", "content": text }),
    ];
    let turn = chat(&summarizer_cfg, &messages, &Value::Array(vec![]), |_| {})?;
    Ok(turn.content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage_from_payload() {
        let v = json!({
            "usage": { "prompt_tokens": 120, "completion_tokens": 42, "total_tokens": 162 }
        });
        let u = usage_from_value(&v);
        assert_eq!(u.prompt_tokens, 120);
        assert_eq!(u.completion_tokens, 42);

        // Missing usage (e.g. a stream delta) counts as zero, not an error.
        assert_eq!(
            usage_from_value(&json!({ "choices": [] })),
            Usage::default()
        );
    }

    #[test]
    fn retryable_errors_are_recognised() {
        assert!(is_retryable_error("HTTP 429 from API: rate limited"));
        assert!(is_retryable_error("HTTP 503 from API: unavailable"));
        assert!(!is_retryable_error("HTTP 401 from API: bad key"));
        assert!(!is_retryable_error("request failed: dns error"));
    }

    #[test]
    fn backoff_grows_exponentially_and_is_capped() {
        assert_eq!(backoff_ms(500, 1), 500);
        assert_eq!(backoff_ms(500, 2), 1000);
        assert_eq!(backoff_ms(500, 3), 2000);
        assert_eq!(backoff_ms(500, 0), 500); // attempt 0 is treated as the first
        assert_eq!(backoff_ms(20_000, 5), 30_000); // capped at 30s
        assert_eq!(backoff_ms(1, 64), 30_000); // no overflow panic
    }
}
