//! NanoHermes Agent — a lightweight, low-memory terminal coding agent.
//!
//! Design goals (and where each is met):
//!   * small binary / low RSS  -> no async runtime, blocking I/O, few deps
//!   * bounded memory          -> capped tool output, context compaction, capped command capture
//!   * useful by default       -> OpenAI-compatible endpoint, streaming, real file/shell tools

use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent::{
    agent::{load_session_file, save_session_file},
    config, mcp, sandbox,
    ui::UiEvent,
    user_content, Agent, Config,
};
use base64::Engine;
use serde_json::{json, Value};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Subcommand: `agent mcp list --mcp-config <FILE>`
    if args.first().map(|s| s.as_str()) == Some("mcp") {
        match handle_mcp_subcommand(&args[1..]) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }

    let (mut cfg, cli_prompt, images) = match parse_args(args) {
        Ok(Some(v)) => v,
        Ok(None) => return, // --help / --version already printed
        Err(msg) => {
            eprintln!("error: {msg}\n\nrun `agent --help` for usage");
            std::process::exit(2);
        }
    };

    if cfg.api_key.trim().is_empty() {
        eprintln!(
            "warning: no API key set (NANOHERMES_API_KEY / OPENAI_API_KEY); requests may fail"
        );
    }

    // Load sub-agent definitions if a config was provided.
    if let Some(path) = cfg.subagents_config.as_ref() {
        match config::SubAgentDef::load_all(path) {
            Ok(defs) => {
                if !defs.is_empty() {
                    eprintln!("sub-agents: loaded {} from {}", defs.len(), path.display());
                }
                cfg.subagents = defs;
            }
            Err(e) => eprintln!("warning: sub-agents config {}: {e}", path.display()),
        }
    }

    let mcp = match cfg.mcp_config.clone() {
        Some(path) => match mcp::Manager::from_config(&path) {
            Ok(m) => {
                let n = m.all_tools().len();
                if n > 0 {
                    eprintln!("MCP: loaded {n} tool(s) from {}", path.display());
                }
                Some(m)
            }
            Err(e) => {
                eprintln!("warning: MCP config {}: {e}", path.display());
                None
            }
        },
        None => None,
    };

    let input_format = cfg.input_format;
    // Warn early if a sandbox is requested but the kernel will refuse to apply it.
    if cfg.sandbox.is_active() && !sandbox::is_supported() {
        eprintln!(
            "warning: sandbox {:?} requested but this kernel does not expose Landlock; \
             shell commands will be refused",
            cfg.sandbox
        );
    }
    let mut agent = Agent::with_mcp(cfg, mcp);

    // Daemon mode: serve one-shot requests over HTTP instead of REPL/oneshot.
    if agent.cfg().daemon {
        run_daemon(agent);
        return;
    }

    let task = if cli_prompt.is_empty() {
        task_from_stdin_or_repl(input_format)
    } else {
        Task::OneShot(cli_prompt.join(" "))
    };

    match task {
        Task::OneShot(text) => {
            let content = build_content(&text, &images).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                std::process::exit(1);
            });
            match agent.run(content) {
                Ok(response) => {
                    match agent.cfg().output_format {
                        config::OutputFormat::Json => {
                            let u = agent.usage();
                            let out = json!({
                                "response": response,
                                "usage": {
                                    "requests": u.requests,
                                    "prompt_tokens": u.prompt_tokens,
                                    "completion_tokens": u.completion_tokens,
                                    "total_tokens": u.total_tokens(),
                                }
                            });
                            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
                        }
                        config::OutputFormat::StreamJson => {
                            // Deltas were already streamed as JSON lines; emit a final summary line.
                            let u = agent.usage();
                            let out = json!({
                                "done": true,
                                "response": response,
                                "usage": {
                                    "requests": u.requests,
                                    "prompt_tokens": u.prompt_tokens,
                                    "completion_tokens": u.completion_tokens,
                                    "total_tokens": u.total_tokens(),
                                }
                            });
                            println!("{}", out);
                        }
                        config::OutputFormat::Text => {}
                    }
                }
                Err(e) => {
                    eprintln!("\nerror: {e}");
                    std::process::exit(1);
                }
            }
        }
        Task::Repl => repl(&mut agent),
        Task::None => {
            eprintln!("no task provided\n\nrun `agent --help` for usage");
            std::process::exit(2);
        }
    }
}

/// Build the user message content: a plain string, or a multimodal content
/// array when images are supplied (vision-capable models only).
fn build_content(text: &str, images: &[PathBuf]) -> Result<Value, String> {
    if images.is_empty() {
        return Ok(json!(text));
    }
    let mut parts = vec![json!({"type": "text", "text": text})];
    for img in images {
        parts.push(image_part(img)?);
    }
    Ok(Value::Array(parts))
}

fn image_part(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mime = mime_for(path);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(json!({
        "type": "image_url",
        "image_url": { "url": format!("data:{mime};base64,{b64}") }
    }))
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "image/png",
    }
}

enum Task {
    OneShot(String),
    Repl,
    None,
}

/// No CLI prompt given: use an interactive REPL on a TTY, otherwise read stdin.
fn task_from_stdin_or_repl(input_format: config::InputFormat) -> Task {
    if io::stdin().is_terminal() {
        return Task::Repl;
    }
    let mut buf = String::new();
    if io::stdin().read_to_string(&mut buf).is_ok() {
        let text = match input_format {
            config::InputFormat::Text => buf.trim().to_string(),
            config::InputFormat::StreamJson => {
                // Each line is {"text": "..."}; concatenate all text fields.
                let mut combined = String::new();
                for line in buf.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(line) {
                        if let Some(t) = v.get("text").and_then(|x| x.as_str()) {
                            if !combined.is_empty() {
                                combined.push('\n');
                            }
                            combined.push_str(t);
                        }
                    }
                }
                combined
            }
        };
        if !text.is_empty() {
            return Task::OneShot(text);
        }
    }
    Task::None
}

