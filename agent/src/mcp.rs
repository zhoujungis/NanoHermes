//! Minimal MCP (Model Context Protocol) client.
//!
//! Two transports are supported:
//!
//! * **stdio** — the server is a child process; messages use LSP-style
//!   `Content-Length` framing over its stdin/stdout.
//! * **streamable HTTP** — the server is a remote endpoint; JSON-RPC requests
//!   are POSTed and the response is read either as `application/json` or from a
//!   `text/event-stream` body. An `Mcp-Session-Id` returned by the server is
//!   echoed on subsequent requests.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

type RespMap = Arc<Mutex<HashMap<u64, std::sync::mpsc::Sender<Value>>>>;

/// Lock without propagating poisoning: a panic in one request thread must not
/// make every later MCP call fail (or panic) for the rest of the session.
fn lock_or_recover<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One connected MCP server.
pub struct Server {
    name: String,
    transport: Transport,
    tool_names: HashSet<String>,
    openai_tools: Vec<Value>,
}

/// A collection of MCP servers, addressable by tool name.
pub struct Manager {
    servers: Vec<Server>,
}

impl Manager {
    /// Load servers from a JSON config:
    ///
    /// ```json
    /// { "servers": {
    ///     "<stdio-name>": { "command": "...", "args": [...] },
    ///     "<http-name>":  { "url": "https://host/mcp", "headers": { "Authorization": "..." } }
    /// } }
    /// ```
    pub fn from_config(path: &Path) -> Result<Manager, String> {
        let data = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let v: Value = serde_json::from_str(&data).map_err(|e| format!("bad MCP config: {e}"))?;
        let servers_obj = v
            .get("servers")
            .and_then(|s| s.as_object())
            .ok_or("MCP config is missing a `servers` object".to_string())?;

        let mut servers = Vec::new();
        for (name, spec) in servers_obj {
            servers.push(Server::connect(name, spec)?);
        }
        Ok(Manager { servers })
    }

    /// All tools from every server, in OpenAI function-calling format.
    pub fn all_tools(&self) -> Vec<Value> {
        self.servers
            .iter()
            .flat_map(|s| s.openai_tools.clone())
            .collect()
    }

    /// List servers with their tool names, for the `mcp list` subcommand.
    pub fn list_servers(&self) -> Vec<(String, Vec<String>)> {
        self.servers
            .iter()
            .map(|s| {
                let mut names: Vec<String> = s.tool_names.iter().cloned().collect();
                names.sort();
                (s.name.clone(), names)
            })
            .collect()
    }

    /// Call a tool by name. Returns `None` if no server exposes it.
    pub fn call(&self, name: &str, args: &Value) -> Option<Result<String, String>> {
        for s in &self.servers {
            if s.tool_names.contains(name) {
                return Some(s.call(name, args.clone()));
            }
        }
        None
    }

    /// Aggregated resource list from every server, each annotated with its server name.
    pub fn list_resources(&self) -> Result<Vec<Value>, String> {
        let mut all = Vec::new();
        for s in &self.servers {
            all.append(&mut s.list_resources()?);
        }
        Ok(all)
    }

