//! Web access tools: `web_fetch` (read a URL as plain text) and `web_search`
//! (query DuckDuckGo and return the top results). Both use the blocking `ureq`
//! client and honour the standard proxy environment variables.

use std::io::Read;
use std::time::Duration;

use serde_json::Value;

use super::Ctx;

/// Hard ceiling on the response body we keep in memory (the rest is drained).
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const FETCH_TIMEOUT_SECS: u64 = 30;
const SEARCH_TIMEOUT_SECS: u64 = 20;
const USER_AGENT: &str = "Mozilla/5.0 (compatible; nanohermes)";

/// Fetch a URL and return its content as readable text. HTML pages are reduced
/// to plain text (tags stripped, entities decoded, whitespace collapsed).
pub fn web_fetch(args: &Value, _ctx: &Ctx) -> Result<String, String> {
    let url = req_str(args, "url")?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("`url` must start with http:// or https://".to_string());
    }
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(20_000)
        .clamp(500, 200_000) as usize;

    let (ctype, bytes, total) = http_get(url, FETCH_TIMEOUT_SECS)?;
    let body = String::from_utf8_lossy(&bytes);
    let is_html = ctype.to_ascii_lowercase().contains("html")
        || (ctype.is_empty() && body.trim_start().starts_with('<'));
    let text = if is_html {
        html_to_text(&body)
    } else {
        body.trim().to_string()
    };

    let mut out = format!("URL: {url}\n");
    if total > bytes.len() {
        out.push_str(&format!(
            "[response truncated at {MAX_RESPONSE_BYTES} bytes]\n"
        ));
    }
    out.push_str(&clip_chars(&text, max_chars));
    Ok(out)
}

/// Search the web (DuckDuckGo HTML endpoint) and return the top results.
pub fn web_search(args: &Value, _ctx: &Ctx) -> Result<String, String> {
    let query = req_str(args, "query")?;
    if query.trim().is_empty() {
        return Err("`query` must not be empty".to_string());
    }
    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 10) as usize;

    let url = format!("https://html.duckduckgo.com/html/?q={}", url_encode(query));
    let (_, bytes, _) = http_get(&url, SEARCH_TIMEOUT_SECS)?;
    let html = String::from_utf8_lossy(&bytes);
    let results = parse_ddg_results(&html, count);
    if results.is_empty() {
        return Ok(format!("no web results for `{query}`"));
    }

    let mut out = String::new();
    for (i, r) in results.iter().enumerate() {
        out.push_str(&format!("{}. {}\n   {}\n", i + 1, r.title, r.url));
        if !r.snippet.is_empty() {
            out.push_str(&format!("   {}\n", r.snippet));
        }
    }
    Ok(out.trim_end().to_string())
}

/// GET `url` and return `(content_type, kept_bytes, total_bytes)`.
fn http_get(url: &str, timeout_secs: u64) -> Result<(String, Vec<u8>, usize), String> {
    let mut builder = ureq::AgentBuilder::new().timeout(Duration::from_secs(timeout_secs));
    if let Some(p) = crate::llm::proxy_for(url) {
        builder = builder.proxy(p);
    }
    let agent = builder.build();

    let resp = match agent.get(url).set("User-Agent", USER_AGENT).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_string().unwrap_or_default();
            return Err(format!(
                "HTTP {code} fetching {url}: {}",
                clip_chars(detail.trim(), 300)
            ));
        }
        Err(e) => return Err(format!("request to {url} failed: {e}")),
    };

    let ctype = resp.header("Content-Type").unwrap_or("").to_string();
    let mut reader = resp.into_reader();
    let mut kept: Vec<u8> = Vec::new();
    let mut total = 0usize;
    let mut buf = [0u8; 16 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if kept.len() < MAX_RESPONSE_BYTES {
                    let room = MAX_RESPONSE_BYTES - kept.len();
                    kept.extend_from_slice(&buf[..n.min(room)]);
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("error reading response from {url}: {e}")),
        }
    }
    Ok((ctype, kept, total))
}

// ---------------------------------------------------------------------------
// HTML → text
// ---------------------------------------------------------------------------