fn repl(agent: &mut Agent) {
    let style = agent.style();
    println!("{}", style.bold("NanoHermes Agent"));
    println!(
        "{}",
        style.dim("type a task · /help for commands · !<cmd> runs shell · end a line with \\ for multi-line")
    );
    let history_path = history_file_path();
    // Images queued via `/image` are attached to the next task, then cleared.
    let mut pending_images: Vec<PathBuf> = Vec::new();
    let mut line = String::new();
    loop {
        print!("\n{} ", style.green("›"));
        let _ = io::stdout().flush();
        line.clear();
        match io::stdin().read_line(&mut line) {
            Ok(0) => break, // EOF (Ctrl-D)
            Ok(_) => {}
            Err(e) => {
                eprintln!("input error: {e}");
                break;
            }
        }
        let mut input = line.trim().to_string();

        // Multi-line input: if the line ends with a backslash, keep reading.
        while input.ends_with('\\') {
            input.truncate(input.len() - 1);
            print!("{} ", style.green("·"));
            let _ = io::stdout().flush();
            let mut cont = String::new();
            if io::stdin().read_line(&mut cont).unwrap_or(0) == 0 {
                break;
            }
            input.push_str(cont.trim());
        }

        if input.is_empty() {
            continue;
        }

        // Shell-escape: lines starting with `!` run a shell command directly.
        if let Some(cmd) = input.strip_prefix('!') {
            let cmd = cmd.trim();
            if !cmd.is_empty() {
                run_shell_direct(cmd);
            }
            continue;
        }

        // `/model <name>` switches the active model.
        if let Some(rest) = input.strip_prefix("/model") {
            let name = rest.trim();
            if name.is_empty() {
                println!(
                    "{}",
                    style.dim(&format!("current model: {}", agent.cfg().model))
                );
            } else {
                agent.set_model(name);
                println!("{}", style.dim(&format!("model switched to {name}")));
            }
            continue;
        }

        // `/mode <mode>` switches the permission mode.
        if let Some(rest) = input.strip_prefix("/mode") {
            let name = rest.trim();
            if name.is_empty() {
                let m = agent.cfg().permission_mode.as_str();
                println!("{}", style.dim(&format!("current mode: {m}")));
            } else {
                match config::PermissionMode::parse(name) {
                    Ok(m) => {
                        agent.set_permission_mode(m);
                        println!("{}", style.dim(&format!("permission mode: {name}")));
                    }
                    Err(e) => println!("{}", style.dim(&format!("error: {e}"))),
                }
            }
            continue;
        }

        // `/format <fmt>` switches the output format.
        if let Some(rest) = input.strip_prefix("/format") {
            let name = rest.trim();
            if name.is_empty() {
                let f = match agent.cfg().output_format {
                    config::OutputFormat::Text => "text",
                    config::OutputFormat::Json => "json",
                    config::OutputFormat::StreamJson => "stream-json",
                };
                println!("{}", style.dim(&format!("current format: {f}")));
            } else {
                match config::OutputFormat::parse(name) {
                    Ok(f) => {
                        agent.set_output_format(f);
                        println!("{}", style.dim(&format!("output format: {name}")));
                    }
                    Err(e) => println!("{}", style.dim(&format!("error: {e}"))),
                }
            }
            continue;
        }

        // `/tools` lists all currently available tools.
        if input == "/tools" {
            let tools = agent.available_tools();
            if tools.is_empty() {
                println!("{}", style.dim("no tools available"));
            } else {
                for (name, desc) in &tools {
                    println!("  {}  {}", style.bold(name), style.dim(desc));
                }
            }
            continue;
        }

        // `/status` shows session statistics.
        if input == "/status" {
            let cfg = agent.cfg();
            let pmode = cfg.permission_mode.as_str();
            let ofmt = match cfg.output_format {
                config::OutputFormat::Text => "text",
                config::OutputFormat::Json => "json",
                config::OutputFormat::StreamJson => "stream-json",
            };
            println!("  messages: {}", agent.message_count());
            println!("  model:    {}", cfg.model);
            println!("  mode:     {pmode}");
            println!("  format:   {ofmt}");
            println!("  tools:    {}", agent.available_tools().len());
            println!("  workspace: {}", cfg.workspace.display());
            continue;
        }

        // `/cost` shows cumulative token usage for the session.
        if input == "/cost" {
            let u = agent.usage();
            println!("  requests:          {}", u.requests);
            println!("  prompt tokens:     {}", u.prompt_tokens);
            println!("  completion tokens: {}", u.completion_tokens);
            println!("  total tokens:      {}", u.total_tokens());
            continue;
        }

        // `/compact` forces context compaction (summarize older messages away).
        if input == "/compact" {
            let dropped = agent.compact_now();
            if dropped == 0 {
                println!("{}", style.dim("nothing to compact"));
            }
            continue;
        }

        // `/images` lists queued images; `/drop-images` clears them.
        if input == "/images" {
            if pending_images.is_empty() {
                println!("{}", style.dim("no images queued"));
            } else {
                for p in &pending_images {
                    println!("  {}", p.display());
                }
            }
            continue;
        }
        if input == "/drop-images" {
            pending_images.clear();
            println!("{}", style.dim("cleared queued images"));
            continue;
        }

        // `/image <path>` queues an image for the next task (vision models only).
        if let Some(rest) = input.strip_prefix("/image") {
            let path = rest.trim();
            if path.is_empty() {
                println!("{}", style.dim("usage: /image <path>"));
            } else {
                let p = PathBuf::from(path);
                if !p.is_file() {
                    println!("{}", style.dim(&format!("error: not a file: {path}")));
                } else {
                    pending_images.push(p);
                    println!(
                        "{}",
                        style.dim(&format!("queued image ({})", pending_images.len()))
                    );
                }
            }
            continue;
        }

        // `/save <file>` saves the conversation to a specific file.
        if let Some(rest) = input.strip_prefix("/save") {
            let path = rest.trim();
            if path.is_empty() {
                println!("{}", style.dim("usage: /save <file>"));
            } else {
                match agent.save_session_to(std::path::Path::new(path)) {
                    Ok(()) => println!("{}", style.dim(&format!("session saved to {path}"))),
                    Err(e) => println!("{}", style.dim(&format!("error: {e}"))),
                }
            }
            continue;
        }

        // `/load <file>` loads a conversation from a file.
        if let Some(rest) = input.strip_prefix("/load") {
            let path = rest.trim();
            if path.is_empty() {
                println!("{}", style.dim("usage: /load <file>"));
            } else {
                match agent.load_session_from(std::path::Path::new(path)) {
                    Ok(()) => println!("{}", style.dim(&format!("session loaded from {path}"))),
                    Err(e) => println!("{}", style.dim(&format!("error: {e}"))),
                }
            }
            continue;
        }

        // `/undo [n]` rewinds `n` full user turns (default 1).
        if let Some(rest) = input.strip_prefix("/undo") {
            let rest = rest.trim();
            let turns = if rest.is_empty() {
                1
            } else {
                match rest.parse::<usize>() {
                    Ok(0) | Err(_) => {
                        println!("{}", style.dim("usage: /undo [n]  (n >= 1)"));
                        continue;
                    }
                    Ok(n) => n,
                }
            };
            let removed = agent.rewind(turns);
            if removed == 0 {
                println!("{}", style.dim("nothing to undo"));
            } else {
                println!("{}", style.dim(&format!("rewound {removed} turn(s)")));
            }
            continue;
        }

        match input.as_str() {
            "/exit" | "/quit" | ":q" => break,
            "/clear" => {
                agent.reset();
                println!("{}", style.dim("conversation cleared"));
                continue;
            }
            "/help" => {
                println!("{}", style.dim("REPL commands:"));
                println!("  /help            show this help");
                println!("  /clear           reset the conversation");
                println!("  /undo [n]        rewind the last n turns (default 1)");
                println!("  /model <name>    switch model");
                println!("  /mode <mode>     switch permission mode (default|plan|goal|debug)");
                println!("  /format <fmt>    switch output format (text|json|stream-json)");
                println!("  /tools           list available tools");
                println!("  /status          show session stats");
                println!("  /cost            show token usage");
                println!("  /compact         summarize older messages to free context");
                println!("  /image <path>    attach an image to the next task");
                println!("  /images          list queued images");
                println!("  /drop-images     clear queued images");
                println!("  /save <file>     save conversation to file");
                println!("  /load <file>     load conversation from file");
                println!("  /exit            quit");
                println!("  !<cmd>           run a shell command directly");
                continue;
            }
            _ => {}
        }

        // Persist the line to the history file (best-effort).
        let _ = append_history(&history_path, &input);

        let content = match build_content(&input, &pending_images) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("\nerror: {e}");
                continue;
            }
        };
        pending_images.clear();
        if let Err(e) = agent.run(content) {
            eprintln!("\nerror: {e}");
        }
    }
    println!("\n{}", style.dim("bye"));
}

