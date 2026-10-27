use serde_json::{json, Value};

use crate::config::{Config, HookEvent};
use crate::hooks;
use crate::llm::{self, AssistantTurn};
use crate::mcp::Manager as McpManager;
use crate::memory;
use crate::prompt;
use crate::tools::{self, Ctx};
use crate::ui::{Style, UiSink};

use std::path::PathBuf;

/// Approximate character budget for the conversation. Once exceeded we drop the
/// oldest turns (keeping the system prompt + recent context) to bound memory.
const CONTEXT_CHAR_BUDGET: usize = 120_000;
/// Minimum number of most-recent messages always kept when compacting.
const KEEP_RECENT: usize = 10;
/// How many sub-agents may run at the same time within one turn.
const MAX_PARALLEL_SUBAGENTS: usize = 4;

/// Bound on tool-call threads spawned at once. The model controls how many
/// calls it returns; without a cap, one turn could spawn hundreds of OS threads
/// (and hundreds of shell processes) simultaneously.
const MAX_PARALLEL_TOOL_CALLS: usize = 8;

/// Cumulative token usage across every model request in the session.
#[derive(Default, Clone, Copy, Debug)]
pub struct UsageTotals {
    pub requests: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl UsageTotals {
    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

/// How many images one turn accepts. Extra ones are dropped rather than silently
/// inflating the request (and the stored session file).
pub const MAX_ATTACHED_IMAGES: usize = 4;

/// Build the user message body for one turn: a plain string, or a multimodal
/// content array when the caller attached images. Images arrive as
/// `data:<mime>;base64,<payload>` URLs produced by the client, so nothing here
/// has to decode or re-encode them. Anything that is not an image data URL is
/// dropped — a broken part would fail the whole request at the API instead.
pub fn user_content(text: &str, images: &[String]) -> Value {
    let parts: Vec<Value> = images
        .iter()
        .filter(|url| url.starts_with("data:image/"))
        .take(MAX_ATTACHED_IMAGES)
        .map(|url| json!({ "type": "image_url", "image_url": { "url": url } }))
        .collect();
    if parts.is_empty() {
        return json!(text);
    }
    let mut content = vec![json!({ "type": "text", "text": text })];
    content.extend(parts);
    Value::Array(content)
}

/// Installs the Ctrl-C takeover handler for the duration of one turn and
/// restores the default disposition when the turn ends (including on early
/// error returns, via `Drop`). Ref-counted so nested turns (e.g. a sub-agent
/// running inside a tool call) do not tear down the outer handler.
struct InterruptGuard;

impl InterruptGuard {
    fn new() -> Self {
        crate::interrupt::take(); // discard any stale flag from an earlier turn
        crate::interrupt::install();
        InterruptGuard
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        crate::interrupt::restore_default();
    }
}

pub struct Agent {
    cfg: Config,
    ctx: Ctx,
    /// `auto_approve` as configured, before the permission mode overrides it.
    /// Kept so switching modes back to `Default` restores the user's setting.
    base_auto_approve: bool,
    /// Approval policy as configured, before the permission mode overrides it.
    base_approval: crate::config::ApprovalPolicy,
    style: Style,
    /// Full conversation in OpenAI message format, kept across REPL turns.
    messages: Vec<Value>,
    /// Optional MCP servers whose tools are merged into the tool list.
    mcp: Option<McpManager>,
    /// Optional JSON file the conversation is persisted to after every turn.
    session: Option<PathBuf>,
    /// Accumulated token usage for the session.
    usage: UsageTotals,
}

impl Agent {
    pub fn with_mcp(mut cfg: Config, mcp: Option<McpManager>) -> Agent {
        // JSON output mode needs clean stdout, so disable streaming.
        // StreamJson keeps streaming (each delta is a JSON line).
        if matches!(cfg.output_format, crate::config::OutputFormat::Json) {
            cfg.stream = false;
        }
        // The permission mode may override the configured approval behaviour
        // (e.g. Goal never blocks). Remember the configured values so leaving
        // the mode restores them.
        let base_auto_approve = cfg.auto_approve;
        let base_approval = cfg.approval_policy;
        let auto_approve = cfg.permission_mode.auto_approve(base_auto_approve);
        let approval = cfg.permission_mode.approval(base_approval);
        cfg.auto_approve = auto_approve;
        cfg.approval_policy = approval;
        let ctx = Ctx {
            root: cfg.workspace.clone(),
            auto_approve,
            approval,
            sandbox: cfg.sandbox,
            max_output: 20_000,
            tasks: crate::tools::tasks::TaskManager::new(),
            todos: crate::tools::todo::TodoList::new(),
            skills: crate::skills::discover(&cfg.workspace),
            board: crate::board::Board::new(),
            gate: crate::board::Gate::new(MAX_PARALLEL_SUBAGENTS),
            // 记忆功能关闭时置 None，工具层据此禁用 memory_save/memory_search。
            memory_dir: if cfg.memory {
                cfg.memory_dir.clone().or_else(memory::default_dir)
            } else {
                None
            },
        };
        let session = cfg.session.clone();
        let messages = match &session {
            Some(path) => match load_session_file(path) {
                // The system prompt is configuration (personas, AGENTS.md, mode),
                // not conversation: refresh it so a reloaded task picks up the
                // current settings instead of replaying a stale prompt.
                Ok(mut loaded) => {
                    refresh_system(&mut loaded, &cfg, &ctx.skills);
                    loaded
                }
                Err(e) => {
                    eprintln!("warning: could not load session {}: {e}", path.display());
                    default_messages(&cfg, &ctx.skills)
                }
            },
            None => default_messages(&cfg, &ctx.skills),
        };
        Agent {
            cfg,
            ctx,
            base_auto_approve,
            base_approval,
            style: Style::detect(),
            messages,
            mcp,
            session,
            usage: UsageTotals::default(),
        }
    }

    /// Cumulative token usage for the session so far.
    pub fn usage(&self) -> UsageTotals {
        self.usage
    }

    pub fn style(&self) -> Style {
        self.style.clone()
    }

    /// Route all user-facing output (streamed text, tool banners, status notes)
    /// to `sink` instead of the terminal. Used by the desktop GUI, which has no
    /// TTY. Passing the same closure to every agent keeps one event channel.
    pub fn set_ui_sink(&mut self, sink: UiSink) {
        self.style = Style::to_sink(sink);
    }

    /// Drop an installed [`UiSink`] and go back to writing to the terminal.
    /// The daemon uses this after streaming a turn to one client, so later turns
    /// do not keep writing into a socket that is already closed.
    pub fn clear_ui_sink(&mut self) {
        self.style = Style::detect();
    }

    /// Turn token streaming on or off. A streaming HTTP client asks for deltas
    /// explicitly, which must work even when the daemon was started in a mode
    /// that disabled streaming.
    pub fn set_stream(&mut self, on: bool) {
        self.cfg.stream = on;
    }

    /// Expose the config so callers (e.g. main for output-format decisions)
    /// can read runtime settings without cloning the whole agent.
    pub fn cfg(&self) -> &Config {
        &self.cfg
    }

    /// Drop the conversation history but keep the system prompt.
    pub fn reset(&mut self) {
        self.messages = default_messages(&self.cfg, &self.ctx.skills);
        self.persist();
    }

    /// Rewind the conversation by `turns` full user turns: from the last user
    /// message backwards, each turn (its user message plus everything after it)
    /// is removed. Returns the number of turns actually removed.
    pub fn rewind(&mut self, turns: usize) -> usize {
        let mut removed = 0;
        for _ in 0..turns.max(1) {
            let last_user = self
                .messages
                .iter()
                .rposition(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"));
            match last_user {
                Some(idx) if idx > 0 => {
                    self.messages.truncate(idx);
                    removed += 1;
                }
                _ => break,
            }
        }
        if removed > 0 {
            self.persist();
        }
        removed
    }

    /// Switch the active model at runtime (used by the REPL `/model` command).
    pub fn set_model(&mut self, model: &str) {
        self.cfg.model = model.to_string();
    }

    /// Switch the permission mode at runtime.
    pub fn set_permission_mode(&mut self, mode: crate::config::PermissionMode) {
        self.cfg.permission_mode = mode;
        self.rebuild_auto_approve();
    }

    /// Switch the output format at runtime.
    pub fn set_output_format(&mut self, fmt: crate::config::OutputFormat) {
        self.cfg.output_format = fmt;
        if matches!(fmt, crate::config::OutputFormat::Json) {
            self.cfg.stream = false;
        }
    }

    /// Re-derive the effective approval behaviour from the current permission
    /// mode and push it into both the config and the tool context, so that a
    /// runtime mode switch actually changes how tools gate themselves.
    fn rebuild_auto_approve(&mut self) {
        let auto_approve = self
            .cfg
            .permission_mode
            .auto_approve(self.base_auto_approve);
        let approval = self.cfg.permission_mode.approval(self.base_approval);
        self.cfg.auto_approve = auto_approve;
        self.cfg.approval_policy = approval;
        self.ctx.auto_approve = auto_approve;
        self.ctx.approval = approval;
    }

    /// Save the current conversation to a specific file (regardless of --session).
    pub fn save_session_to(&self, path: &std::path::Path) -> Result<(), String> {
        save_session_file(path, &self.messages)
    }

    /// Replace the current conversation with one loaded from a file.
    pub fn load_session_from(&mut self, path: &std::path::Path) -> Result<(), String> {
        let msgs = load_session_file(path)?;
        self.messages = msgs;
        Ok(())
    }

    /// Swap the conversation message vector, returning the previous one.
    /// Used by the daemon to multiplex several isolated sessions over one agent.
    pub fn replace_messages(&mut self, msgs: Vec<Value>) -> Vec<Value> {
        std::mem::replace(&mut self.messages, msgs)
    }

    /// Build a fresh conversation (system prompt only) for a new session.
    pub fn fresh_messages(&self) -> Vec<Value> {
        default_messages(&self.cfg, &self.ctx.skills)
    }

    /// Number of messages currently in the conversation.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Return a list of (name, description) for every tool currently available
    /// to the agent, filtered by the active permission rules.
    pub fn available_tools(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let specs = self.build_tools();
        if let Some(arr) = specs.as_array() {
            for t in arr {
                let name = t
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let desc = t
                    .get("function")
                    .and_then(|f| f.get("description"))
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push((name, desc));
            }
        }
        out
    }

    /// Persist the current conversation to the session file (if any).
    fn persist(&self) {
        if let Some(path) = &self.session {
            if let Err(e) = save_session_file(path, &self.messages) {
                eprintln!("warning: could not save session {}: {e}", path.display());
            }
        }
    }

    /// Run one user turn to completion (model <-> tools until the model stops calling tools).
    /// `content` is the user message body: either a plain string or a content-array
    /// (for multimodal input, e.g. text + image_url parts).
    /// Returns the final assistant text response.
    pub fn run(&mut self, content: Value) -> Result<String, String> {
        // A Ctrl-C during this turn sets a flag instead of killing the process.
        let _guard = InterruptGuard::new();
        // 跨会话记忆：按当前指令检索相关记忆，整体替换系统消息里的注入块
        // （每轮重检索、按标记替换，不会越积越多）。
        let user_text = memory::content_text(&content);
        if let Some(dir) = self.ctx.memory_dir.clone() {
            let block = memory::retrieve_block(&dir, &user_text);
            self.set_memory_block(&block);
        }
        self.messages
            .push(json!({ "role": "user", "content": content }));

        let mut completed = false;
        let mut interrupted = false;
        let mut had_tools = false;
        let mut final_text = String::new();
        // 无迭代上限：一直跑到模型不再调用工具、被停止或请求出错为止。
        loop {
            // Abort only at a step boundary so the history never keeps an
            // assistant tool-call without its matching tool result.
            if crate::interrupt::take() {
                interrupted = true;
                break;
            }
            // Persist on a failed request too: the user message (and any tool
            // exchanges from earlier iterations) must survive so that fixing
            // the API key and retrying does not silently drop the turn.
            let turn = match self.complete() {
                Ok(turn) => turn,
                Err(e) => {
                    self.persist();
                    return Err(e);
                }
            };
            self.usage.requests += 1;
            self.usage.prompt_tokens += turn.usage.prompt_tokens;
            self.usage.completion_tokens += turn.usage.completion_tokens;
            // Report this one request's increment rather than the running total:
            // the cloud daemon serves many tasks from a single agent, so its
            // totals span every conversation. Sending increments lets any client
            // build a correct per-task total, and locally they add up to the same
            // number anyway.
            self.style
                .usage(1, turn.usage.prompt_tokens, turn.usage.completion_tokens);

            if !turn.content.trim().is_empty() && !self.style.is_silent() {
                println!(); // terminate the streamed text line (terminal only)
            }
            final_text = turn.content.clone();
            self.messages.push(assistant_message(&turn));

            if turn.tool_calls.is_empty() {
                completed = true;
                break;
            }
            had_tools = true;

            // Execute every tool call in this turn concurrently. Results are
            // collected in call order and appended together so the model sees a
            // consistent tool-call/tool-result pairing regardless of which call
            // finished first.
            let results: Vec<String> = if turn.tool_calls.len() == 1 {
                vec![self.execute(&turn.tool_calls[0])]
            } else {
                // Reborrow as `&Agent` so the scoped threads can share it
                // (`execute` takes `&self`, and `Agent` is `Sync`).
                let this: &Agent = self;
                std::thread::scope(|s| {
                    let mut results = Vec::with_capacity(turn.tool_calls.len());
                    // Run in bounded batches: keeps ordering, avoids a
                    // model-controlled thread/process spike.
                    for batch in turn.tool_calls.chunks(MAX_PARALLEL_TOOL_CALLS) {
                        let handles: Vec<_> = batch
                            .iter()
                            .map(|call| s.spawn(move || this.execute(call)))
                            .collect();
                        results.extend(handles.into_iter().map(|h| {
                            h.join()
                                .unwrap_or_else(|_| "ERROR: tool thread panicked".to_string())
                        }));
                    }
                    results
                })
            };

            for (call, result) in turn.tool_calls.iter().zip(results) {
                self.style
                    .tool_call(&call.name, &summarize(&call.name, &call.arguments), &result);
                self.messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": result,
                }));
            }
            self.compact();
        }

        if interrupted {
            // A GUI has no `/undo`, so it gets a plain description instead of the
            // REPL's hint.
            if self.style.is_silent() {
                self.style
                    .warn("已停止：当前步骤执行完毕，对话上下文已保留");
            } else {
                self.style.warn(
                    "interrupted — the conversation was kept; type a new instruction or /undo to rewind",
                );
            }
        }
        // 对话收尾自动提炼：只在完整结束的轮次沉淀（中断的半截对话不提炼），
        // 提炼失败只影响记忆，不影响对话结果。
        if completed && self.cfg.memory {
            self.distill_memories(&user_text, had_tools);
        }
        self.persist();
        Ok(final_text)
    }

