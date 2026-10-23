use std::env;
use std::path::PathBuf;

use serde_json::Value;

/// Permission mode controls which tools the agent is allowed to invoke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionMode {
    /// Normal operation: respect `auto_approve` for run_command, all tools available.
    Default,
    /// Auto-approve file edits (write_file / edit_file / apply_patch) but still
    /// require confirmation for shell commands.
    AcceptEdits,
    /// Read-only planning: mutating tools (write_file, edit_file, apply_patch,
    /// run_command) are rejected. Useful for safe code analysis.
    Plan,
    /// Goal-directed, unattended work: every tool is available and nothing asks
    /// for confirmation, so the agent keeps going until the objective is met.
    Goal,
    /// Diagnosis: reads files and runs commands to reproduce the problem, while
    /// the file-editing tools are blocked. Shell commands can still write, so
    /// pair this with `--sandbox read-only` for a hard filesystem guarantee.
    Debug,
}

impl PermissionMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "default" => Ok(PermissionMode::Default),
            "accept-edits" | "acceptedits" => Ok(PermissionMode::AcceptEdits),
            "plan" => Ok(PermissionMode::Plan),
            // "goat" is accepted because that is how the mode is commonly typed.
            "goal" | "goat" => Ok(PermissionMode::Goal),
            "debug" => Ok(PermissionMode::Debug),
            other => Err(format!(
                "unknown permission mode `{other}` (expected: default, accept-edits, plan, goal, debug)"
            )),
        }
    }

    /// Canonical name, used wherever a mode has to be echoed back out
    /// (CLI status output, JSON status, settings round-trip).
    pub fn as_str(self) -> &'static str {
        match self {
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "accept-edits",
            PermissionMode::Plan => "plan",
            PermissionMode::Goal => "goal",
            PermissionMode::Debug => "debug",
        }
    }

    /// Whether tool calls run without asking anyone, overriding `configured`.
    ///
    /// `AcceptEdits` always keeps shell confirmation on; `Goal` has nobody to
    /// ask, so it never blocks; the rest inherit the configured flag.
    pub fn auto_approve(self, configured: bool) -> bool {
        match self {
            PermissionMode::AcceptEdits => false,
            PermissionMode::Goal => true,
            PermissionMode::Default | PermissionMode::Plan | PermissionMode::Debug => configured,
        }
    }

    /// Effective approval policy, overriding `configured` when the mode has to
    /// answer for itself.
    pub fn approval(self, configured: ApprovalPolicy) -> ApprovalPolicy {
        match self {
            PermissionMode::Goal => ApprovalPolicy::Never,
            _ => configured,
        }
    }

    /// Tools that are blocked in this mode regardless of allow/deny lists.
    pub fn blocked_tools(self) -> &'static [&'static str] {
        match self {
            PermissionMode::Default | PermissionMode::AcceptEdits | PermissionMode::Goal => &[],
            PermissionMode::Plan => &[
                "write_file",
                "edit_file",
                "apply_patch",
                "run_command",
                "git_commit",
            ],
            // Diagnosis may run things, but must leave the tree untouched.
            PermissionMode::Debug => &["write_file", "edit_file", "apply_patch", "git_commit"],
        }
    }
}

/// When a shell command must be confirmed by the user before it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalPolicy {
    /// Never ask: run every command as-is (implies `auto_approve`).
    Never,
    /// Ask for anything that is not a recognised read-only command.
    OnRequest,
    /// Ask for every command, even read-only ones.
    Untrusted,
}

impl ApprovalPolicy {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().replace('_', "-").as_str() {
            "never" => Ok(ApprovalPolicy::Never),
            "on-request" | "onrequest" => Ok(ApprovalPolicy::OnRequest),
            "untrusted" => Ok(ApprovalPolicy::Untrusted),
            other => Err(format!(
                "unknown approval policy `{other}` (expected: never, on-request, untrusted)"
            )),
        }
    }

    /// True if this policy auto-approves everything without asking.
    pub fn is_never(self) -> bool {
        matches!(self, ApprovalPolicy::Never)
    }
}