/// In-memory + on-disk store for the daemon's isolated conversations. Each
/// session id maps to its own message vector; requests with different ids never
/// see each other's context. When a `--session` path is configured, sessions are
/// persisted under its parent directory (the default id uses the original path).
struct SessionStore {
    sessions: std::collections::HashMap<String, Vec<Value>>,
    dir: Option<std::path::PathBuf>,
    default_path: Option<std::path::PathBuf>,
}

impl SessionStore {
    fn new(session_path: Option<std::path::PathBuf>) -> Self {
        let dir = session_path
            .as_ref()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
        SessionStore {
            sessions: std::collections::HashMap::new(),
            dir,
            default_path: session_path,
        }
    }

    /// Resolve the on-disk path for a session id.
    fn path_for(&self, id: &str) -> Option<std::path::PathBuf> {
        if id == "default" {
            self.default_path.clone()
        } else {
            self.dir.as_ref().map(|d| d.join(format!("{id}.json")))
        }
    }

    /// Load the messages for a session (cache → disk → fresh).
    fn get(&mut self, id: &str, fresh: Vec<Value>) -> Vec<Value> {
        if let Some(msgs) = self.sessions.get(id) {
            return msgs.clone();
        }
        if let Some(path) = self.path_for(id) {
            if path.exists() {
                match load_session_file(&path) {
                    Ok(msgs) => {
                        self.sessions.insert(id.to_string(), msgs.clone());
                        return msgs;
                    }
                    Err(e) => eprintln!("warning: could not load session {}: {e}", path.display()),
                }
            }
        }
        fresh
    }

    /// Persist a session's messages to memory and disk.
    fn put(&mut self, id: &str, msgs: Vec<Value>) {
        if let Some(path) = self.path_for(id) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = save_session_file(&path, &msgs) {
                eprintln!("warning: could not save session {}: {e}", path.display());
            }
        }
        self.sessions.insert(id.to_string(), msgs);
    }
}

/// Run NanoHermes as a lightweight HTTP daemon. Accepts POST requests with a JSON
/// body `{"prompt": "..."}` and returns `{"response", "session", "usage"}` —
/// `usage` is what this request added, so a client can total up a task's cost.
/// Conversations are isolated per `X-Session-Id` header (default session when
/// omitted).
fn run_daemon(mut agent: Agent) {
    let port = agent.cfg().daemon_port;
    let bind = agent.cfg().daemon_bind.clone();
    let has_token = !agent.cfg().daemon_token.is_empty();
    let mut store = SessionStore::new(agent.cfg().session.clone());
    let listener = match std::net::TcpListener::bind(format!("{bind}:{port}")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: failed to bind daemon to {bind}:{port}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("agent daemon listening on http://{bind}:{port}");
    if has_token {
        eprintln!("auth: Bearer token required (NANOHERMES_DAEMON_TOKEN)");
    }
    eprintln!("POST {{\"prompt\": \"...\"}} to run a task, GET /health for liveness");
    eprintln!("isolation: set X-Session-Id header to pick a conversation (default when omitted)");

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("accept error: {e}");
                continue;
            }
        };
        let peer = stream.peer_addr().ok();
        match handle_daemon_request(&mut agent, &mut store, &mut stream) {
            Ok(()) => {}
            Err(e) => {
                if let Some(addr) = peer {
                    eprintln!("daemon request error from {addr}: {e}");
                } else {
                    eprintln!("daemon request error: {e}");
                }
            }
        }
    }
}