    /// 用新的长期记忆块替换系统消息里的旧块（空块即删除），保证每轮按
    /// 当前用户消息重新检索、不随对话轮次累积。
    fn set_memory_block(&mut self, block: &str) {
        if let Some(system) = self.messages.first_mut() {
            if system.get("role").and_then(|r| r.as_str()) == Some("system") {
                if let Some(text) = system.get("content").and_then(|c| c.as_str()) {
                    let updated = memory::set_block(text, block);
                    system["content"] = json!(updated);
                }
            }
        }
    }

    /// 对话收尾提炼经验/教训/偏好并写回长期记忆库。廉价门槛过滤寒暄类短消息，
    /// 任何失败都只静默降级（记忆是加分项，不打断对话）。
    fn distill_memories(&mut self, user_text: &str, had_tools: bool) {
        let dir = match self.ctx.memory_dir.clone() {
            Some(d) => d,
            None => return,
        };
        if !memory::worth_distilling(user_text, had_tools) {
            return;
        }
        let excerpt = memory::excerpt_from(&self.messages, 6000);
        let items = match memory::distill(&self.cfg, &excerpt) {
            Ok(items) => items,
            Err(_) => return,
        };
        let mut saved = 0usize;
        for (kind, text) in items {
            if memory::remember(&dir, kind, &text, "auto").unwrap_or(false) {
                saved += 1;
            }
        }
        if saved > 0 {
            self.style
                .note(&format!("已从本次对话沉淀 {saved} 条长期记忆"));
        }
    }