/// Filesystem sandbox applied to spawned shell commands (Linux only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxMode {
    /// No sandboxing (default).
    Off,
    /// Read + execute everywhere; no filesystem writes at all.
    ReadOnly,
    /// Read + execute everywhere; writes allowed only under the workspace.
    WorkspaceWrite,
    /// Explicit alias for `off`: no restrictions.
    DangerFullAccess,
}

impl SandboxMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().replace('_', "-").as_str() {
            "off" | "none" => Ok(SandboxMode::Off),
            "read-only" | "readonly" => Ok(SandboxMode::ReadOnly),
            "workspace-write" | "workspacewrite" => Ok(SandboxMode::WorkspaceWrite),
            "danger-full-access" | "dangerfullaccess" | "full" => Ok(SandboxMode::DangerFullAccess),
            other => Err(format!(
                "unknown sandbox mode `{other}` (expected: off, read-only, workspace-write, danger-full-access)"
            )),
        }
    }

    /// True if this mode restricts the spawned process at all.
    pub fn is_active(self) -> bool {
        matches!(self, SandboxMode::ReadOnly | SandboxMode::WorkspaceWrite)
    }
}

/// Runtime configuration. Everything can be set via CLI flags (in `main.rs`)
/// or environment variables so the binary stays behaviour-light.
#[derive(Clone)]
pub struct Config {
    pub api_key: String,
    /// Base URL of an OpenAI-compatible API, without the trailing `/chat/completions`.
    pub base_url: String,
    pub model: String,
    pub temperature: f64,
    pub max_tokens: u32,
    /// Stream assistant tokens as they arrive (SSE).
    pub stream: bool,
    /// Hard cap on model<->tool round trips per user turn.
    pub max_iterations: usize,
    pub workspace: PathBuf,
    /// When false, shell commands ask for confirmation on stdin first.
    pub auto_approve: bool,
    /// When shell commands must be confirmed (refines `auto_approve`).
    pub approval_policy: ApprovalPolicy,
    /// Filesystem sandbox applied to spawned shell commands (Linux only).
    pub sandbox: SandboxMode,
    /// Per-HTTP-request timeout in seconds.
    pub timeout_secs: u64,
    /// How many times a transient failure (HTTP 429/5xx) is retried with
    /// exponential backoff before giving up (or falling back).
    pub max_retries: u32,
    /// Base delay in milliseconds for the exponential backoff between retries.
    pub retry_base_ms: u64,
    /// Optional path to an MCP servers config file (JSON).
    pub mcp_config: Option<PathBuf>,
    /// Optional path to a JSON file where the conversation is persisted after
    /// every turn (and loaded from on startup if it already exists).
    pub session: Option<PathBuf>,
    /// Permission mode controlling which tools may run.
    pub permission_mode: PermissionMode,
    /// If non-empty, only these tool names are allowed to execute.
    pub allowed_tools: Vec<String>,
    /// Tool names that are always denied.
    pub disallowed_tools: Vec<String>,
    /// Optional fallback model used when the primary model returns 429/5xx.
    pub fallback_model: Option<String>,
    /// Output format: "text" (default) or "json".
    pub output_format: OutputFormat,
    /// Input format for stdin: "text" (default) or "stream-json" (newline-delimited {"text":"..."}).
    pub input_format: InputFormat,
    /// When true, automatically continue the most recent session for this directory.
    pub continue_session: bool,
    /// When true, list available sessions and let the user pick one to resume.
    pub resume_session: bool,
    /// When true, enable extended thinking (reasoning_effort=high) for models that support it.
    pub thinking: bool,
    /// When true, AGENTS.md files (global + ancestors of the workspace) are
    /// loaded into the system prompt as project guidance.
    pub project_docs: bool,
    /// When true, long-term memory is on: relevant entries are injected into the
    /// system prompt each turn, and a finished turn is distilled into new ones.
    pub memory: bool,
    /// Where the long-term memory store lives. `None` means the default
    /// location (`~/.nanohermes`).
    pub memory_dir: Option<PathBuf>,
    /// Extra instructions appended to the built-in system prompt. This is how a
    /// user-selected persona customises the agent; empty/`None` leaves the
    /// built-in prompt untouched.
    pub system_prompt: Option<String>,
    /// When true, run as an HTTP daemon serving one-shot requests.
    pub daemon: bool,
    /// Port for the HTTP daemon (default 8787).
    pub daemon_port: u16,
    /// Bind address for the HTTP daemon (default 127.0.0.1).
    pub daemon_bind: String,
    /// Optional Bearer token the daemon requires on every request
    /// (`Authorization: Bearer <token>`). Empty means no auth.
    pub daemon_token: String,
    /// Optional path to a sub-agents config JSON file.
    pub subagents_config: Option<PathBuf>,
    /// Loaded sub-agent definitions (name -> prompt + optional allowed tools).
    pub subagents: Vec<SubAgentDef>,
    /// Lifecycle hooks run before/after every tool call.
    pub hooks: Vec<HookDef>,
}