/// Request bodies carry prompts and optionally base64 image data URLs (up to
/// four images). Cap them so an unauthenticated or buggy client cannot make the
/// single-threaded daemon allocate without bound.
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

/// Upper bound on distinct in-memory conversations, so a client cannot grow the
/// map (and the session directory) without limit.
const MAX_DAEMON_SESSIONS: usize = 256;

/// Session ids become file names, so only a conservative alphabet is accepted.
/// This blocks `../`, absolute paths, NUL bytes and Windows device names.
fn valid_session_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 || id.starts_with('.') {
        return false;
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    {
        return false;
    }
    let stem = id.split('.').next().unwrap_or("").to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    !device
}

/// Write a small JSON response (used for error paths).
fn write_json_error(stream: &mut std::net::TcpStream, status: &str, message: &str) {
    use std::io::Write;
    let body = json!({ "error": message }).to_string();
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

/// Parse one HTTP request from the stream, run the agent, and write the response.
fn handle_daemon_request(
    agent: &mut Agent,
    store: &mut SessionStore,
    stream: &mut std::net::TcpStream,
) -> Result<(), String> {
    use std::io::{Read, Write};

    let mut buf = [0u8; 8192];
    let mut data = Vec::new();
    loop {
        let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        // Simple end-of-headers detection: look for \r\n\r\n.
        if data.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if data.len() > 1024 * 1024 {
            return Err("request too large".to_string());
        }
    }

    let text = String::from_utf8_lossy(&data);
    let lines: Vec<&str> = text.split("\r\n").collect();
    let first_line = lines.first().unwrap_or(&"");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");

    // GET /health -> lightweight liveness check (no auth needed so monitors can use it).
    if method == "GET" && path == "/health" {
        let resp = json!({"status": "ok", "sessions": store.sessions.len()}).to_string();
        let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", resp.len());
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(resp.as_bytes());
        return Ok(());
    }

    // Bearer token check (defense in depth; Cloudflare Access is the outer layer).
    if !agent.cfg().daemon_token.is_empty() {
        let auth = header_value(&lines, "authorization");
        let expected = format!("Bearer {}", agent.cfg().daemon_token);
        if auth.as_deref() != Some(expected.as_str()) {
            let _ = stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nWWW-Authenticate: Bearer\r\n\r\n");
            return Ok(());
        }
    }

    if method != "POST" {
        let _ = stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n");
        return Ok(());
    }

    // Session isolation: pick a conversation by X-Session-Id header (default when omitted).
    let session_id = header_value(&lines, "x-session-id")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "default".to_string());
    let session_id = session_id.trim().to_string();
    if !valid_session_id(&session_id) {
        write_json_error(
            stream,
            "400 Bad Request",
            "invalid X-Session-Id: use 1-64 characters from [A-Za-z0-9._-], not starting with '.'",
        );
        return Ok(());
    }
    if !store.sessions.contains_key(&session_id) && store.sessions.len() >= MAX_DAEMON_SESSIONS {
        write_json_error(
            stream,
            "429 Too Many Requests",
            "too many sessions; reuse an existing X-Session-Id",
        );
        return Ok(());
    }

    // Header names are case-insensitive, so go through the same helper as
    // every other header. A strict `Content-Length:` prefix check dropped the
    // body of clients that send `content-length:`.
    let content_len = header_value(&lines, "content-length")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if content_len > MAX_BODY_BYTES {
        write_json_error(stream, "413 Payload Too Large", "request body too large");
        return Ok(());
    }

    // Find the byte offset of the body (after \r\n\r\n).
    let header_end = data
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(data.len());
    let mut body = data[header_end..].to_vec();
    while body.len() < content_len {
        let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
        // Also bound bodies whose declared length was absent or lied.
        if body.len() > MAX_BODY_BYTES {
            write_json_error(stream, "413 Payload Too Large", "request body too large");
            return Ok(());
        }
    }

    let (prompt, images, want_stream) = if body.is_empty() {
        (String::new(), Vec::new(), false)
    } else {
        let body_str = String::from_utf8_lossy(&body).to_string();
        match serde_json::from_str::<Value>(&body_str) {
            Ok(v) => (
                v.get("prompt")
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .to_string(),
                // Attached images travel as `data:image/...;base64,...` URLs so a
                // client never has to upload files the daemon cannot see; the
                // legacy CLI attach path keeps using real paths.
                v.get("images")
                    .and_then(|i| i.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str())
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                // A client that asks for `"stream": true` gets events as they
                // happen (the plain response is one blocking body).
                v.get("stream").and_then(|s| s.as_bool()).unwrap_or(false),
            ),
            Err(_) => (String::new(), Vec::new(), false),
        }
    };

    if prompt.trim().is_empty() {
        let resp = json!({"error": "missing `prompt` in request body"}).to_string();
        let header = format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", resp.len());
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(resp.as_bytes());
        return Ok(());
    }

    if want_stream {
        return stream_turn(agent, store, stream, &session_id, &prompt, &images);
    }

    // Swap in this session's conversation, run, then swap it back out and persist.
    let fresh = agent.fresh_messages();
    let session_msgs = store.get(&session_id, fresh);
    let _prev = agent.replace_messages(session_msgs);

    let before = agent.usage();
    let result = agent.run(user_content(&prompt, &images));
    let after = agent.usage();

    let updated = agent.replace_messages(Vec::new());
    store.put(&session_id, updated);

    // A failed turn is reported as an error, never as an empty-but-ok reply: the
    // client must be able to tell "no answer" from "it broke".
    let resp = match result {
        Ok(response) => json!({
            "response": response,
            "session": session_id,
            "usage": {
                "requests": after.requests.saturating_sub(before.requests),
                "prompt_tokens": after.prompt_tokens.saturating_sub(before.prompt_tokens),
                "completion_tokens": after.completion_tokens.saturating_sub(before.completion_tokens),
                "total_tokens": after.total_tokens().saturating_sub(before.total_tokens()),
            }
        }),
        Err(e) => json!({ "error": e, "session": session_id }),
    }
    .to_string();
    let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", resp.len());
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
    Ok(())
}