    /// One model request, streaming any text to stdout.
    fn complete(&self) -> Result<AssistantTurn, String> {
        let style = self.style.clone();
        let tools = self.build_tools();
        match self.cfg.output_format {
            crate::config::OutputFormat::StreamJson => {
                // Each streamed delta is emitted as a newline-delimited JSON object.
                llm::chat(&self.cfg, &self.messages, &tools, |delta| {
                    if !delta.is_empty() {
                        let chunk = json!({ "delta": delta }).to_string();
                        println!("{chunk}");
                    }
                })
            }
            _ => llm::chat(&self.cfg, &self.messages, &tools, |delta| {
                style.stream(delta)
            }),
        }
    }

    /// Built-in tools plus any tools exposed by configured MCP servers.
    /// Tools blocked by the permission mode or deny-list are filtered out so
    /// the model never tries to call them.
    fn build_tools(&self) -> Value {
        let mut list = tools::specs().as_array().cloned().unwrap_or_default();
        // Only advertise the `skill` tool when skills were actually discovered.
        if !self.ctx.skills.is_empty() {
            list.push(skill_tool_spec());
        }
        if let Some(m) = &self.mcp {
            list.extend(m.all_tools());
            list.extend(mcp_aux_tools());
        }
        // Add the subagent delegation tool if any sub-agents are configured.
        // It goes in before filtering so mode and allow/deny lists apply to it
        // as well.
        if !self.cfg.subagents.is_empty() {
            list.push(subagent_tool_spec());
            list.push(agent_list_tool_spec());
        }
        // Drop any tool whose name is not permitted by the current config.
        list.retain(|t| {
            let name = t
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            self.cfg.check_tool(name).is_ok()
        });
        Value::Array(list)
    }