/// When a hook fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookEvent {
    /// Runs before the tool. A non-zero exit blocks the call.
    PreToolUse,
    /// Runs after the tool. A non-zero exit only appends a warning.
    PostToolUse,
}

impl HookEvent {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "pretooluse" | "pre_tool_use" => Ok(HookEvent::PreToolUse),
            "posttooluse" | "post_tool_use" => Ok(HookEvent::PostToolUse),
            other => Err(format!(
                "unknown hook event `{other}` (expected: PreToolUse, PostToolUse)"
            )),
        }
    }
}

/// A command hook: `command` is run via the platform shell (`sh -c` on Unix,
/// `cmd.exe /C` on Windows) when a matching tool is called.
#[derive(Clone, Debug)]
pub struct HookDef {
    pub event: HookEvent,
    /// `*`/empty matches every tool, otherwise a comma-separated tool-name list.
    pub matcher: String,
    pub command: String,
}

impl HookDef {
    pub fn matches(&self, tool: &str) -> bool {
        let m = self.matcher.trim();
        if m.is_empty() || m == "*" {
            return true;
        }
        m.split(',').map(|s| s.trim()).any(|name| name == tool)
    }

    /// Parse the `hooks` array from a config file:
    /// `{"hooks": [{"event": "PreToolUse", "matcher": "run_command", "command": "..."}]}`
    fn load_all(obj: &serde_json::Map<String, Value>) -> Result<Vec<HookDef>, String> {
        let arr = match obj.get("hooks") {
            None => return Ok(Vec::new()),
            Some(a) => a
                .as_array()
                .ok_or_else(|| "`hooks` must be an array".to_string())?,
        };
        let mut out = Vec::new();
        for (i, spec) in arr.iter().enumerate() {
            let event = spec
                .get("event")
                .and_then(|e| e.as_str())
                .ok_or_else(|| format!("hooks[{i}] is missing `event`"))?;
            let command = spec
                .get("command")
                .and_then(|c| c.as_str())
                .ok_or_else(|| format!("hooks[{i}] is missing `command`"))?
                .to_string();
            let matcher = spec
                .get("matcher")
                .and_then(|m| m.as_str())
                .unwrap_or("*")
                .to_string();
            out.push(HookDef {
                event: HookEvent::parse(event)?,
                matcher,
                command,
            });
        }
        Ok(out)
    }
}