/// One event as a newline-delimited JSON object (NDJSON). Deliberately not SSE
/// framing: a line-oriented payload survives any intermediary that buffers, and
/// the connection close ends the body — a client that never sees a `done` line
/// simply knows the turn did not finish.
const NDJSON_CONTENT_TYPE: &str = "application/x-ndjson";

/// Write one JSON event line and flush, so the client sees it immediately rather
/// than when the socket buffer fills.
fn write_event(out: &Arc<Mutex<std::net::TcpStream>>, value: &Value) {
    let line = format!("{value}\n");
    if let Ok(mut w) = out.lock() {
        let _ = w.write_all(line.as_bytes());
        let _ = w.flush();
    }
}

/// Run one turn and push its progress to the client as it happens: streamed text,
/// tool calls, notes and token usage, then a final `done` line carrying the full
/// reply. The agent's UI sink is installed for the duration, which is what makes
/// tool calls and notes reach the client instead of the daemon's own log.
fn stream_turn(
    agent: &mut Agent,
    store: &mut SessionStore,
    stream: &mut std::net::TcpStream,
    session_id: &str,
    prompt: &str,
    images: &[String],
) -> Result<(), String> {
    use std::io::Write;

    // A second handle on the same socket: the sink writes from the worker threads
    // the agent may use for tool calls, while this function keeps the original for
    // the final flush. Writes are serialised through the mutex.
    let mut out = stream.try_clone().map_err(|e| e.to_string())?;
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {NDJSON_CONTENT_TYPE}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
    );
    out.write_all(header.as_bytes())
        .map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    let out = Arc::new(Mutex::new(out));

    let sink_out = Arc::clone(&out);
    agent.set_ui_sink(Arc::new(move |ev: UiEvent| {
        let payload = match ev {
            UiEvent::Delta(text) => json!({ "type": "delta", "text": text }),
            UiEvent::ToolCall {
                name,
                summary,
                output,
            } => json!({ "type": "tool", "name": name, "summary": summary, "output": output }),
            UiEvent::Note(text) => json!({ "type": "note", "text": text }),
            UiEvent::Usage {
                requests,
                prompt_tokens,
                completion_tokens,
            } => json!({
                "type": "usage",
                "requests": requests,
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
            }),
        };
        write_event(&sink_out, &payload);
    }));
    // The client asked for deltas, so stream even if the daemon was started with
    // a format that turns them off.
    agent.set_stream(true);

    let fresh = agent.fresh_messages();
    let session_msgs = store.get(session_id, fresh);
    let _prev = agent.replace_messages(session_msgs);
    // One agent serves every session in this process, so its running totals span
    // all conversations. Report only what this request added.
    let result = agent.run(user_content(prompt, images));
    let updated = agent.replace_messages(Vec::new());
    store.put(session_id, updated);

    // Detach the sink: this socket is finished, and a later turn must not write
    // into it.
    agent.clear_ui_sink();

    // A failed turn ends with an explicit `error` line, so the client shows the
    // reason instead of an empty answer.
    match result {
        Ok(response) => write_event(
            &out,
            &json!({ "type": "done", "response": response, "session": session_id }),
        ),
        Err(e) => write_event(&out, &json!({ "type": "error", "error": e })),
    }
    Ok(())
}

/// Extract a header value (case-insensitive) from the parsed request lines.
fn header_value<'a>(lines: &'a [&'a str], name: &str) -> Option<String> {
    let prefix = format!("{name}:");
    lines
        .iter()
        .find(|l| {
            l.to_ascii_lowercase()
                .starts_with(&prefix.to_ascii_lowercase())
        })
        .map(|l| l[prefix.len()..].trim().to_string())
}

/// Run a shell command directly from the REPL (bypasses the agent).
fn run_shell_direct(cmd: &str) {
    match agent::tools::shell::shell_command(cmd).status() {
        Ok(status) => {
            if !status.success() {
                eprintln!("exit code: {}", status.code().unwrap_or(-1));
            }
        }
        Err(e) => eprintln!("failed to run `{cmd}`: {e}"),
    }
}

/// Path to the persistent REPL history file (~/.nanohermes/history).
fn history_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = std::path::PathBuf::from(home).join(".nanohermes");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("history")
}

/// Append a line to the history file (best-effort, ignored on failure).
fn append_history(path: &Path, line: &str) -> io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

/// Handle the `agent mcp <subcommand>` family of commands.
/// Currently supports `mcp list --mcp-config <FILE>`.
fn handle_mcp_subcommand(args: &[String]) -> Result<(), String> {
    let mut sub = None;
    let mut mcp_config: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "list" => sub = Some("list"),
            "--mcp-config" => {
                i += 1;
                mcp_config = Some(PathBuf::from(
                    args.get(i).ok_or("--mcp-config requires a value")?,
                ));
            }
            other => return Err(format!("unknown mcp argument `{other}`")),
        }
        i += 1;
    }
    match sub {
        Some("list") => {
            let path = mcp_config.ok_or("`mcp list` requires --mcp-config <FILE>")?;
            let mgr = mcp::Manager::from_config(&path)?;
            let servers = mgr.list_servers();
            if servers.is_empty() {
                println!("(no MCP servers configured)");
                return Ok(());
            }
            for (name, tools) in &servers {
                println!("server: {name}");
                if tools.is_empty() {
                    println!("  (no tools)");
                } else {
                    for t in tools {
                        println!("  - {t}");
                    }
                }
            }
            Ok(())
        }
        Some(other) => Err(format!("unknown mcp subcommand `{other}`")),
        None => Err("usage: agent mcp list --mcp-config <FILE>".to_string()),
    }
}