    /// Execute a single tool call, turning any error into a tool result string.
    /// Runs the PreToolUse hooks first (they may veto the call) and the
    /// PostToolUse hooks afterwards (they may only add a warning).
    fn execute(&self, call: &llm::ToolCall) -> String {
        // Enforce permission mode + allow/deny lists before doing anything.
        if let Err(reason) = self.cfg.check_tool(&call.name) {
            return format!("ERROR: {reason}");
        }

        let args: Value = if call.arguments.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str(&call.arguments) {
                Ok(v) => v,
                Err(e) => return format!("ERROR: invalid JSON arguments for `{}`: {e}", call.name),
            }
        };

        for hook in self
            .cfg
            .hooks
            .iter()
            .filter(|h| h.event == HookEvent::PreToolUse && h.matches(&call.name))
        {
            match hooks::run(hook, &call.name, &args, &self.cfg.workspace) {
                Ok(o) if o.ok() => {}
                Ok(o) => {
                    return format!(
                        "ERROR: `{}` blocked by PreToolUse hook (exit {}):\n{}",
                        call.name, o.code, o.output
                    )
                }
                Err(e) => return format!("ERROR: PreToolUse hook failed: {e}"),
            }
        }

        let mut result = self.run_tool(&call.name, &args);

        for hook in self
            .cfg
            .hooks
            .iter()
            .filter(|h| h.event == HookEvent::PostToolUse && h.matches(&call.name))
        {
            match hooks::run(hook, &call.name, &args, &self.cfg.workspace) {
                Ok(o) if o.ok() => {}
                Ok(o) => result.push_str(&format!(
                    "\n[PostToolUse hook exited {}]\n{}",
                    o.code, o.output
                )),
                Err(e) => result.push_str(&format!("\n[PostToolUse hook failed: {e}]")),
            }
        }
        result
    }

    /// The tool dispatch itself (permissions and hooks already handled).
    fn run_tool(&self, name: &str, args: &Value) -> String {
        // MCP resources/prompts are routed directly to the manager.
        if let Some(m) = &self.mcp {
            match name {
                "mcp_list_resources" => return render_resources(m.list_resources()),
                "mcp_read_resource" => {
                    let uri = args.get("uri").and_then(|u| u.as_str()).unwrap_or("");
                    return result_string(m.read_resource(uri));
                }
                "mcp_list_prompts" => return render_prompts(m.list_prompts()),
                "mcp_get_prompt" => {
                    let pname = args.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let pa = args.get("arguments").cloned().unwrap_or(json!({}));
                    return result_string(m.get_prompt(pname, &pa));
                }
                _ => {}
            }
        }

        // Sub-agent delegation.
        if name == "subagent" {
            let sname = args.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let task = args.get("task").and_then(|t| t.as_str()).unwrap_or("");
            return match self.run_subagent(sname, task) {
                Ok(s) => s,
                Err(e) => format!("ERROR: {e}"),
            };
        }

        match tools::dispatch(name, args, &self.ctx) {
            Ok(s) => {
                // The plan is worth showing live, but it must not go straight to
                // stdout: `json` / `stream-json` output has to stay machine-readable.
                if name == "todo_write" {
                    self.style.note(&s);
                }
                s
            }
            Err(e) => {
                // Built-in tools don't know this name; try MCP servers.
                if e.contains("unknown tool") {
                    if let Some(m) = &self.mcp {
                        if let Some(result) = m.call(name, args) {
                            return match result {
                                Ok(s) => s,
                                Err(err) => format!("ERROR: {err}"),
                            };
                        }
                    }
                }
                format!("ERROR: {e}")
            }
        }
    }

    /// Delegate a task to a named sub-agent. The sub-agent runs with its own
    /// system prompt and (optionally) a restricted tool set and model. Returns
    /// the sub-agent's final text response.
    ///
    /// Every delegation is recorded on the shared board (so progress can be
    /// inspected with `agent_list`) and bounded by the concurrency gate, since
    /// a single turn may request several delegations at once.
    fn run_subagent(&self, name: &str, task: &str) -> Result<String, String> {
        let id = self.ctx.board.start(name, task);
        let _permit = self.ctx.gate.acquire();
        let result = self.run_subagent_inner(name, task);
        match &result {
            Ok(text) => self.ctx.board.finish_ok(&id, text),
            Err(err) => self.ctx.board.finish_err(&id, err),
        }
        result
    }

    fn run_subagent_inner(&self, name: &str, task: &str) -> Result<String, String> {
        let def = self
            .cfg
            .subagents
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("unknown sub-agent `{name}`"))?;

        // Build a per-sub-agent config: optional model override.
        let mut sub_cfg = self.cfg.clone();
        if let Some(m) = &def.model {
            sub_cfg.model = m.clone();
        }
        // Sub-agents never stream (we want the full response to return).
        sub_cfg.stream = false;

        // Build the tool list: full set filtered by the sub-agent's allow-list.
        let all_tools = self.build_tools();
        let tool_list: Vec<Value> = if !def.allowed_tools.is_empty() {
            all_tools
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|t| {
                    let tname = t
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    def.allowed_tools.iter().any(|a| a == tname)
                })
                .collect()
        } else {
            all_tools.as_array().cloned().unwrap_or_default()
        };
        // Exclude the `subagent` tool itself to prevent infinite nesting.
        let tool_list: Vec<Value> = tool_list
            .into_iter()
            .filter(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    != Some("subagent")
            })
            .collect();
        let tools = Value::Array(tool_list);

        let mut messages: Vec<Value> = vec![
            json!({ "role": "system", "content": def.prompt }),
            json!({ "role": "user", "content": task }),
        ];

        let mut result = String::new();
        // 与主循环一致：子代理也没有迭代上限，直到模型不再调用工具。
        loop {
            let turn = llm::chat(&sub_cfg, &messages, &tools, |_| {})?;
            result = turn.content.clone();
            messages.push(assistant_message(&turn));
            if turn.tool_calls.is_empty() {
                break;
            }
            // Execute sub-agent tool calls sequentially (sub-agents are bounded).
            for call in &turn.tool_calls {
                let out = self.execute(call);
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": out,
                }));
            }
        }
        Ok(result)
    }

    /// Trim history when it grows beyond the budget. Instead of silently dropping
    /// old messages, we ask the model to summarize them and replace the dropped
    /// region with a single system message holding that summary. If the summary
    /// call fails we fall back to a plain truncation notice.
    fn compact(&mut self) {
        self.compact_with(false);
    }

    /// Force context compaction regardless of the size budget (used by the
    /// REPL `/compact` command). Returns the number of messages summarized away.
    pub fn compact_now(&mut self) -> usize {
        self.compact_with(true)
    }

    fn compact_with(&mut self, force: bool) -> usize {
        let size: usize = self.messages.iter().map(|m| m.to_string().len()).sum();
        if !force && (size <= CONTEXT_CHAR_BUDGET || self.messages.len() <= KEEP_RECENT + 1) {
            return 0;
        }

        let mut start = self.messages.len().saturating_sub(KEEP_RECENT);
        while start < self.messages.len() && self.messages[start]["role"] == "tool" {
            start += 1;
        }
        if start <= 1 {
            return 0;
        }

        let dropped: Vec<Value> = self.messages.drain(1..start).collect();
        let note = match self.summarize_messages(&dropped) {
            Some(summary) => format!("[Earlier conversation summary]\n{summary}"),
            None => "[Earlier conversation was omitted to keep the context small]".to_string(),
        };
        self.messages
            .insert(1, json!({ "role": "system", "content": note }));

        self.style.note(&format!(
            "context compacted: {} older message(s) replaced by a summary",
            dropped.len()
        ));
        dropped.len()
    }

    /// Produce a short LLM summary of a slice of messages. Returns None on any
    /// failure so the caller can fall back to plain truncation.
    fn summarize_messages(&self, messages: &[Value]) -> Option<String> {
        if messages.is_empty() {
            return None;
        }
        let mut excerpt = String::new();
        for m in messages {
            let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("?");
            let content = match m.get("content") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            let content = content.trim();
            if content.is_empty() {
                continue;
            }
            excerpt.push_str(&format!("{role}: {}\n", content));
        }
        if excerpt.trim().is_empty() {
            return None;
        }
        match llm::summarize(&self.cfg, &excerpt) {
            Ok(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    }
}

fn assistant_message(turn: &AssistantTurn) -> Value {
    let content = if turn.content.is_empty() {
        Value::Null
    } else {
        Value::String(turn.content.clone())
    };
    if turn.tool_calls.is_empty() {
        return json!({ "role": "assistant", "content": content });
    }
    let calls: Vec<Value> = turn
        .tool_calls
        .iter()
        .map(|c| {
            let args = if c.arguments.trim().is_empty() {
                "{}"
            } else {
                &c.arguments
            };
            json!({
                "id": c.id,
                "type": "function",
                "function": { "name": c.name, "arguments": args }
            })
        })
        .collect();
    json!({ "role": "assistant", "content": content, "tool_calls": calls })
}

/// One-line description of a tool call for the terminal banner.
fn summarize(name: &str, args: &str) -> String {
    let v: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    if name == "todo_write" {
        if let Some(arr) = v.get("todos").and_then(|x| x.as_array()) {
            return format!("{} todos", arr.len());
        }
    }
    let key = match name {
        "run_command" => "command",
        "edit_file" | "write_file" | "read_file" | "apply_patch" => "path",
        "list_dir" => "path",
        "mcp_read_resource" => "uri",
        "mcp_get_prompt" => "name",
        "web_fetch" => "url",
        "web_search" => "query",
        "git_commit" => "message",
        "git_status" | "git_diff" | "git_log" => "path",
        "subagent" => "task",
        _ => "pattern",
    };
    // Paths and URLs are fed back to the results panel as file locations, so a
    // 100-byte cap would make long paths unresolvable (the "…" is sent back to
    // the preview/diff commands). Keep commands short, but leave paths intact.
    let limit = match name {
        "read_file" | "write_file" | "edit_file" | "apply_patch" | "list_dir" | "git_status"
        | "git_diff" | "git_log" | "web_fetch" => 512,
        _ => 100,
    };
    match v.get(key).and_then(|x| x.as_str()) {
        Some(s) => {
            let s = s.replace('\n', " ");
            let mut end = s.len().min(limit);
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            if s.len() > end {
                format!("{}…", &s[..end])
            } else {
                s
            }
        }
        None => String::new(),
    }
}

/// OpenAI specs for the four MCP resource/prompt auxiliary tools.
fn mcp_aux_tools() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "mcp_list_resources",
                "description": "List resources available from MCP servers.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "mcp_read_resource",
                "description": "Read the text content of an MCP resource by URI.",
                "parameters": {
                    "type": "object",
                    "properties": { "uri": { "type": "string", "description": "Resource URI from mcp_list_resources." } },
                    "required": ["uri"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "mcp_list_prompts",
                "description": "List prompts available from MCP servers.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "mcp_get_prompt",
                "description": "Render an MCP prompt by name, with optional arguments.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Prompt name from mcp_list_prompts." },
                        "arguments": { "type": "object", "description": "Optional prompt arguments." }
                    },
                    "required": ["name"]
                }
            }
        }),
    ]
}