    /// Read a resource by URI, trying each server until one succeeds.
    pub fn read_resource(&self, uri: &str) -> Result<String, String> {
        let mut last_err = String::from("no MCP server could read this resource");
        for s in &self.servers {
            match s.read_resource(uri) {
                Ok(text) => return Ok(text),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    /// Aggregated prompt list from every server.
    pub fn list_prompts(&self) -> Result<Vec<Value>, String> {
        let mut all = Vec::new();
        for s in &self.servers {
            all.append(&mut s.list_prompts()?);
        }
        Ok(all)
    }

    /// Get (and render) a prompt by name from any server that provides it.
    pub fn get_prompt(&self, name: &str, args: &Value) -> Result<String, String> {
        let mut last_err = format!("no MCP server provides prompt `{name}`");
        for s in &self.servers {
            match s.get_prompt(name, args) {
                Ok(text) => return Ok(text),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }
}

/// The transport a `Server` speaks to its backing MCP server.
enum Transport {
    Stdio(StdioTransport),
    Http(HttpTransport),
}

/// Child-process transport. `child` / `_reader` are wrapped in `Mutex` only so
/// that `Server` is `Sync`; the child is touched solely in `Drop`.
struct StdioTransport {
    child: Mutex<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    responses: RespMap,
    next_id: Arc<AtomicU64>,
    _reader: Mutex<std::thread::JoinHandle<()>>,
}

impl StdioTransport {
    fn send(&self, msg: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
        let frame = format!("Content-Length: {}\r\n\r\n", body.len());
        let mut stdin = self
            .stdin
            .lock()
            .map_err(|_| "MCP stdin lock poisoned".to_string())?;
        stdin
            .write_all(frame.as_bytes())
            .map_err(|e| e.to_string())?;
        stdin.write_all(&body).map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Streamable-HTTP transport. Requests are synchronous; no reader thread is
/// needed because each POST carries its own response.
struct HttpTransport {
    url: String,
    headers: Vec<(String, String)>,
    session: Mutex<Option<String>>,
    next_id: AtomicU64,
    agent: ureq::Agent,
}

impl Server {
    /// Connect to one server described by `spec` (either `command`+`args` for
    /// stdio, or `url`+optional `headers` for HTTP), then run the handshake.
    fn connect(name: &str, spec: &Value) -> Result<Server, String> {
        let transport = if let Some(url) = spec.get("url").and_then(|u| u.as_str()) {
            Transport::Http(connect_http(name, url, spec))
        } else {
            let cmd = spec
                .get("command")
                .and_then(|c| c.as_str())
                .ok_or_else(|| {
                    format!("MCP server `{name}` needs either `command` (stdio) or `url` (HTTP)")
                })?;
            let args: Vec<String> = spec
                .get("args")
                .and_then(|a| a.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            Transport::Stdio(connect_stdio(name, cmd, &args)?)
        };

        let mut server = Server {
            name: name.to_string(),
            transport,
            tool_names: HashSet::new(),
            openai_tools: Vec::new(),
        };
        server
            .handshake()
            .map_err(|e| format!("MCP `{name}` handshake failed: {e}"))?;
        Ok(server)
    }

    /// `initialize`, `notifications/initialized`, then `tools/list`.
    fn handshake(&mut self) -> Result<(), String> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "nanohermes", "version": env!("CARGO_PKG_VERSION") }
            }),
        )?;

        self.notify("notifications/initialized", json!({}))?;

        let list = self.request("tools/list", json!({}))?;
        let mcp_tools = list
            .get("tools")
            .and_then(|t| t.as_array())
            .ok_or("server returned no tools array")?;

        for t in mcp_tools {
            let tname = t.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if tname.is_empty() {
                continue;
            }
            self.tool_names.insert(tname.to_string());
            let desc = t.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let schema = t
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
            self.openai_tools.push(json!({
                "type": "function",
                "function": { "name": tname, "description": desc, "parameters": schema }
            }));
        }
        Ok(())
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        match &self.transport {
            Transport::Stdio(t) => {
                let id = t.next_id.fetch_add(1, Ordering::Relaxed);
                let (tx, rx) = std::sync::mpsc::channel();
                lock_or_recover(&t.responses).insert(id, tx);

                let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
                if let Err(e) = t.send(&msg) {
                    lock_or_recover(&t.responses).remove(&id);
                    return Err(e);
                }

                match rx.recv_timeout(Duration::from_secs(60)) {
                    Ok(resp) => unwrap_rpc(resp, method),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        lock_or_recover(&t.responses).remove(&id);
                        Err(format!("MCP request `{method}` timed out"))
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(format!(
                        "MCP server disconnected while waiting for `{method}`"
                    )),
                }
            }
            Transport::Http(t) => {
                let id = t.next_id.fetch_add(1, Ordering::Relaxed);
                let resp = t.post(&json!({
                    "jsonrpc": "2.0", "id": id, "method": method, "params": params
                }))?;
                let msg = match resp {
                    HttpBody::Json(v) => v,
                    HttpBody::Sse(resp, wanted) => read_sse_response(*resp, wanted)?,
                    HttpBody::Empty => {
                        return Err(format!("MCP request `{method}` returned an empty response"))
                    }
                };
                unwrap_rpc(msg, method)
            }
        }
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        match &self.transport {
            Transport::Stdio(t) => {
                t.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            }
            Transport::Http(t) => {
                // Notifications get no response body; the server replies 202 with none.
                t.post(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))?;
                Ok(())
            }
        }
    }

    fn call(&self, name: &str, arguments: Value) -> Result<String, String> {
        let res = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .map_err(|e| format!("MCP tool `{name}` call failed: {e}"))?;

        if res
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let msg = res
                .get("content")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
                .and_then(|p| p.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("(tool error)");
            return Err(format!("MCP tool `{name}` returned an error: {msg}"));
        }

        let mut out = String::new();
        if let Some(content) = res.get("content").and_then(|c| c.as_array()) {
            for part in content {
                if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                    out.push_str(t);
                    out.push('\n');
                }
            }
        }
        Ok(out.trim_end().to_string())
    }

    /// `resources/list` — returns the server's resources.
    fn list_resources(&self) -> Result<Vec<Value>, String> {
        let res = self.request("resources/list", json!({}))?;
        let arr = res
            .get("resources")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(arr)
    }

    /// `resources/read` — returns the text content of the first resource part.
    fn read_resource(&self, uri: &str) -> Result<String, String> {
        let res = self.request("resources/read", json!({ "uri": uri }))?;
        let contents = res
            .get("contents")
            .and_then(|c| c.as_array())
            .ok_or("resource has no contents")?;
        let mut out = String::new();
        for part in contents {
            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                out.push_str(t);
                out.push('\n');
            } else if let Some(b) = part.get("blob").and_then(|b| b.as_str()) {
                out.push_str(&format!("<{} bytes blob>\n", b.len()));
            }
        }
        Ok(out.trim_end().to_string())
    }