/// Scan args for `--config <FILE>` and return the path, if present.
fn extract_config_path(args: &[String]) -> Option<PathBuf> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            return Some(PathBuf::from(&args[i + 1]));
        }
        i += 1;
    }
    None
}

/// Apply an agent mode preset. Modes bundle common config combinations:
/// - `minimal`: no tools, just chat (lowest latency)
/// - `create`: write/edit tools only, no shell (safe file creation)
fn apply_agent_mode(cfg: &mut Config, mode: &str) -> Result<(), String> {
    match mode.to_ascii_lowercase().as_str() {
        "minimal" => {
            // `*` also covers tools that only appear at runtime (MCP servers,
            // long-term memory), which an explicit name list would miss.
            cfg.allowed_tools = vec![];
            cfg.disallowed_tools = vec!["*".into()];
        }
        "create" => {
            cfg.allowed_tools = vec![
                "write_file".into(),
                "edit_file".into(),
                "apply_patch".into(),
                "read_file".into(),
                "list_dir".into(),
            ];
        }
        other => {
            return Err(format!(
                "unknown mode `{other}` (expected: minimal, create)"
            ))
        }
    }
    Ok(())
}

/// Print the resolved configuration as JSON (for `--print-config`).
fn print_resolved_config(cfg: &Config) {
    let pmode = cfg.permission_mode.as_str();
    let ofmt = match cfg.output_format {
        config::OutputFormat::Text => "text",
        config::OutputFormat::Json => "json",
        config::OutputFormat::StreamJson => "stream-json",
    };
    let ifmt = match cfg.input_format {
        config::InputFormat::Text => "text",
        config::InputFormat::StreamJson => "stream-json",
    };
    let approval = match cfg.approval_policy {
        config::ApprovalPolicy::Never => "never",
        config::ApprovalPolicy::OnRequest => "on-request",
        config::ApprovalPolicy::Untrusted => "untrusted",
    };
    let sandbox = match cfg.sandbox {
        config::SandboxMode::Off => "off",
        config::SandboxMode::ReadOnly => "read-only",
        config::SandboxMode::WorkspaceWrite => "workspace-write",
        config::SandboxMode::DangerFullAccess => "danger-full-access",
    };
    let v = json!({
        "model": cfg.model,
        "base_url": cfg.base_url,
        "temperature": cfg.temperature,
        "max_tokens": cfg.max_tokens,
        "stream": cfg.stream,
        "workspace": cfg.workspace.display().to_string(),
        "auto_approve": cfg.auto_approve,
        "timeout_secs": cfg.timeout_secs,
        "max_retries": cfg.max_retries,
        "retry_base_ms": cfg.retry_base_ms,
        "permission_mode": pmode,
        "approval_policy": approval,
        "sandbox": sandbox,
        "allowed_tools": cfg.allowed_tools,
        "disallowed_tools": cfg.disallowed_tools,
        "fallback_model": cfg.fallback_model,
        "output_format": ofmt,
        "input_format": ifmt,
        "thinking": cfg.thinking,
        "project_docs": cfg.project_docs,
        "daemon": cfg.daemon,
        "daemon_port": cfg.daemon_port,
        "daemon_bind": cfg.daemon_bind,
        "daemon_token_set": !cfg.daemon_token.is_empty(),
        "mcp_config": cfg.mcp_config.as_ref().map(|p| p.display().to_string()),
        "subagents_config": cfg.subagents_config.as_ref().map(|p| p.display().to_string()),
        "hooks": cfg.hooks.len(),
    });
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// Parsed CLI result: optional (config, prompt tokens, image paths).
type CliParse = Result<Option<(Config, Vec<String>, Vec<PathBuf>)>, String>;

fn parse_args(args: Vec<String>) -> CliParse {
    // First pass: extract an explicit --config path (if any) before loading.
    let config_path = extract_config_path(&args);
    let print_config = args.iter().any(|a| a == "--print-config");
    let mut cfg = Config::default();

    // Apply config file (explicit --config, or NANOHERMES_CONFIG env, or ~/.nanohermes/config.json).
    if let Some(path) = config_path {
        if let Err(e) = cfg.apply_file(&path) {
            return Err(format!("config file error: {e}"));
        }
    } else if let Ok(path) = std::env::var("NANOHERMES_CONFIG") {
        let p = std::path::PathBuf::from(path);
        if p.exists() {
            if let Err(e) = cfg.apply_file(&p) {
                return Err(format!("config file error: {e}"));
            }
        }
    } else if let Some(p) = Config::default_config_path() {
        if p.exists() {
            if let Err(e) = cfg.apply_file(&p) {
                return Err(format!("config file error: {e}"));
            }
        }
    }

    let mut prompt: Vec<String> = Vec::new();
    let mut images: Vec<PathBuf> = Vec::new();

    let mut i = 0;
    let need = |i: &mut usize, args: &[String], flag: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("`{flag}` requires a value"))
    };

    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => {
                print_help();
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("NanoHermes Agent {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-m" | "--model" => cfg.model = need(&mut i, &args, a)?,
            "-k" | "--api-key" => cfg.api_key = need(&mut i, &args, a)?,
            "-u" | "--base-url" => cfg.base_url = need(&mut i, &args, a)?,
            "--fallback-model" => cfg.fallback_model = Some(need(&mut i, &args, a)?),
            "-C" | "--cwd" => {
                let dir = need(&mut i, &args, a)?;
                let p = PathBuf::from(dir);
                cfg.workspace = if p.is_absolute() {
                    p
                } else {
                    std::env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                        .join(p)
                };
            }
            "-i" | "--image" => {
                images.push(PathBuf::from(need(&mut i, &args, a)?));
            }
            "--mcp-config" => {
                cfg.mcp_config = Some(PathBuf::from(need(&mut i, &args, a)?));
            }
            "--session" => {
                cfg.session = Some(PathBuf::from(need(&mut i, &args, a)?));
            }
            "-c" | "--continue" => cfg.continue_session = true,
            "-r" | "--resume" => cfg.resume_session = true,
            "--timeout" => {
                let v = need(&mut i, &args, a)?;
                cfg.timeout_secs = v.parse().map_err(|_| format!("invalid number: {v}"))?;
            }
            "--max-retries" => {
                let v = need(&mut i, &args, a)?;
                cfg.max_retries = v.parse().map_err(|_| format!("invalid number: {v}"))?;
            }
            "--retry-base-ms" => {
                let v = need(&mut i, &args, a)?;
                cfg.retry_base_ms = v.parse().map_err(|_| format!("invalid number: {v}"))?;
            }
            "--temperature" => {
                let v = need(&mut i, &args, a)?;
                cfg.temperature = v.parse().map_err(|_| format!("invalid number: {v}"))?;
            }
            "--max-tokens" => {
                let v = need(&mut i, &args, a)?;
                cfg.max_tokens = v.parse().map_err(|_| format!("invalid number: {v}"))?;
            }
            "--no-stream" => cfg.stream = false,
            "--ask" => {
                cfg.approval_policy = config::ApprovalPolicy::Untrusted;
                cfg.auto_approve = false;
            }
            "-y" | "--yes" => {
                cfg.approval_policy = config::ApprovalPolicy::Never;
                cfg.auto_approve = true;
            }
            "--approval-policy" => {
                let v = need(&mut i, &args, a)?;
                let policy = config::ApprovalPolicy::parse(&v)?;
                cfg.approval_policy = policy;
                cfg.auto_approve = policy.is_never();
            }
            "--sandbox" => {
                let v = need(&mut i, &args, a)?;
                cfg.sandbox = config::SandboxMode::parse(&v)?;
            }
            "--permission-mode" => {
                let v = need(&mut i, &args, a)?;
                cfg.permission_mode = config::PermissionMode::parse(&v)?;
            }
            "--allowed-tools" => {
                let v = need(&mut i, &args, a)?;
                cfg.allowed_tools = v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "--disallowed-tools" => {
                let v = need(&mut i, &args, a)?;
                cfg.disallowed_tools = v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "--output-format" => {
                let v = need(&mut i, &args, a)?;
                cfg.output_format = config::OutputFormat::parse(&v)?;
            }
            "--input-format" => {
                let v = need(&mut i, &args, a)?;
                cfg.input_format = config::InputFormat::parse(&v)?;
            }
            "--subagents" => {
                cfg.subagents_config = Some(PathBuf::from(need(&mut i, &args, a)?));
            }
            "--thinking" => cfg.thinking = true,
            "--no-project-docs" => cfg.project_docs = false,
            "--daemon" => cfg.daemon = true,
            "--daemon-port" => {
                let v = need(&mut i, &args, a)?;
                cfg.daemon_port = v.parse().map_err(|_| format!("invalid port: {v}"))?;
            }
            "--daemon-bind" => {
                cfg.daemon_bind = need(&mut i, &args, a)?.to_string();
            }
            "--daemon-token" => {
                cfg.daemon_token = need(&mut i, &args, a)?.to_string();
            }
            "--config" => {
                // Already processed in the first pass; skip its value.
                let _ = need(&mut i, &args, a)?;
            }
            "--print-config" => {} // handled after parsing
            "--mode" => {
                let v = need(&mut i, &args, a)?;
                apply_agent_mode(&mut cfg, &v)?;
            }
            "--" => {
                prompt.extend(args[i + 1..].iter().cloned());
                break;
            }
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("unknown option `{s}`")),
            _ => prompt.push(args[i].clone()),
        }
        i += 1;
    }

    // --print-config: print resolved config as JSON and exit.
    if print_config {
        print_resolved_config(&cfg);
        return Ok(None);
    }

    // Handle session auto-continue: if -c is set and no explicit --session given,
    // derive a session file from the workspace directory hash.
    if cfg.continue_session && cfg.session.is_none() {
        cfg.session = Some(continue_session_path(&cfg.workspace));
    }

    // Handle --resume: list available sessions and let the user pick one.
    if cfg.resume_session && cfg.session.is_none() {
        match pick_session_interactively() {
            Some(path) => cfg.session = Some(path),
            None => {
                eprintln!("no session selected, starting a new conversation");
            }
        }
    }

    Ok(Some((cfg, prompt, images)))
}