/// Tool spec for loading a skill's full instructions on demand.
fn skill_tool_spec() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "skill",
            "description": "Load the full instructions for a named skill listed in the system prompt. Call this before starting work that a skill covers.",
            "parameters": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Skill name from the available-skills list." }
                },
                "required": ["name"]
            }
        }
    })
}

/// Tool spec for inspecting the sub-agent job board.
fn agent_list_tool_spec() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "agent_list",
            "description": "List the sub-agent delegations started in this session (id, status, name, task, result preview).",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }
    })
}

/// Tool spec for the sub-agent delegation tool. The `name` must match a
/// sub-agent defined in the config; `task` is the work to hand off.
fn subagent_tool_spec() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "subagent",
            "description": "Delegate a sub-task to a specialized sub-agent. The sub-agent runs independently with its own instructions and returns a final result. Use this for review, testing, research, or any task that benefits from a focused expert.",
            "parameters": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the sub-agent to invoke." },
                    "task": { "type": "string", "description": "The task description to give the sub-agent." }
                },
                "required": ["name", "task"]
            }
        }
    })
}

fn render_resources(r: Result<Vec<Value>, String>) -> String {
    match r {
        Ok(list) if list.is_empty() => "(no resources)".to_string(),
        Ok(list) => {
            let mut out = String::new();
            for item in list {
                let uri = item
                    .get("uri")
                    .and_then(|u| u.as_str())
                    .unwrap_or("(no uri)");
                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let desc = item
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                out.push_str(&format!("{uri}  {name}  {desc}\n"));
            }
            out.trim_end().to_string()
        }
        Err(e) => format!("ERROR: {e}"),
    }
}