/// Definition of a sub-agent loaded from a config file.
#[derive(Clone, Debug)]
pub struct SubAgentDef {
    pub name: String,
    pub prompt: String,
    /// If non-empty, only these tools are available to the sub-agent.
    pub allowed_tools: Vec<String>,
    /// Optional model override for this sub-agent.
    pub model: Option<String>,
}

impl SubAgentDef {
    /// Load sub-agent definitions from a JSON file:
    /// `{ "subagents": { "<name>": { "prompt": "...", "allowed_tools": [...], "model": "..." } } }`
    pub fn load_all(path: &std::path::Path) -> Result<Vec<SubAgentDef>, String> {
        let data = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let v: Value =
            serde_json::from_str(&data).map_err(|e| format!("bad sub-agents config: {e}"))?;
        let obj = v
            .get("subagents")
            .and_then(|s| s.as_object())
            .ok_or("sub-agents config is missing a `subagents` object".to_string())?;
        let mut out = Vec::new();
        for (name, spec) in obj {
            let prompt = spec
                .get("prompt")
                .and_then(|p| p.as_str())
                .ok_or_else(|| format!("sub-agent `{name}` missing `prompt`"))?
                .to_string();
            let allowed_tools = spec
                .get("allowed_tools")
                .and_then(|a| a.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let model = spec
                .get("model")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string());
            out.push(SubAgentDef {
                name: name.clone(),
                prompt,
                allowed_tools,
                model,
            });
        }
        Ok(out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
    /// Newline-delimited JSON stream: each chunk is a JSON object on its own line.
    /// Useful for piping through `jq` or other streaming processors.
    StreamJson,
}

impl OutputFormat {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "text" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            "stream-json" => Ok(OutputFormat::StreamJson),
            other => Err(format!(
                "unknown output format `{other}` (expected: text, json, stream-json)"
            )),
        }
    }
}

/// How stdin input is interpreted when no CLI prompt is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputFormat {
    /// Plain text (default).
    Text,
    /// Newline-delimited JSON: each line is `{"text": "..."}`; all text fields
    /// are concatenated into one prompt. Useful for piping structured logs.
    StreamJson,
}

impl InputFormat {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "text" => Ok(InputFormat::Text),
            "stream-json" => Ok(InputFormat::StreamJson),
            other => Err(format!(
                "unknown input format `{other}` (expected: text, stream-json)"
            )),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            api_key: env_first(&["NANOHERMES_API_KEY", "OPENAI_API_KEY"]).unwrap_or_default(),
            base_url: env_first(&["NANOHERMES_BASE_URL", "OPENAI_BASE_URL"])
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: env_first(&["NANOHERMES_MODEL", "OPENAI_MODEL"])
                .unwrap_or_else(|| "gpt-4o-mini".to_string()),
            temperature: 0.0,
            max_tokens: 4096,
            stream: true,
            max_iterations: 40,
            workspace: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            auto_approve: true,
            approval_policy: ApprovalPolicy::Untrusted,
            sandbox: SandboxMode::Off,
            timeout_secs: 120,
            max_retries: env_first(&["NANOHERMES_MAX_RETRIES"])
                .and_then(|v| v.parse().ok())
                .unwrap_or(2),
            retry_base_ms: env_first(&["NANOHERMES_RETRY_BASE_MS"])
                .and_then(|v| v.parse().ok())
                .unwrap_or(500),
            mcp_config: None,
            session: None,
            permission_mode: PermissionMode::Default,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            fallback_model: None,
            output_format: OutputFormat::Text,
            input_format: InputFormat::Text,
            continue_session: false,
            resume_session: false,
            thinking: false,
            project_docs: true,
            memory: true,
            memory_dir: None,
            system_prompt: None,
            daemon: false,
            daemon_port: 8787,
            daemon_bind: "127.0.0.1".to_string(),
            daemon_token: env_first(&["NANOHERMES_DAEMON_TOKEN"]).unwrap_or_default(),
            subagents_config: None,
            subagents: Vec::new(),
            hooks: Vec::new(),
        }
    }
}