fn print_help() {
    println!(
        r#"NanoHermes Agent {ver} — a lightweight terminal coding agent

USAGE:
    agent [OPTIONS] [PROMPT...]      one-shot: run PROMPT and exit
    agent [OPTIONS]                  interactive REPL (when stdin is a TTY)
    echo "PROMPT" | agent [OPTIONS]  one-shot with the prompt on stdin

OPTIONS:
    -m, --model <NAME>          Model name                 (env NANOHERMES_MODEL / OPENAI_MODEL)
    -u, --base-url <URL>        OpenAI-compatible base URL (env NANOHERMES_BASE_URL / OPENAI_BASE_URL)
    -k, --api-key <KEY>         API key                   (env NANOHERMES_API_KEY / OPENAI_API_KEY)
        --fallback-model <NAME> Fallback model used when the primary returns 429/5xx
    -C, --cwd <DIR>             Workspace root (default: current directory)
    -i, --image <PATH>          Attach an image (repeatable); requires a vision-capable model
        --mcp-config <FILE>     Path to an MCP servers config JSON (enables MCP tools)
        --subagents <FILE>      Path to a sub-agents config JSON (enables the subagent tool)
        --session <FILE>        Persist conversation history to FILE (loaded on startup if present)
    -c, --continue              Auto-load the most recent session for this directory
    -r, --resume                List saved sessions and pick one to resume
        --max-tokens <N>        Max response tokens (default 4096)
        --temperature <F>       Sampling temperature (default 0.0)
        --timeout <SECS>        HTTP request timeout (default 120)
        --max-retries <N>       Retries for transient API failures (default 2)
        --retry-base-ms <MS>    Base backoff delay between retries (default 500)
        --no-stream             Disable streaming (buffered response)
        --ask                   Confirm every shell command before running (untrusted)
        --approval-policy <P>   never | on-request | untrusted (default: never, no prompting)
        --sandbox <MODE>        off | read-only | workspace-write
                                (file tools: all platforms; commands: Linux Landlock)
    -y, --yes                   Auto-approve shell commands (default)
        --permission-mode <M>   default | plan | goal | debug
                                (plan = read-only, debug = no edits but commands run,
                                 goal = unattended)
        --allowed-tools <LIST>  Comma-separated allow-list of tool names
        --disallowed-tools <L>  Comma-separated deny-list of tool names
        --output-format <FMT>   text (default) | json | stream-json (newline-delimited JSON)
        --input-format <FMT>    text (default) | stream-json (parse stdin as JSON lines)
        --thinking              Enable extended thinking (reasoning_effort=high)
        --no-project-docs       Do not load AGENTS.md into the system prompt
        --daemon                Run as an HTTP daemon (POST {{prompt}} -> {{response}})
        --daemon-port <PORT>    Daemon listen port (default 8787)
        --daemon-bind <ADDR>    Daemon bind address (default 127.0.0.1)
        --daemon-token <TOKEN>  Bearer token required for daemon requests (or NANOHERMES_DAEMON_TOKEN)
        --config <FILE>         Load settings from a JSON config file
                                (also NANOHERMES_CONFIG or ~/.nanohermes/config.json)
        --mode <MODE>           Agent preset: minimal (no tools) | create (file tools only)
        --print-config          Print resolved config as JSON and exit
    -h, --help                  Show this help
    -V, --version               Show version

EXAMPLES:
    agent "add unit tests for src/tools/search.rs"
    agent --permission-mode plan "review the security of auth.rs"
    agent --allowed-tools read_file,grep,list_dir "explain the codebase"
    agent -c                              # resume the last session in this dir
    agent --output-format json "list files" | jq .response
    agent --daemon --daemon-port 8787     # start HTTP daemon

BUILT-IN TOOLS:
    read_file, write_file, edit_file, apply_patch, list_dir, grep, glob, run_command,
    web_fetch, web_search, todo_write, todo_read,
    git_status, git_diff, git_log, git_commit, task_output, task_list, task_kill
    (plus any tools exposed by MCP servers configured via --mcp-config)
"#,
        ver = env!("CARGO_PKG_VERSION")
    );
}