fn render_prompts(r: Result<Vec<Value>, String>) -> String {
    match r {
        Ok(list) if list.is_empty() => "(no prompts)".to_string(),
        Ok(list) => {
            let mut out = String::new();
            for item in list {
                let name = item
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("(no name)");
                let desc = item
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                out.push_str(&format!("{name}  {desc}\n"));
            }
            out.trim_end().to_string()
        }
        Err(e) => format!("ERROR: {e}"),
    }
}

fn result_string(r: Result<String, String>) -> String {
    match r {
        Ok(s) => s,
        Err(e) => format!("ERROR: {e}"),
    }
}

/// The initial conversation: just the system prompt. If a `NANOHERMES.md` file
/// exists in the workspace root, its contents are appended as project context.
fn default_messages(cfg: &Config, skills: &[crate::skills::Skill]) -> Vec<Value> {
    vec![json!({
        "role": "system",
        "content": system_content(cfg, skills)
    })]
}

/// Build the system message text: the mode-aware built-in prompt, plus
/// `NANOHERMES.md` project context and the skill listing when they exist.
fn system_content(cfg: &Config, skills: &[crate::skills::Skill]) -> String {
    let mut system = prompt::system_prompt(cfg);
    let project_ctx = cfg.workspace.join("NANOHERMES.md");
    if let Ok(text) = std::fs::read_to_string(&project_ctx) {
        if !text.trim().is_empty() {
            system.push_str("\n\n=== Project context (NANOHERMES.md) ===\n");
            system.push_str(text.trim());
            system.push('\n');
        }
    }
    if !skills.is_empty() {
        system.push('\n');
        system.push_str(&crate::skills::listing(skills));
    }
    system
}