/// Reduce an HTML document to readable plain text.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let bytes = html.as_bytes();
    let n = bytes.len();
    let mut i = 0;

    while i < n {
        if bytes[i] == b'<' {
            // Comments are dropped whole.
            if html[i..].starts_with("<!--") {
                match html[i..].find("-->") {
                    Some(rel) => {
                        i += rel + 3;
                        continue;
                    }
                    None => break,
                }
            }
            let Some(end) = html[i..].find('>').map(|rel| i + rel) else {
                break;
            };
            let tag = &html[i + 1..end];
            let name = tag_name(tag);
            let closing = tag.trim_start().starts_with('/');

            if !closing && is_skip_tag(&name) {
                // Skip the whole raw-text element body (e.g. <script>…</script>).
                let close = format!("</{name}");
                match find_ci(&html[i..], &close) {
                    Some(rel) => {
                        i += rel + close.len();
                        continue;
                    }
                    None => break,
                }
            } else if is_block_tag(&name) {
                out.push('\n');
            }
            i = end + 1;
            continue;
        }

        let start = i;
        while i < n && bytes[i] != b'<' {
            i += 1;
        }
        push_decoded(&html[start..i], &mut out);
    }

    normalize_whitespace(&out)
}

/// Case-insensitive substring search over ASCII tags.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let h = hay.as_bytes();
    let nd = needle.as_bytes();
    if nd.is_empty() || h.len() < nd.len() {
        return None;
    }
    'out: for i in 0..=h.len() - nd.len() {
        for j in 0..nd.len() {
            if !h[i + j].eq_ignore_ascii_case(&nd[j]) {
                continue 'out;
            }
        }
        return Some(i);
    }
    None
}

/// Lowercased tag name, dropping a leading `/` and any attributes.
fn tag_name(tag: &str) -> String {
    tag.trim_start()
        .trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn is_skip_tag(name: &str) -> bool {
    matches!(name, "script" | "style" | "noscript" | "template" | "head")
}

fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "p" | "br"
            | "div"
            | "li"
            | "tr"
            | "ul"
            | "ol"
            | "table"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "blockquote"
            | "pre"
            | "hr"
            | "td"
            | "th"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
    )
}

/// Append `run` to `out`, decoding HTML entities (`&amp;`, `&#39;`, `&#x1F;`, …).
fn push_decoded(run: &str, out: &mut String) {
    let chars: Vec<char> = run.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '&' {
            let mut ent = String::new();
            let mut j = i + 1;
            while j < chars.len() && ent.len() < 10 {
                let nc = chars[j];
                if nc == ';' {
                    break;
                }
                if nc == '&' || nc.is_whitespace() || nc == '<' || nc == '>' {
                    break;
                }
                ent.push(nc);
                j += 1;
            }
            if j < chars.len() && chars[j] == ';' && !ent.is_empty() {
                if let Some(ch) = decode_entity(&ent) {
                    out.push(ch);
                    i = j + 1;
                    continue;
                }
            }
            out.push('&');
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
}

fn decode_entity(ent: &str) -> Option<char> {
    match ent {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = ent.strip_prefix('#') {
                dec.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                None
            }
        }
    }
}

/// Collapse runs of spaces/tabs and blank lines so the text reads cleanly.
fn normalize_whitespace(s: &str) -> String {
    let mut out = String::new();
    let mut blank = 0usize;
    for line in s.split('\n') {
        let mut collapsed = String::new();
        let mut prev_space = false;
        for ch in line.chars() {
            if ch == ' ' || ch == '\t' || ch == '\r' {
                if !prev_space {
                    collapsed.push(' ');
                }
                prev_space = true;
            } else {
                collapsed.push(ch);
                prev_space = false;
            }
        }
        let t = collapsed.trim();
        if t.is_empty() {
            blank += 1;
            if blank <= 1 {
                out.push('\n');
            }
        } else {
            out.push_str(t);
            out.push('\n');
            blank = 0;
        }
    }
    out.trim().to_string()
}

// ---------------------------------------------------------------------------
// DuckDuckGo HTML result parsing
// ---------------------------------------------------------------------------

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

/// Parse the `html.duckduckgo.com/html/` result page into at most `count` items.
fn parse_ddg_results(html: &str, count: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut from = 0usize;

    while results.len() < count {
        let Some(rel) = html[from..].find("result__a") else {
            break;
        };
        let marker = from + rel;

        // The `result__a` class sits inside the opening <a> tag.
        let Some(tag_start) = html[..marker].rfind('<') else {
            break;
        };
        let Some(tag_end) = html[marker..].find('>').map(|r| marker + r) else {
            break;
        };
        let tag = &html[tag_start..tag_end];
        let href = extract_attr(tag, "href").unwrap_or_default();
        let url = clean_ddg_url(&href);

        let title_end = html[tag_end..]
            .find("</a>")
            .map(|r| tag_end + r)
            .unwrap_or(tag_end);
        let title = html_to_text(&html[tag_end + 1..title_end]);

        let mut snippet = String::new();
        if let Some(srel) = html[title_end..].find("result__snippet") {
            let sabs = title_end + srel;
            if let Some(gt) = html[sabs..].find('>') {
                let text_start = sabs + gt + 1;
                let text_end = html[text_start..]
                    .find("</a>")
                    .map(|r| text_start + r)
                    .unwrap_or(text_start);
                snippet = html_to_text(&html[text_start..text_end]);
            }
        }

        if !url.is_empty() || !title.is_empty() {
            results.push(SearchResult {
                title,
                url,
                snippet,
            });
        }
        from = title_end.max(marker + 1);
        if from >= html.len() {
            break;
        }
    }
    results
}