/// Derive a deterministic session-file path for a given workspace directory.
/// Sessions live under `~/.nanohermes/sessions/` keyed by a hash of the absolute
/// workspace path, so `-c` always resumes the right conversation.
fn continue_session_path(workspace: &std::path::Path) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    workspace.hash(&mut hasher);
    let hash = hasher.finish();
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = std::path::PathBuf::from(home)
        .join(".nanohermes")
        .join("sessions");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{hash:x}.json"))
}

/// List session files in `~/.nanohermes/sessions/`, let the user pick one by number,
/// and return its path. Returns None if the list is empty or the user cancels.
fn pick_session_interactively() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = std::path::PathBuf::from(home)
        .join(".nanohermes")
        .join("sessions");
    let mut entries: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(&dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        entries.push((modified, path));
                    }
                }
            }
        }
    }
    if entries.is_empty() {
        eprintln!("no saved sessions found");
        return None;
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.0)); // newest first

    println!("Available sessions:");
    for (i, (modified, path)) in entries.iter().enumerate() {
        let when = modified
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| format!("{}s ago", d.as_secs()))
            .unwrap_or_else(|_| "?".to_string());
        println!(
            "  [{}] {}  ({})",
            i + 1,
            path.file_name().unwrap_or_default().to_string_lossy(),
            when
        );
    }
    print!("Select a session (1-{}, or 0 to cancel): ", entries.len());
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return None;
    }
    match line.trim().parse::<usize>() {
        Ok(n) if n >= 1 && n <= entries.len() => Some(entries[n - 1].1.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_content_text_only_is_string() {
        let c = build_content("hello", &[]).unwrap();
        assert_eq!(c, json!("hello"));
    }

    #[test]
    fn build_content_attaches_images() {
        let dir = std::env::temp_dir().join(format!("nanohermes-img-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("pic.png");
        std::fs::write(&img, [0x89, 0x50, 0x4E, 0x47]).unwrap();

        let c = build_content("describe", std::slice::from_ref(&img)).unwrap();
        let arr = c.as_array().expect("content should be an array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "describe");
        assert_eq!(arr[1]["type"], "image_url");
        assert!(arr[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));

        // A missing file is an error, not a panic.
        assert!(build_content("x", &[dir.join("nope.png")]).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_ids_cannot_escape_the_session_directory() {
        for ok in ["default", "project-alpha", "a_b.c", "Team42"] {
            assert!(valid_session_id(ok), "{ok} should be accepted");
        }
        let long = "x".repeat(65);
        for bad in [
            "",
            ".",
            "..",
            "../evil",
            "a/b",
            "a\\b",
            "/etc/passwd",
            "C:evil",
            ".hidden",
            "nul",
            "CON",
            "COM1",
            "with space",
            "semi;colon",
            long.as_str(),
        ] {
            assert!(!valid_session_id(bad), "{bad:?} should be rejected");
        }
    }
}