/// Replace the leading system message of a loaded conversation with a freshly
/// built one, so a reloaded session reflects the current configuration. Messages
/// without a leading system message get one inserted at the front.
fn refresh_system(messages: &mut Vec<Value>, cfg: &Config, skills: &[crate::skills::Skill]) {
    let system = json!({
        "role": "system",
        "content": system_content(cfg, skills)
    });
    let is_system = messages
        .first()
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        == Some("system");
    if is_system {
        messages[0] = system;
    } else {
        messages.insert(0, system);
    }
}

/// Load a previously persisted conversation from a JSON file. The file is
/// expected to contain a JSON array of OpenAI-style messages.
pub fn load_session_file(path: &std::path::Path) -> Result<Vec<Value>, String> {
    let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&data).map_err(|e| e.to_string())?;
    let arr = v
        .as_array()
        .ok_or_else(|| "session file does not contain a JSON array".to_string())?;
    if arr.is_empty() {
        return Err("session file is empty".to_string());
    }
    Ok(arr.clone())
}

/// Atomically-ish persist the conversation to a JSON file. We write to a temp
/// sibling and rename so a crash mid-write can't leave a truncated session.
pub fn save_session_file(path: &std::path::Path, messages: &[Value]) -> Result<(), String> {
    let body = serde_json::to_string_pretty(messages).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod content_tests {
    use super::*;

    #[test]
    fn text_only_stays_a_plain_string() {
        assert_eq!(user_content("hi", &[]), json!("hi"));
    }

    #[test]
    fn an_image_makes_a_multimodal_array() {
        let imgs = vec!["data:image/png;base64,AAA".to_string()];
        let content = user_content("看图", &imgs);
        let parts = content.as_array().expect("content array");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "看图");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,AAA");
    }

    #[test]
    fn non_image_urls_are_dropped_and_the_count_is_capped() {
        // A page URL and an HTML data URL must not be forwarded as image parts.
        let mut imgs: Vec<String> = (0..MAX_ATTACHED_IMAGES + 3)
            .map(|i| format!("data:image/png;base64,{i}"))
            .collect();
        imgs.push("https://example.com/a.png".to_string());
        imgs.push("data:text/html;base64,AA".to_string());
        imgs.push("javascript:alert(1)".to_string());

        let content = user_content("x", &imgs);
        let parts = content.as_array().expect("content array");
        assert_eq!(parts.len(), 1 + MAX_ATTACHED_IMAGES);
        assert!(parts[1..].iter().all(|p| p["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/")));
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;

    #[test]
    fn file_tools_report_their_path() {
        // 结果区靠这条摘要里的路径把文件归到「已修改 / 已读取」，摘要为空会让
        // 整个改动在产物列表里消失（apply_patch 就曾如此）。
        let args = r#"{"path":"src/main.rs","diff":"@@ -1 +1 @@"}"#;
        for name in ["read_file", "write_file", "edit_file", "apply_patch"] {
            assert_eq!(
                summarize(name, args),
                "src/main.rs",
                "{name} 应给出文件路径"
            );
        }
        assert_eq!(
            summarize("run_command", r#"{"command":"ls -la"}"#),
            "ls -la"
        );
        // 参数不是 JSON 时给出空摘要，而不是一段误导性的文本。
        assert_eq!(summarize("apply_patch", "not json"), "");
    }

    #[test]
    fn long_paths_are_preserved_for_the_results_panel() {
        // >100 bytes: truncating this used to append "…" to the path that the
        // results panel later sends back to preview/diff.
        let long = format!("src/{}/file.rs", "深".repeat(60));
        let args = serde_json::json!({ "path": long }).to_string();
        assert_eq!(summarize("write_file", &args), long);

        // Commands still get a short banner.
        let cmd = format!("echo {}", "x".repeat(200));
        let args = serde_json::json!({ "command": cmd }).to_string();
        assert!(summarize("run_command", &args).ends_with('…'));
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("nanohermes-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");

        let messages = vec![
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "user", "content": "hello" }),
            json!({
                "role": "assistant",
                "content": "hi",
                "tool_calls": [{ "id": "1", "type": "function", "function": { "name": "echo", "arguments": "{}" } }]
            }),
        ];

        save_session_file(&path, &messages).expect("save");
        let loaded = load_session_file(&path).expect("load");
        assert_eq!(loaded.len(), messages.len());
        assert_eq!(loaded[0]["role"], "system");
        assert_eq!(loaded[2]["tool_calls"][0]["function"]["name"], "echo");

        // No leftover .tmp file after a successful save.
        assert!(!path.with_extension("tmp").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_or_empty_errors() {
        let missing =
            std::env::temp_dir().join(format!("nanohermes-missing-{}.json", std::process::id()));
        assert!(load_session_file(&missing).is_err());
    }

    #[test]
    fn a_reloaded_session_picks_up_the_current_system_prompt() {
        let dir = std::env::temp_dir().join(format!("nanohermes-refresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        save_session_file(
            &path,
            &[
                json!({ "role": "system", "content": "旧的系统提示词" }),
                json!({ "role": "user", "content": "hello" }),
            ],
        )
        .unwrap();

        let cfg = Config {
            workspace: dir.clone(),
            session: Some(path),
            // 切换人设后重开会话：系统消息必须换成新的，而不是沿用旧文件里的那段。
            system_prompt: Some("回答一律用中文".to_string()),
            // 测试不碰真实记忆库（~/.nanohermes）。
            memory: false,
            ..Default::default()
        };
        let agent = Agent::with_mcp(cfg, None);
        let sys = agent.messages[0]["content"].as_str().unwrap();
        assert!(
            sys.starts_with("You are NanoHermes,"),
            "基底提示词仍要在最前"
        );
        assert!(sys.contains("回答一律用中文"), "新的人设要生效");
        assert!(!sys.contains("旧的系统提示词"), "旧提示词不能残留");
        assert_eq!(agent.messages[1]["content"], "hello", "对话历史必须保留");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Thread-safety tests for the parallel tool-call path. `run()` spawns one
/// scoped thread per `tool_call`, all sharing `&Agent` and calling `execute()`.
/// These tests exercise that exact path directly, with and without MCP, so a
/// data race in built-in dispatch or MCP routing shows up here rather than in
/// production.
#[cfg(test)]
mod concurrency_tests {
    use super::*;
    use crate::llm::ToolCall;

    fn mock_config() -> std::path::PathBuf {
        crate::mcp::mock_server_config()
    }

    fn agent(mcp: Option<McpManager>) -> Agent {
        let cfg = Config {
            workspace: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            auto_approve: true,
            // 测试不碰真实记忆库（~/.nanohermes）。
            memory: false,
            ..Default::default()
        };
        Agent::with_mcp(cfg, mcp)
    }

    fn call(name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: "t".to_string(),
            name: name.to_string(),
            arguments: args.to_string(),
        }
    }

    /// Built-in tools only (no MCP): verify `tools::dispatch` and the per-tool
    /// implementations have no shared mutable state that breaks under concurrency.
    #[test]
    fn concurrent_execute_builtin_tools() {
        let agent = agent(None);
        let agent = &agent;

        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for _ in 0..10 {
                handles.push(s.spawn(move || {
                    let out = agent.execute(&call("list_dir", "{}"));
                    assert!(
                        out.contains("src") || out.contains("Cargo.toml"),
                        "list_dir: {out}"
                    );
                }));
                handles.push(s.spawn(move || {
                    let out = agent.execute(&call("glob", r#"{"pattern":"src/**/*.rs"}"#));
                    assert!(out.contains("agent.rs"), "glob: {out}");
                }));
                handles.push(s.spawn(move || {
                    let out = agent.execute(&call("read_file", r#"{"path":"Cargo.toml"}"#));
                    assert!(out.contains("name = \"agent\""), "read_file: {out}");
                }));
            }
            for h in handles {
                h.join().expect("builtin tool thread panicked");
            }
        });
    }

    /// Realistic parallel-turn scenario: an Agent with MCP, where several
    /// concurrent `execute()` calls mix built-in tools, auto-dispatched MCP
    /// tools, and the MCP resource/prompt aux tools. This is exactly what
    /// `run()` does when the model returns multiple tool calls at once.
    #[test]
    fn concurrent_execute_mixed_mcp_and_builtin() {
        let mgr = McpManager::from_config(&mock_config()).expect("connect mock");
        let agent = agent(Some(mgr));
        let agent = &agent;

        std::thread::scope(|s| {
            let mut handles = Vec::new();

            // Auto-dispatched MCP tool (echo) from many threads at once.
            for i in 0..8 {
                handles.push(s.spawn(move || {
                    let text = format!("echo-{i}");
                    let args = serde_json::to_string(&json!({ "text": text })).unwrap();
                    let out = agent.execute(&call("echo", &args));
                    assert_eq!(out, text, "echo mismatch");
                }));
            }

            // MCP aux tools: resources + prompts.
            handles.push(s.spawn(move || {
                let out = agent.execute(&call("mcp_list_resources", "{}"));
                assert!(
                    out.contains("notes") && out.contains("config"),
                    "list_resources: {out}"
                );
            }));
            handles.push(s.spawn(move || {
                let out = agent.execute(&call(
                    "mcp_get_prompt",
                    r#"{"name":"greet","arguments":{"name":"Bot"}}"#,
                ));
                assert!(out.contains("Hello, Bot!"), "get_prompt: {out}");
            }));
            handles.push(s.spawn(move || {
                let out = agent.execute(&call("mcp_read_resource", r#"{"uri":"mock://config"}"#));
                assert!(out.contains("answer=42"), "read_resource: {out}");
            }));

            // Built-in tools interleaved with the MCP traffic above.
            handles.push(s.spawn(move || {
                let out = agent.execute(&call("list_dir", "{}"));
                assert!(out.contains("src"), "list_dir: {out}");
            }));
            handles.push(s.spawn(move || {
                let out = agent.execute(&call("glob", r#"{"pattern":"**/Cargo.toml"}"#));
                assert!(out.contains("Cargo.toml"), "glob: {out}");
            }));

            for h in handles {
                h.join().expect("mixed execute thread panicked");
            }
        });
    }
}