    /// `prompts/list` — returns the server's prompts.
    fn list_prompts(&self) -> Result<Vec<Value>, String> {
        let res = self.request("prompts/list", json!({}))?;
        let arr = res
            .get("prompts")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(arr)
    }

    /// `prompts/get` — renders a prompt and flattens its messages to text.
    fn get_prompt(&self, name: &str, args: &Value) -> Result<String, String> {
        let mut params = json!({ "name": name });
        if let Some(obj) = args.as_object() {
            if !obj.is_empty() {
                params["arguments"] = args.clone();
            }
        }
        let res = self.request("prompts/get", params)?;
        let messages = res
            .get("messages")
            .and_then(|m| m.as_array())
            .ok_or("prompt has no messages")?;
        let mut out = String::new();
        for m in messages {
            let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("?");
            let text = m
                .get("content")
                .and_then(|c| c.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            out.push_str(&format!("[{role}] {text}\n"));
        }
        Ok(out.trim_end().to_string())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Transport::Stdio(t) = &self.transport {
            if let Ok(mut child) = t.child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
            // The reader JoinHandle (inside `_reader`) is dropped here, detaching
            // the reader thread; it exits on its own once the killed child closes stdout.
        }
    }
}

/// Build the child-process transport and start its reader thread.
fn connect_stdio(name: &str, command: &str, args: &[String]) -> Result<StdioTransport, String> {
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("MCP server `{name}` (`{command}`) failed to start: {e}"))?;

    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");

    let stdin = Arc::new(Mutex::new(stdin));
    let responses: RespMap = Arc::new(Mutex::new(HashMap::new()));
    let next_id = Arc::new(AtomicU64::new(1));

    let reader = {
        let responses = Arc::clone(&responses);
        std::thread::spawn(move || reader_loop(stdout, responses))
    };

    Ok(StdioTransport {
        child: Mutex::new(child),
        stdin,
        responses,
        next_id,
        _reader: Mutex::new(reader),
    })
}