/// Extract `name="value"` (or `name='value'`) from a tag string.
fn extract_attr(tag: &str, name: &str) -> Option<String> {
    let mut search = 0usize;
    while let Some(rel) = tag[search..].find(name) {
        let at = search + rel;
        // Must be preceded by whitespace (attribute boundary).
        let boundary = at == 0 || tag[..at].ends_with(|c: char| c.is_whitespace());
        let rest = &tag[at + name.len()..];
        if let Some(r) = rest.strip_prefix('=') {
            if boundary {
                let r = r.trim_start();
                let quote = r.chars().next();
                let value = match quote {
                    Some(q @ ('"' | '\'')) => {
                        let after = &r[1..];
                        after.split(q).next().unwrap_or("").to_string()
                    }
                    _ => r
                        .split(|c: char| c.is_whitespace() || c == '>')
                        .next()
                        .unwrap_or("")
                        .to_string(),
                };
                return Some(value);
            }
        }
        search = at + name.len();
    }
    None
}

/// DuckDuckGo wraps result URLs in `/l/?uddg=<percent-encoded>`. Unwrap them.
fn clean_ddg_url(href: &str) -> String {
    if let Some(idx) = href.find("uddg=") {
        let rest = &href[idx + "uddg=".len()..];
        let enc = rest.split('&').next().unwrap_or(rest);
        return percent_decode(enc);
    }
    if let Some(stripped) = href.strip_prefix("//") {
        return format!("https://{stripped}");
    }
    href.to_string()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Percent-encode a query string for a URL (`space` -> `+`).
fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Truncate to at most `max` characters on a char boundary.
fn clip_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("\n… truncated");
    out
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing string field `{key}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_decodes_entities() {
        let html = "<html><head><title>T</title><style>p{color:red}</style></head>\
                    <body><h1>Hi &amp; welcome</h1><p>First&nbsp;line</p>\
                    <script>var x = 1 < 2;</script><p>A &#39;quote&#39; &#x41;</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Hi & welcome"), "got: {text}");
        assert!(text.contains("First line"), "got: {text}");
        assert!(text.contains("A 'quote' A"), "got: {text}");
        // Script/style bodies must not leak into the text.
        assert!(!text.contains("color:red"), "got: {text}");
        assert!(!text.contains("var x"), "got: {text}");
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(html_to_text("<p>a</p>\n\n\n<p>b</p>"), "a\n\nb");
    }

    #[test]
    fn parses_ddg_results() {
        let html = r#"
        <div class="result">
          <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&amp;rut=abc">Example &amp; Page</a>
          <a class="result__snippet">A short <b>snippet</b> here.</a>
        </div>
        <div class="result">
          <a rel="nofollow" class="result__a" href="https://rust-lang.org/">Rust</a>
          <a class="result__snippet">Systems language.</a>
        </div>
        "#;
        let results = parse_ddg_results(html, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://example.com/page");
        assert_eq!(results[0].title, "Example & Page");
        assert_eq!(results[0].snippet, "A short snippet here.");
        assert_eq!(results[1].url, "https://rust-lang.org/");
        assert_eq!(results[1].title, "Rust");
    }

    #[test]
    fn encodes_query() {
        assert_eq!(url_encode("a b&c"), "a+b%26c");
        assert_eq!(url_encode("rust lang"), "rust+lang");
    }

    #[test]
    fn extracts_attributes() {
        let tag = r#"<a rel="nofollow" href="https://x.y/z" class="result__a">"#;
        assert_eq!(extract_attr(tag, "href").as_deref(), Some("https://x.y/z"));
        assert_eq!(extract_attr(tag, "class").as_deref(), Some("result__a"));
        assert_eq!(extract_attr(tag, "missing"), None);
    }
}