impl Config {
    /// Full endpoint URL for chat completions.
    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// Load a config file (JSON) and apply its values on top of the current
    /// config. Fields not present in the file are left unchanged. CLI flags
    /// should be applied after this to override file settings.
    pub fn apply_file(&mut self, path: &std::path::Path) -> Result<(), String> {
        let data = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let v: Value = serde_json::from_str(&data)
            .map_err(|e| format!("bad config file {}: {e}", path.display()))?;
        let obj = v
            .as_object()
            .ok_or_else(|| format!("config file {} is not a JSON object", path.display()))?;

        if let Some(s) = obj.get("api_key").and_then(|x| x.as_str()) {
            self.api_key = s.to_string();
        }
        if let Some(s) = obj.get("base_url").and_then(|x| x.as_str()) {
            self.base_url = s.to_string();
        }
        if let Some(s) = obj.get("model").and_then(|x| x.as_str()) {
            self.model = s.to_string();
        }
        if let Some(n) = obj.get("temperature").and_then(|x| x.as_f64()) {
            self.temperature = n;
        }
        if let Some(n) = obj.get("max_tokens").and_then(|x| x.as_u64()) {
            self.max_tokens = n as u32;
        }
        if let Some(b) = obj.get("stream").and_then(|x| x.as_bool()) {
            self.stream = b;
        }
        if let Some(n) = obj.get("max_iterations").and_then(|x| x.as_u64()) {
            self.max_iterations = n as usize;
        }
        if let Some(s) = obj.get("cwd").and_then(|x| x.as_str()) {
            self.workspace = PathBuf::from(s);
        }
        if let Some(b) = obj.get("auto_approve").and_then(|x| x.as_bool()) {
            self.auto_approve = b;
        }
        if let Some(s) = obj.get("approval_policy").and_then(|x| x.as_str()) {
            self.approval_policy = ApprovalPolicy::parse(s)?;
            // The policy is authoritative and also drives the auto-approve flag.
            self.auto_approve = self.approval_policy.is_never();
        }
        if let Some(s) = obj.get("sandbox").and_then(|x| x.as_str()) {
            self.sandbox = SandboxMode::parse(s)?;
        }
        if let Some(n) = obj.get("timeout_secs").and_then(|x| x.as_u64()) {
            self.timeout_secs = n;
        }
        if let Some(n) = obj.get("max_retries").and_then(|x| x.as_u64()) {
            self.max_retries = n as u32;
        }
        if let Some(n) = obj.get("retry_base_ms").and_then(|x| x.as_u64()) {
            self.retry_base_ms = n;
        }
        if let Some(s) = obj.get("mcp_config").and_then(|x| x.as_str()) {
            self.mcp_config = Some(PathBuf::from(s));
        }
        if let Some(s) = obj.get("session").and_then(|x| x.as_str()) {
            self.session = Some(PathBuf::from(s));
        }
        if let Some(s) = obj.get("permission_mode").and_then(|x| x.as_str()) {
            self.permission_mode = PermissionMode::parse(s)?;
        }
        if let Some(arr) = obj.get("allowed_tools").and_then(|x| x.as_array()) {
            self.allowed_tools = arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(arr) = obj.get("disallowed_tools").and_then(|x| x.as_array()) {
            self.disallowed_tools = arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(s) = obj.get("fallback_model").and_then(|x| x.as_str()) {
            self.fallback_model = Some(s.to_string());
        }
        if let Some(s) = obj.get("output_format").and_then(|x| x.as_str()) {
            self.output_format = OutputFormat::parse(s)?;
        }
        if let Some(s) = obj.get("input_format").and_then(|x| x.as_str()) {
            self.input_format = InputFormat::parse(s)?;
        }
        if let Some(b) = obj.get("continue_session").and_then(|x| x.as_bool()) {
            self.continue_session = b;
        }
        if let Some(b) = obj.get("resume_session").and_then(|x| x.as_bool()) {
            self.resume_session = b;
        }
        if let Some(b) = obj.get("thinking").and_then(|x| x.as_bool()) {
            self.thinking = b;
        }
        if let Some(b) = obj.get("project_docs").and_then(|x| x.as_bool()) {
            self.project_docs = b;
        }
        if let Some(b) = obj.get("memory").and_then(|x| x.as_bool()) {
            self.memory = b;
        }
        if let Some(s) = obj.get("memory_dir").and_then(|x| x.as_str()) {
            // Blank means "use the default location", not "disable memory".
            self.memory_dir = Some(PathBuf::from(s)).filter(|p| !p.as_os_str().is_empty());
        }
        if let Some(s) = obj.get("system_prompt").and_then(|x| x.as_str()) {
            // A blank value means "no persona", not "clear the prompt".
            self.system_prompt = Some(s.to_string()).filter(|v| !v.trim().is_empty());
        }
        if let Some(b) = obj.get("daemon").and_then(|x| x.as_bool()) {
            self.daemon = b;
        }
        if let Some(n) = obj.get("daemon_port").and_then(|x| x.as_u64()) {
            self.daemon_port = n as u16;
        }
        if let Some(s) = obj.get("daemon_bind").and_then(|x| x.as_str()) {
            self.daemon_bind = s.to_string();
        }
        if let Some(s) = obj.get("daemon_token").and_then(|x| x.as_str()) {
            if !s.is_empty() {
                self.daemon_token = s.to_string();
            }
        }
        if let Some(s) = obj.get("subagents_config").and_then(|x| x.as_str()) {
            self.subagents_config = Some(PathBuf::from(s));
        }
        self.hooks = HookDef::load_all(obj)?;
        Ok(())
    }

    /// Return the default config file path: `~/.nanohermes/config.json`.
    pub fn default_config_path() -> Option<PathBuf> {
        let home = env::var("HOME").ok()?;
        Some(PathBuf::from(home).join(".nanohermes").join("config.json"))
    }

    /// Whether a tool name is permitted under the current permission mode,
    /// allow-list and deny-list. Returns Ok(()) if allowed, Err(reason) if not.
    pub fn check_tool(&self, name: &str) -> Result<(), String> {
        // Hard block from the permission mode (e.g. plan mode blocks mutations).
        if self.permission_mode.blocked_tools().contains(&name) {
            return Err(format!(
                "tool `{name}` is not available in {:?} mode",
                self.permission_mode
            ));
        }
        // Deny list always wins. `*` denies every tool, including tools that are
        // discovered later (MCP servers, memory) rather than enumerated by name.
        if self.disallowed_tools.iter().any(|t| t == "*" || t == name) {
            return Err(format!("tool `{name}` is disallowed by configuration"));
        }
        // Allow list: if set, only listed tools may run.
        if !self.allowed_tools.is_empty() && !self.allowed_tools.iter().any(|t| t == name) {
            return Err(format!("tool `{name}` is not in the allowed list"));
        }
        Ok(())
    }
}

fn env_first(keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Ok(v) = env::var(k) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_list_wildcard_blocks_every_tool() {
        let cfg = Config {
            disallowed_tools: vec!["*".to_string()],
            ..Config::default()
        };
        assert!(cfg.check_tool("read_file").is_err());
        // Tools discovered at runtime (MCP / memory) are covered too.
        assert!(cfg.check_tool("memory_search").is_err());
        assert!(cfg.check_tool("some_mcp_tool").is_err());
    }

    #[test]
    fn named_deny_still_only_blocks_that_tool() {
        let cfg = Config {
            disallowed_tools: vec!["run_command".to_string()],
            ..Config::default()
        };
        assert!(cfg.check_tool("run_command").is_err());
        assert!(cfg.check_tool("read_file").is_ok());
    }
}