/// Build the HTTP transport. Uses the standard proxy environment variables so a
/// remote MCP server is reachable through a sandbox egress proxy.
fn connect_http(name: &str, url: &str, spec: &Value) -> HttpTransport {
    let headers: Vec<(String, String)> = spec
        .get("headers")
        .and_then(|h| h.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let _ = name;

    let mut builder = ureq::AgentBuilder::new().timeout(Duration::from_secs(120));
    if let Some(p) = crate::llm::proxy_for(url) {
        builder = builder.proxy(p);
    }
    HttpTransport {
        url: url.to_string(),
        headers,
        session: Mutex::new(None),
        next_id: AtomicU64::new(1),
        agent: builder.build(),
    }
}

/// Body of an HTTP response, classified by content type.
enum HttpBody {
    Json(Value),
    /// A live SSE response plus the id whose message we are waiting for.
    Sse(Box<ureq::Response>, u64),
    Empty,
}

impl HttpTransport {
    /// POST a JSON-RPC message and classify the response body.
    fn post(&self, msg: &Value) -> Result<HttpBody, String> {
        let body = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
        let mut req = self
            .agent
            .post(&self.url)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json, text/event-stream");
        for (k, v) in &self.headers {
            req = req.set(k, v);
        }
        if let Some(sid) = lock_or_recover(&self.session).clone() {
            req = req.set("Mcp-Session-Id", &sid);
        }

        let resp = match req.send_bytes(&body) {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                let detail = r.into_string().unwrap_or_default();
                return Err(format!(
                    "HTTP {code} from MCP server: {}",
                    truncate(&detail, 400)
                ));
            }
            Err(e) => return Err(format!("MCP request failed: {e}")),
        };

        // Remember the session id the server issued (or refreshed).
        if let Some(sid) = resp.header("Mcp-Session-Id") {
            if !sid.is_empty() {
                *lock_or_recover(&self.session) = Some(sid.to_string());
            }
        }

        let content_type = resp.header("Content-Type").unwrap_or("").to_string();
        let wanted = msg.get("id").and_then(|i| i.as_u64());

        if content_type.contains("text/event-stream") {
            // Notification streams never carry a matching id; treat them as empty.
            return Ok(match wanted {
                Some(id) => HttpBody::Sse(Box::new(resp), id),
                None => HttpBody::Empty,
            });
        }

        let text = resp.into_string().map_err(|e| e.to_string())?;
        if text.trim().is_empty() {
            return Ok(HttpBody::Empty);
        }
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("invalid MCP response: {e}"))?;
        Ok(HttpBody::Json(v))
    }
}

/// Turn a JSON-RPC response into its `result`, or an error for an `error` member.
fn unwrap_rpc(resp: Value, method: &str) -> Result<Value, String> {
    if let Some(err) = resp.get("error") {
        return Err(format!(
            "MCP error {} for `{method}`: {}",
            err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1),
            err.get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("(no message)")
        ));
    }
    Ok(resp.get("result").cloned().unwrap_or(Value::Null))
}

/// Read an SSE stream from an HTTP response until the JSON-RPC message with id
/// `want_id` arrives.
fn read_sse_response(resp: ureq::Response, want_id: u64) -> Result<Value, String> {
    read_sse_message(BufReader::new(resp.into_reader()), want_id)
}

