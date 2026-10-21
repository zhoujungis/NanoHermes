//! End-to-end integration test: drive a real `Agent` against a local mock
//! OpenAI-compatible server.
//!
//! Everything below the HTTP boundary is the real code path — `llm::chat`,
//! SSE parsing, tool execution, session bookkeeping — so this catches wiring
//! bugs that unit tests on individual functions cannot. The mock server is
//! built from `std::net` only: no new dependency is pulled in.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use agent::ui::UiEvent;
use agent::{Agent, Config};
use serde_json::{json, Value};

/* ------------------------------------------------------------- mock server */

/// A fixed set of canned HTTP responses, served one per connection in order.
struct MockLlm {
    port: u16,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockLlm {
    fn start(responses: Vec<String>) -> MockLlm {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();

        let handle = thread::spawn(move || {
            let total = responses.len();
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut served = 0usize;
            while served < total {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Accept() handed us a non-blocking socket; reads must block.
                        stream.set_nonblocking(false).ok();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(10)))
                            .ok();
                        read_full_request(&mut stream);
                        let _ = stream.write_all(responses[served].as_bytes());
                        let _ = stream.flush();
                        served += 1;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() > deadline {
                            break; // the agent made fewer requests than expected
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        MockLlm {
            port,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    /// Wait for the server thread (it self-terminates on its own deadline).
    fn finish(mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Read one full HTTP request (headers + `Content-Length` body) off the socket.
fn read_full_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = stream.read(&mut tmp).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(head_end) = find_subslice(&buf, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..head_end]).to_lowercase();
            let len = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= head_end + 4 + len {
                break;
            }
        }
    }
    buf
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// A complete SSE (`text/event-stream`) response carrying the given data frames.
fn sse_response(frames: &[Value]) -> String {
    let mut out = String::from(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
    );
    for f in frames {
        out.push_str("data: ");
        out.push_str(&f.to_string());
        out.push_str("\r\n\r\n");
    }
    out.push_str("data: [DONE]\r\n\r\n");
    out
}

/// A complete non-streaming JSON response.
fn json_response(v: &Value) -> String {
    let body = v.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn status_response(code: u16, reason: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

/* ----------------------------------------------------------------- helpers */

/// A fresh, empty directory under the system temp dir. The name is built from
/// process id, a monotonic counter and nanoseconds so parallel tests never
/// collide — and it stays a legal Windows file name (no braces or spaces).
fn scratch(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);

    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "nanohermes-e2e-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cfg_for(base_url: String, workspace: PathBuf) -> Config {
    Config {
        api_key: "test-key".to_string(),
        base_url,
        model: "mock-model".to_string(),
        stream: true,
        workspace,
        auto_approve: true,
        max_retries: 0,
        retry_base_ms: 1,
        timeout_secs: 10,
        project_docs: false,
        memory: false,
        ..Config::default()
    }
}

/// Route user-facing output to a no-op sink so the tests do not spew tool
/// banners and streamed text into the test log.
fn silent(agent: &mut Agent) {
    agent.set_ui_sink(std::sync::Arc::new(|_: UiEvent| {}));
}

/* ------------------------------------------------------------------- tests */

#[test]
fn runs_a_tool_call_then_reports_the_final_answer() {
    // Turn 1: the model asks for `write_file`, seeded as two SSE fragments so
    // the incremental tool-call reassembly is exercised too.
    // Turn 2: the model has the tool result and answers in plain text.
    let ws = scratch("tool");
    let file = ws.join("hello.txt");

    let stream = sse_response(&[
        json!({ "choices": [ { "delta": { "tool_calls": [ {
            "index": 0, "id": "call_1", "type": "function",
            "function": { "name": "write_file", "arguments": "{\"path\":\"hello.txt\"," } } ] } } ] }),
        json!({ "choices": [ { "delta": { "tool_calls": [ {
            "index": 0, "function": { "arguments": "\"content\":\"hi from test\"}" } } ] } } ] }),
        json!({ "choices": [ { "delta": {}, "finish_reason": "tool_calls" } ],
                "usage": { "prompt_tokens": 10, "completion_tokens": 5 } }),
    ]);

    let done = sse_response(&[
        json!({ "choices": [ { "delta": { "content": "done" } } ] }),
        json!({ "choices": [ { "delta": {}, "finish_reason": "stop" } ],
                "usage": { "prompt_tokens": 20, "completion_tokens": 3 } }),
    ]);

    let mock = MockLlm::start(vec![stream, done]);
    let mut agent = Agent::with_mcp(cfg_for(mock.base_url(), ws.clone()), None);
    silent(&mut agent);

    let answer = agent.run(json!("write a file")).expect("turn completes");
    mock.finish();

    assert_eq!(answer, "done");
    assert_eq!(
        std::fs::read_to_string(&file).expect("file was written"),
        "hi from test"
    );

    // Two model requests happened; the reported usage is their sum.
    let usage = agent.usage();
    assert_eq!(usage.requests, 2);
    assert_eq!(usage.prompt_tokens, 30);
    assert_eq!(usage.completion_tokens, 8);

    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn streams_plain_text_without_touching_any_tool() {
    let ws = scratch("text");
    let stream = sse_response(&[
        json!({ "choices": [ { "delta": { "content": "Hel" } } ] }),
        json!({ "choices": [ { "delta": { "content": "lo" } } ] }),
        json!({ "choices": [ { "delta": {}, "finish_reason": "stop" } ],
                "usage": { "prompt_tokens": 4, "completion_tokens": 2 } }),
    ]);

    let mock = MockLlm::start(vec![stream]);
    let mut agent = Agent::with_mcp(cfg_for(mock.base_url(), ws.clone()), None);
    silent(&mut agent);

    let answer = agent.run(json!("hi")).expect("turn completes");
    mock.finish();

    assert_eq!(answer, "Hello");
    assert_eq!(agent.usage().requests, 1);

    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn reads_a_non_streaming_json_response() {
    let ws = scratch("json");
    let body = json_response(&json!({
        "choices": [ { "message": { "content": "plain" } } ],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1 },
    }));

    let mock = MockLlm::start(vec![body]);
    let mut cfg = cfg_for(mock.base_url(), ws.clone());
    cfg.stream = false;
    let mut agent = Agent::with_mcp(cfg, None);
    silent(&mut agent);

    let answer = agent.run(json!("hi")).expect("turn completes");
    mock.finish();

    assert_eq!(answer, "plain");

    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn retries_a_transient_server_error_then_succeeds() {
    let ws = scratch("retry");
    let boom = status_response(500, "Internal Server Error", "boom");
    let ok = json_response(&json!({
        "choices": [ { "message": { "content": "recovered" } } ],
    }));

    let mock = MockLlm::start(vec![boom, ok]);
    let mut cfg = cfg_for(mock.base_url(), ws.clone());
    cfg.stream = false;
    cfg.max_retries = 1;

    let mut agent = Agent::with_mcp(cfg, None);
    silent(&mut agent);
    let answer = agent.run(json!("hi")).expect("turn completes");
    mock.finish();

    assert_eq!(answer, "recovered");

    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn surfaces_a_hard_api_error_without_hanging() {
    let ws = scratch("error");
    let err = json_response(&json!({ "error": { "message": "bad key" } }));

    let mock = MockLlm::start(vec![err]);
    let mut cfg = cfg_for(mock.base_url(), ws.clone());
    cfg.stream = false;

    let mut agent = Agent::with_mcp(cfg, None);
    silent(&mut agent);
    let result = agent.run(json!("hi"));
    mock.finish();

    let e = result.expect_err("an API error should propagate");
    assert!(e.contains("bad key"), "unexpected error: {e}");

    let _ = std::fs::remove_dir_all(&ws);
}