/// Read an SSE stream until the JSON-RPC message with id `want_id` arrives.
fn read_sse_message(mut reader: impl BufRead, want_id: u64) -> Result<Value, String> {
    let mut data = String::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("MCP SSE stream ended without a response".to_string());
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            // End of an event: try to interpret the accumulated data.
            if !data.is_empty() {
                let payload = std::mem::take(&mut data);
                if let Ok(v) = serde_json::from_str::<Value>(&payload) {
                    if v.get("id").and_then(|i| i.as_u64()) == Some(want_id) {
                        return Ok(v);
                    }
                }
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
        // `event:`, `id:`, `retry:` and `:` comments are ignored.
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

/// Reads framed JSON-RPC messages from the server stdout and routes responses
/// (messages with an `id`) back to the waiting request thread.
fn reader_loop(stdout: std::process::ChildStdout, responses: RespMap) {
    let mut reader = BufReader::new(stdout);
    let failure = loop {
        match read_message(&mut reader) {
            Ok(msg) => {
                if let Some(id) = msg.get("id").and_then(|i| i.as_u64()) {
                    if let Some(tx) = lock_or_recover(&responses).remove(&id) {
                        let _ = tx.send(msg);
                    }
                }
                // Notifications (no id) are silently dropped for now.
            }
            Err(e) => break e,
        }
    };
    // Dropping the pending senders wakes every waiting request immediately
    // instead of making each one sit out the full 60 s timeout.
    lock_or_recover(&responses).clear();
    // A closed pipe is the normal shutdown path; a framing error is not.
    if failure != "MCP server closed stdout" {
        eprintln!("warning: MCP server connection lost: {failure}");
    }
}

/// Largest MCP frame we are willing to allocate. A buggy or hostile server
/// could otherwise make `vec![0; content_length]` allocate hundreds of GB.
const MAX_MCP_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

fn read_message(r: &mut impl BufRead) -> Result<Value, String> {
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        let n = r.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("MCP server closed stdout".to_string());
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break; // end of headers
        }
        // Header names are case-insensitive.
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    if content_length == 0 {
        return Err("MCP message missing Content-Length".to_string());
    }
    if content_length > MAX_MCP_MESSAGE_BYTES {
        return Err(format!(
            "MCP message too large ({content_length} > {MAX_MCP_MESSAGE_BYTES} bytes)"
        ));
    }
    let mut buf = vec![0u8; content_length];
    r.read_exact(&mut buf).map_err(|e| e.to_string())?;
    serde_json::from_slice(&buf).map_err(|e| format!("invalid MCP JSON: {e}"))
}

/// Path to a generated MCP config that launches the Python mock server used by
/// tests (`tests/mock_mcp_server.py`). It is written at runtime rather than
/// checked in so the interpreter (`python3` on Unix, `python` on Windows, where
/// the `python3` alias is a Store stub that never replies) and the absolute
/// script path are both correct on the host.
#[cfg(test)]
pub(crate) fn mock_server_config() -> std::path::PathBuf {
    use std::sync::OnceLock;
    static PATH: OnceLock<std::path::PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("mock_mcp_server.py");
        let python = if cfg!(windows) { "python" } else { "python3" };
        let cfg = serde_json::json!({
            "servers": {
                "mock": { "command": python, "args": [script.to_string_lossy()] }
            }
        });
        let path = std::env::temp_dir().join("nanohermes-mock-mcp-config.json");
        std::fs::write(&path, cfg.to_string()).expect("write mock MCP config");
        path
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_config() -> std::path::PathBuf {
        super::mock_server_config()
    }

    #[test]
    fn truncate_is_char_safe() {
        let s = "héllo wörld";
        let t = truncate(s, 2);
        assert!(t.starts_with("h") && !t.starts_with("hé"));
    }

    #[test]
    fn sse_message_returns_matching_id() {
        let stream = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"note\"}\n\n\
                      event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"ok\":true}}\n\n";
        let v = read_sse_message(std::io::Cursor::new(stream.as_bytes()), 2).unwrap();
        assert_eq!(v["result"]["ok"], true);
    }

    #[test]
    fn sse_message_handles_multiline_data() {
        let stream = "data: {\"jsonrpc\":\"2.0\",\"id\":7,\n\
                      data: \"result\":{\"n\":42}}\n\n";
        let v = read_sse_message(std::io::Cursor::new(stream.as_bytes()), 7).unwrap();
        assert_eq!(v["result"]["n"], 42);
    }

    #[test]
    fn resources_list_and_read() {
        let mgr = Manager::from_config(&mock_config()).expect("connect to mock server");

        let list = mgr.list_resources().expect("list resources");
        let uris: Vec<&str> = list
            .iter()
            .filter_map(|r| r.get("uri").and_then(|u| u.as_str()))
            .collect();
        assert!(uris.contains(&"file:///mock/notes.txt"), "got {uris:?}");
        assert!(uris.contains(&"mock://config"), "got {uris:?}");

        let notes = mgr
            .read_resource("file:///mock/notes.txt")
            .expect("read notes");
        assert!(
            notes.contains("hello from the notes resource"),
            "notes content: {notes}"
        );

        let cfg = mgr.read_resource("mock://config").expect("read config");
        assert!(cfg.contains("answer=42"), "config content: {cfg}");

        let err = mgr.read_resource("mock://missing").unwrap_err();
        assert!(err.contains("no such resource"), "unexpected err: {err}");
    }

    #[test]
    fn prompts_list_and_get() {
        let mgr = Manager::from_config(&mock_config()).expect("connect to mock server");

        let list = mgr.list_prompts().expect("list prompts");
        let names: Vec<&str> = list
            .iter()
            .filter_map(|p| p.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&"summarize"), "got {names:?}");
        assert!(names.contains(&"greet"), "got {names:?}");

        let greet = mgr
            .get_prompt("greet", &json!({ "name": "World" }))
            .expect("get greet prompt");
        assert!(greet.contains("Hello, World!"), "greet content: {greet}");

        let summarize = mgr
            .get_prompt("summarize", &json!({}))
            .expect("get summarize prompt");
        assert!(
            summarize.contains("summarize"),
            "summarize content: {summarize}"
        );

        let err = mgr.get_prompt("nope", &json!({})).unwrap_err();
        assert!(err.contains("unknown prompt"), "unexpected err: {err}");
    }

    #[test]
    fn tools_still_work() {
        let mgr = Manager::from_config(&mock_config()).expect("connect to mock server");
        // The mock also exposes an `echo` tool; make sure auto-dispatch still works.
        let out = mgr
            .call("echo", &json!({ "text": "ping" }))
            .expect("tool dispatched")
            .expect("call succeeded");
        assert_eq!(out, "ping");
    }

    /// The agent runs multiple tool calls in one turn concurrently. This test
    /// exercises the same concurrency path directly: several threads share one
    /// `Manager` and hit the mock server at the same time. If the stdin lock /
    /// response map / id counter aren't thread-safe, calls will deadlock or
    /// return mismatched results.
    #[test]
    fn concurrent_requests_are_safe() {
        let mgr = Manager::from_config(&mock_config()).expect("connect to mock server");
        let mgr = &mgr;

        std::thread::scope(|s| {
            let mut handles = Vec::new();

            // Several echo calls with distinct payloads running in parallel.
            for i in 0..8 {
                handles.push(s.spawn(move || {
                    let text = format!("msg-{i}");
                    let out = mgr
                        .call("echo", &json!({ "text": text }))
                        .expect("tool dispatched")
                        .expect("call succeeded");
                    assert_eq!(out, text);
                }));
            }

            // Interleaved resource reads.
            handles.push(s.spawn(move || {
                let notes = mgr
                    .read_resource("file:///mock/notes.txt")
                    .expect("read notes");
                assert!(notes.contains("hello from the notes resource"));
            }));
            handles.push(s.spawn(move || {
                let cfg = mgr.read_resource("mock://config").expect("read config");
                assert!(cfg.contains("answer=42"));
            }));

            // Interleaved prompt renders.
            handles.push(s.spawn(move || {
                let greet = mgr
                    .get_prompt("greet", &json!({ "name": "Ada" }))
                    .expect("get greet prompt");
                assert!(greet.contains("Hello, Ada!"));
            }));
            handles.push(s.spawn(move || {
                let list = mgr.list_resources().expect("list resources");
                assert_eq!(list.len(), 2);
            }));

            for h in handles {
                h.join().expect("concurrent MCP thread panicked");
            }
        });
    }

    /// Repeated bursts of concurrent mixed requests against one server. A single
    /// round might not expose a race in the stdin lock / response map / atomic id
    /// counter, but dozens of back-to-back bursts substantially raise the odds.
    #[test]
    fn concurrent_stress() {
        let mgr = Manager::from_config(&mock_config()).expect("connect to mock server");
        let mgr = &mgr;

        const ROUNDS: usize = 30;
        const PER_ROUND: usize = 12;

        for round in 0..ROUNDS {
            std::thread::scope(|s| {
                let mut handles = Vec::with_capacity(PER_ROUND);
                for i in 0..PER_ROUND {
                    let kind = (round * PER_ROUND + i) % 4;
                    handles.push(s.spawn(move || match kind {
                        0 => {
                            let text = format!("stress-{round}-{i}");
                            let out = mgr
                                .call("echo", &json!({ "text": text }))
                                .expect("dispatched")
                                .expect("call ok");
                            assert_eq!(out, text);
                        }
                        1 => {
                            let r = mgr.read_resource("file:///mock/notes.txt").expect("read");
                            assert!(r.contains("hello from the notes resource"));
                        }
                        2 => {
                            let who = format!("user-{i}");
                            let p = mgr
                                .get_prompt("greet", &json!({ "name": who }))
                                .expect("prompt");
                            assert!(p.contains(&format!("Hello, user-{i}!")));
                        }
                        _ => {
                            let list = mgr.list_resources().expect("list");
                            assert_eq!(list.len(), 2);
                        }
                    }));
                }
                for h in handles {
                    h.join().expect("stress thread panicked");
                }
            });
        }
    }

    // ---- Streamable HTTP transport ----

    /// Spin up a tiny in-process HTTP MCP server that speaks the streamable-HTTP
    /// subset NanoHermes uses (initialize / initialized / tools|resources|prompts).
    /// Returns the endpoint URL plus a handle that stops the server when set.
    fn spawn_mock_http() -> (
        String,
        Arc<std::sync::atomic::AtomicBool>,
        std::thread::JoinHandle<()>,
    ) {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock http");
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !stop_thread.load(Ordering::Relaxed) {
                let mut stream = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                handle_http_conn(&mut stream);
            }
        });
        (format!("http://127.0.0.1:{port}/mcp"), stop, handle)
    }

    fn handle_http_conn(stream: &mut std::net::TcpStream) {
        use std::io::{Read as _, Write as _};
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

        // Read headers.
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while !buf.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(0) => return,
                Ok(_) => buf.push(byte[0]),
                Err(_) => return,
            }
            if buf.len() > 64 * 1024 {
                return;
            }
        }
        let head = String::from_utf8_lossy(&buf).to_string();
        let content_len: usize = head
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length:"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);

        let mut body = vec![0u8; content_len];
        if content_len > 0 && stream.read_exact(&mut body).is_err() {
            return;
        }
        let msg: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => return,
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(json!({}));

        let respond = |stream: &mut std::net::TcpStream, status: &str, body: &str, sid: bool| {
            let extra = if sid {
                "Mcp-Session-Id: test-session\r\n"
            } else {
                ""
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        };

        match method {
            "initialize" => {
                let body = json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": { "tools": {}, "resources": {}, "prompts": {} },
                        "serverInfo": { "name": "mock-http", "version": "1.0.0" }
                    }
                })
                .to_string();
                respond(stream, "200 OK", &body, true);
            }
            "notifications/initialized" => respond(stream, "202 Accepted", "", false),
            "tools/list" => {
                let body = json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "tools": [{
                        "name": "echo",
                        "description": "Echo the input back.",
                        "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] }
                    }] }
                })
                .to_string();
                respond(stream, "200 OK", &body, false);
            }
            "tools/call" => {
                let text = params
                    .get("arguments")
                    .and_then(|a| a.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let body = json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "content": [{ "type": "text", "text": text }] }
                })
                .to_string();
                respond(stream, "200 OK", &body, false);
            }
            "resources/list" => {
                let body = json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "resources": [{ "uri": "mock://http", "name": "http" }] }
                })
                .to_string();
                respond(stream, "200 OK", &body, false);
            }
            _ => {
                let body = json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": -1, "message": "unknown method" }
                })
                .to_string();
                respond(stream, "200 OK", &body, false);
            }
        }
    }

    #[test]
    fn http_transport_handshake_and_call() {
        let (url, stop, handle) = spawn_mock_http();

        let cfg = json!({ "servers": { "remote": { "url": url } } }).to_string();
        let dir = std::env::temp_dir().join(format!("nanohermes-mcp-http-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp_http.json");
        std::fs::write(&path, cfg).unwrap();

        let mgr = Manager::from_config(&path).expect("connect over HTTP");

        let servers = mgr.list_servers();
        assert_eq!(servers.len(), 1);
        assert!(
            servers[0].1.contains(&"echo".to_string()),
            "tools: {:?}",
            servers[0].1
        );

        let out = mgr
            .call("echo", &json!({ "text": "over-http" }))
            .expect("tool dispatched")
            .expect("call succeeded");
        assert_eq!(out, "over-http");

        let resources = mgr.list_resources().expect("list resources");
        assert_eq!(resources[0]["uri"], "mock://http");

        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
