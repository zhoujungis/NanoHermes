# NanoHermes

A lightweight, low-memory terminal coding agent written in Rust. NanoHermes connects to any OpenAI-compatible LLM endpoint and gives the model real tools — read/write/edit files, search the codebase, run shell commands, apply patches, and call external tools via the Model Context Protocol (MCP).

The design goal is a small binary (~2 MB) with minimal runtime memory: no async runtime, a blocking HTTP client, bounded tool output, automatic context compaction, and no heavy dependencies.

## Features

- **Desktop app** — a Tauri GUI with a four-pane shell (title bar, task list, conversation, results). Tasks run locally against a workspace folder or remotely against a daemon; each task keeps its own conversation, permission mode, model profile, persona and token counter, can be pinned, and can be searched in place with `Ctrl+F`.
- **Personas & prompt snippets** — a persona is a block of instructions appended after the built-in system prompt; save several and pick one per task in the composer. Snippets are reusable text you insert into the composer with one click.
- **Themes** — three palettes (Graphite, Midnight, Paper) plus follow-the-system, switchable from settings or `Ctrl+K`; every colour in the UI comes from CSS variables, so a theme is a single palette swap.
- **OpenAI-compatible API** — works with OpenAI, local models (Ollama, vLLM, llamafile), or any endpoint that speaks the chat-completions format.
- **Streaming** — assistant tokens are printed as they arrive.
- **Resilience** — transient API failures (HTTP 429/5xx) are retried with exponential backoff; an optional `--fallback-model` takes over when retries are exhausted. Once streamed text has reached the user the request is never retried, so an answer is never duplicated.
- **Built-in tools** — `read_file`, `write_file`, `edit_file`, `apply_patch`, `list_dir`, `grep`, `glob`, `run_command`, `web_fetch`, `web_search`, `todo_write`, `todo_read`, `git_status`, `git_diff`, `git_log`, `git_commit`, `task_output`, `task_list`, `task_kill`. Conditional tools: `skill` (when skills are discovered), `subagent` / `agent_list` (when sub-agents are configured), `memory_save` / `memory_search` (when long-term memory is enabled).
- **Planning** — `todo_write` / `todo_read` let the model lay out and track a task list across turns.
- **Git** — `git_status` / `git_diff` / `git_log` inspect the repo; `git_commit` stages (optionally) and commits, with user confirmation.
- **Token usage** — cumulative request/token counts, shown by `/cost` and included in `json` / `stream-json` output.
- **Web access** — `web_fetch` reads a URL as plain text (HTML is stripped to readable text); `web_search` returns top web results to follow up on.
- **MCP support** — connect to external MCP servers over stdio or streamable HTTP; their tools are auto-discovered and callable.
- **MCP resources & prompts** — browse and read MCP resources, list and render MCP prompts through dedicated agent tools.
- **Project guidance** — `AGENTS.md` files (global `~/.nanohermes/AGENTS.md` plus every ancestor of the workspace) are loaded into the system prompt as project rules (capped at 20 KB, deduplicated).
- **Long-term memory** — after a completed turn the agent distils lessons, preferences, corrections and facts into `~/.nanohermes/memory.json`; the next turn retrieves the relevant entries and injects them into the system prompt. Ranking is local BM25 over ASCII words plus CJK unigrams and bigrams, so it needs no extra service, and the model can also read and write memory with `memory_save` / `memory_search`.
- **Skills** — reusable instruction bundles discovered from `.nanohermes/skills/<name>/SKILL.md`. Only the name and description enter the system prompt; the model loads the full instructions on demand with the `skill` tool (progressive disclosure).
- **Lifecycle hooks** — `PreToolUse` / `PostToolUse` shell hooks defined in the config file. A `PreToolUse` hook can veto a tool call; a `PostToolUse` hook can only append a warning.
- **Sub-agents** — delegate focused sub-tasks to configured sub-agents that run in parallel (bounded concurrency), tracked on a job board and inspectable with `agent_list`.
- **Sandboxing & approvals** — a `read-only` / `workspace-write` filesystem sandbox enforced by every file-writing tool, plus a Linux Landlock sandbox for spawned commands, and a three-level approval policy (`never` / `on-request` / `untrusted`) with built-in detection of recognised read-only commands.
- **Parallel tool calls** — when the model returns multiple tool calls in one turn, they execute concurrently (at most 8 at a time).
- **Session persistence** — save and restore conversation history to a JSON file with `--session`, or resume the most recent session for the current directory with `-c` / pick one interactively with `-r`.
- **Multimodal input** — attach images with `--image` (one-shot) or `/image` (REPL) for vision-capable models.
- **Context compaction** — when the conversation grows beyond the budget, older messages are summarized (via the model) rather than silently dropped.
- **Interrupt & rewind** — Ctrl-C aborts the running turn without killing the process (the conversation is kept); `/undo [n]` rewinds one or more full user turns.
- **REPL** — interactive mode when stdin is a TTY.

## Build the CLI from source

Requires a recent stable Rust toolchain.

```bash
git clone <repo>
cd agent
cargo build --release
# binary: target/release/agent
```

The release profile uses `opt-level = "z"`, LTO, and `panic = "abort"` to keep the binary small.

## Repository layout

| Path | What it is |
|------|------------|
| `agent/` | The coding agent crate (`agent`): CLI binary, agent loop, tools, MCP client, daemon mode. |
| `gui/` | The desktop app crate (`gui`): the Tauri v2 Rust shell — `tauri.conf.json`, `src/`, `icons/`, `capabilities/`. |
| `ui/` | The desktop frontend (Vite + React + TypeScript): `index.html`, `src/`, `package.json`. |
| `scripts/` | Project-level deployment scripts (`deploy-debian.sh`). |
| `target/` | Cargo build output, shared by the whole workspace. |

The root `package.json` is the project-level npm entry point — `npm run dev` / `npm run build` delegate to `ui/`.

## Testing

```bash
npm install --prefix ui      # once: frontend dev dependencies
npm run test                 # frontend only (vitest + jsdom + testing-library)
npm run test:rust            # Rust only (cargo test --workspace)
npm run test:all             # both
```

- **Frontend** (`ui/src/**/*.test.{ts,tsx}`) — Vitest in a jsdom environment. Covers pure logic (theme resolution and persistence, store state transitions, Markdown sanitising, image attachment handling, conversation search, diff blocks) plus component tests for the settings page and the command palette, and an end-to-end suite (`ui/src/App.e2e.test.tsx`) that renders the real store and components against a mocked Tauri IPC bridge: virtual scrolling of long transcripts, find-in-conversation across off-screen messages, diff change jumps, and the running/stopping composer states. Config lives in the `test` block of `ui/vite.config.ts`; shared setup is `ui/src/test/setup.ts`.
- **Rust** — unit tests sit in each module (`#[cfg(test)]`), and `agent/tests/e2e_mock_llm.rs` is an end-to-end suite that drives a real `Agent` against a local mock OpenAI-compatible server (std-only, no network): tool-call round trips, streaming and non-streaming replies, retry on a transient 5xx, and hard API errors.

## Desktop app (Tauri GUI)

`gui/` wraps the agent in a desktop application: a four-pane shell that runs every task either in-process against a local workspace folder, or by forwarding it to a remote NanoHermes daemon.

```bash
npm --prefix ui install                  # frontend dependencies
npm run build                            # type-check + bundle ui/ → ui/dist
cd gui && npx --no-install tauri build   # release build + installers
# installers: target/release/bundle/nsis/NanoHermes_<version>_x64-setup.exe
#             target/release/bundle/msi/NanoHermes_<version>_x64_en-US.msi
```

`npm run dev` (from the repo root) starts the Vite dev server; `npx --no-install tauri dev` from `gui/` opens the app against it. The installers are unsigned, so Windows SmartScreen warns on first run.

### Window layout

| Pane | Contents |
|------|----------|
| Title bar | Custom-drawn instead of OS chrome: brand, a model/location chip (click to open settings), toggles for the task list and results panel, settings, window controls. |
| Task list (left, collapsible) | Every task with its execution location and cumulative token usage; search by title/content, pin (pinned tasks form their own group at the top), rename, delete, export. |
| Conversation (center) | A task bar above the transcript shows the task title, run state (`Ready` / `Running` / `Stopping`), cumulative tokens, export and clear-context actions. Below it, a streaming Markdown transcript — user turns, assistant replies, tool calls with expandable output, per-message retry. |
| Results (right, collapsible) | Workspace file tree and this task's artifacts (filtered to all / modified / read), a read-only file preview, and a diff of changed files with word-level highlighting and previous/next change jumps. |

### Tasks, workspaces and execution location

- Every task owns its conversation and its own folder. A local task binds to a workspace folder picked in the folder dialog (or typed directly); relative paths in every tool call resolve against it.
- Execution location is per task: `local` runs the agent inside the app, `cloud` forwards the turn to a remote daemon (see [HTTP daemon mode](#http-daemon-mode)) and tags it with `X-Session-Id`, so each task keeps its own conversation on the server.
- A local task can be stopped at the next step boundary; a cloud turn stops when the daemon emits its next stream event, because the daemon holds the whole request.
- Pinned tasks form their own group fixed at the top of the list and skip the Today/Yesterday/This-week buckets; toggle the pin from the task menu or `Ctrl+K`.

### Permission modes

| Value | UI label | Behaviour |
|-------|----------|-----------|
| `default` | Agent | All tools available; file edits and commands run without asking. |
| `accept-edits` | Accept | File edits auto-approved, commands gated. |
| `plan` | Plan | Read-only: no writes, no commands. |
| `goal` | Goal | Unattended: every tool runs until the task is finished. |
| `debug` | Debug | Reads files and runs commands to reproduce a problem; the file-editing tools are blocked, but shell commands can still modify the tree (combine with `--sandbox read-only` for a hard guarantee). |

The mode is stored in settings as the default for new tasks and can be switched from the composer or the command palette. This applies to local tasks only: a cloud turn uses the permission mode the daemon was started with (see [Deploying the daemon on a VPS](#deploying-the-daemon-on-a-vps)).

### Composer

- `Enter` sends, `Shift+Enter` inserts a newline.
- `↑` / `↓` walk the input history while the caret sits on the first / last line; the draft you were typing is restored when you come back to the newest entry.
- `@` opens the workspace file picker and inserts the chosen path at the caret.
- `+` opens the workspace file picker; the files you select are inlined into the prompt.
- While a turn is running the send button becomes **Stop**. A stop request takes effect at the next step boundary (for a cloud task, when the daemon emits its next stream event): the button spins and stays disabled, the task bar switches to `Stopping`, and when the turn winds down you get a "task stopped" notice and the caret returns to the input.
- Pressing `Enter` mid-turn queues the message instead of dropping it. The queue is sent automatically when the turn ends normally; after a stop it stays queued so you can review or send it yourself.
- The model pill left of the mode switch picks which model profile this task talks to. Each task remembers its own choice; the menu is fed by the list in **Settings → Models**.
- The persona pill next to it picks which persona (system-prompt add-on) this task uses; a persona with actual instructions is highlighted. The menu is fed by **Settings → Personas**.
- The bolt button opens the **prompt snippets** menu: clicking one appends its text to the current input. Snippets are maintained in **Settings → Prompt snippets**.
- Images: paste a screenshot, drop a file, or pick one. Up to 4 per turn; each is downscaled to 1024 px on its long edge, and anything that is not a decodable image or is larger than 12 MB is rejected with a reason. They travel as data URLs, so the model has to support vision.
- Models, personas and the tuning knobs under **Advanced** apply to local tasks only; a cloud turn's behaviour is decided by the remote daemon, so the composer hides those pills for cloud tasks.

### Results panel

- **Files** — the task folder as a tree; clicking a file opens a read-only preview (200 KB cap) with a fixed line-number gutter and horizontal scrolling limited to the code body.
- **Artifacts** — the files this task wrote or read, with all / modified / read filters.
- **Diff** — opening a file touched by `write_file` / `edit_file` / `apply_patch` shows its diff: line-level backgrounds, inline syntax colouring, word-level highlighting inside changed lines, and a `n / m` change counter with **previous / next change** buttons that scroll to the block and wrap around. Outside a Git repository the panel says so instead of erroring.

### Settings

`Ctrl+,` opens settings. Every field saves as you type (700 ms debounce, flushed on blur and on close); the settings header shows `Saving…` / `Saved automatically` / `Save failed`, and a **Save** button forces an immediate flush.

| Group | Fields |
|-------|--------|
| Theme | Graphite (dark, default), Midnight (dark), Paper (light), or follow the system; applies instantly and is remembered. |
| Models | A list of model profiles (name, API key, base URL, model name) — add as many as you like and switch per task from the composer. **Set default** marks the one new tasks start on; **Test** probes that endpoint's `/models` with the values on screen. |
| Personas | A list of personas (name + extra system-prompt instructions) — the extra text is appended after the built-in prompt, so the built-in rules and safety constraints stay in force. A blank persona adds nothing; **Current default** marks the one new tasks start on, and each task may pick its own. |
| Prompt snippets | Reusable phrases or templates; click the bolt in the composer to insert one. |
| Advanced | Tuning for local tasks: extended thinking, read project guide (AGENTS.md / NANOHERMES.md), long-term memory (on by default; turning it off stops new distillation but keeps existing entries), temperature, max iterations, max output tokens. |
| Location | Run tasks in-process or forward them to a remote daemon, plus the daemon base URL and Bearer token — **Check** probes `GET /health`. |
| Workspace | The default project folder; each task may override it. |
| Defaults | Execution location for new tasks, default permission mode. |

### Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+K` | Command palette — every action (new task, modes, panels, export, settings…). |
| `Ctrl+N` | New task; when the task list is visible it asks for local or cloud first. |
| `Ctrl+F` | Find in the conversation; `Enter` / `Shift+Enter` jump to the next / previous hit, `Esc` closes the bar. |
| `Ctrl+,` | Settings. |
| `Ctrl+B` / `Ctrl+J` | Toggle task list / results panel. |
| `Ctrl+/` | Shortcut cheat sheet. |
| `↑` / `↓` (composer) | Walk the input history from the first / last line. |
| `@` (composer) | Open the file picker and insert the selected path. |
| `Esc` | Close the topmost layer: palette → settings → menu → find bar → preview. |
| `Tab`, `←` / `→`, `Esc` (settings) | Move between fields, switch within an option group, save and close. |

### Local state

| What | Where |
|------|-------|
| GUI settings (model profiles, personas, snippets, advanced) | `~/.nanohermes/gui-settings.json` |
| Long-term memory | `~/.nanohermes/memory.json` |
| Task conversations | `~/.nanohermes/sessions/<task-id>.json` |
| Exported Markdown | `~/.nanohermes/exports/` |
| UI state (task list, drafts, panel widths) | `localStorage`, key `nanohermes.state.v1` |
| Theme | `localStorage`, key `nanohermes.theme.v1` |

Streaming text reaches the webview on the `nanohermes://delta|tool|note|usage` event channels and is batched every 40 ms. It is held in a dedicated subscription channel rather than the global store, so only the bubble being written re-renders; the transcript is finalised once per phase. While a turn is running the UI defers its disk writes to phase boundaries, so a long answer does not hammer `localStorage`.

## Quick start

```bash
# one-shot
agent "add unit tests for src/tools/search.rs"

# interactive REPL
agent

# prompt from stdin
echo "explain main.rs" | agent

# local model
agent --base-url http://localhost:11434/v1 -m llama3 "list files"
```

## Configuration

NanoHermes is configured through a config file, environment variables and CLI flags. Precedence: **CLI flags > config file > environment variables**.

| Variable | Equivalent flag | Default | Description |
|----------|----------------|---------|-------------|
| `NANOHERMES_API_KEY` / `OPENAI_API_KEY` | `-k, --api-key` | _(empty)_ | API key for the endpoint |
| `NANOHERMES_BASE_URL` / `OPENAI_BASE_URL` | `-u, --base-url` | `https://api.openai.com/v1` | OpenAI-compatible base URL |
| `NANOHERMES_MODEL` / `OPENAI_MODEL` | `-m, --model` | `gpt-4o-mini` | Model name |
| `NANOHERMES_CONFIG` | `--config` | `~/.nanohermes/config.json` | Config file to load when `--config` is absent |
| `NANOHERMES_DAEMON_TOKEN` | `--daemon-token` | _(empty)_ | Bearer token for daemon requests |
| `NANOHERMES_MAX_RETRIES` | `--max-retries` | `2` | Transient-failure retries |
| `NANOHERMES_RETRY_BASE_MS` | `--retry-base-ms` | `500` | Base backoff between retries |

### Config file

Besides environment variables and flags, NanoHermes reads a JSON config file. `~/.nanohermes/config.json` is loaded automatically when it exists; `--config <FILE>` (or `NANOHERMES_CONFIG`) loads a different file. Environment variables are read first, then the file, then CLI flags — so the file overrides env vars and flags override the file. `--print-config` prints the resolved result and exits.

```json
{
  "base_url": "https://openrouter.ai/api/v1",
  "model": "anthropic/claude-sonnet-4",
  "permission_mode": "accept-edits",
  "approval_policy": "on-request",
  "sandbox": "workspace-write",
  "thinking": true,
  "project_docs": true,
  "memory": true,
  "allowed_tools": ["read_file", "grep", "edit_file"],
  "hooks": [
    { "event": "PreToolUse", "matcher": "run_command", "command": "./scripts/check.sh" }
  ]
}
```

Supported keys: `api_key`, `base_url`, `model`, `temperature`, `max_tokens`, `stream`, `max_iterations`, `cwd`, `auto_approve`, `approval_policy`, `sandbox`, `timeout_secs`, `max_retries`, `retry_base_ms`, `mcp_config`, `session`, `permission_mode`, `allowed_tools`, `disallowed_tools`, `fallback_model`, `output_format`, `input_format`, `continue_session`, `resume_session`, `thinking`, `project_docs`, `memory`, `memory_dir`, `system_prompt`, `daemon`, `daemon_port`, `daemon_bind`, `daemon_token`, `subagents_config`, and `hooks`.

### CLI flags

```
agent [OPTIONS] [PROMPT...]      one-shot: run PROMPT and exit
agent [OPTIONS]                  interactive REPL (when stdin is a TTY)
echo "PROMPT" | agent [OPTIONS]  one-shot with the prompt on stdin
```

| Flag | Description |
|------|-------------|
| `-m, --model <NAME>` | Model name |
| `--fallback-model <NAME>` | Model to retry with when the primary model keeps failing (retries exhausted) |
| `-u, --base-url <URL>` | OpenAI-compatible base URL (without `/chat/completions`) |
| `-k, --api-key <KEY>` | API key |
| `-C, --cwd <DIR>` | Workspace root (default: current directory) |
| `-i, --image <PATH>` | Attach an image (repeatable); requires a vision-capable model |
| `--mcp-config <FILE>` | Path to an MCP servers config JSON (enables MCP tools) |
| `--subagents <FILE>` | Path to a sub-agents config JSON (enables the `subagent` / `agent_list` tools) |
| `--session <FILE>` | Persist conversation history to FILE (loaded on startup if present) |
| `-c, --continue` | Resume the most recent session recorded for this workspace |
| `-r, --resume` | List saved sessions and pick one to resume |
| `-n, --max-iterations <N>` | Max model↔tool round trips per turn (default 40) |
| `--max-tokens <N>` | Max response tokens (default 4096) |
| `--temperature <F>` | Sampling temperature (default 0.0) |
| `--timeout <SECS>` | HTTP request timeout (default 120) |
| `--max-retries <N>` | Retries for transient API failures (default 2, env `NANOHERMES_MAX_RETRIES`) |
| `--retry-base-ms <MS>` | Base backoff delay between retries (default 500, env `NANOHERMES_RETRY_BASE_MS`) |
| `--no-stream` | Disable streaming (buffered response) |
| `--output-format <FMT>` | `text` (default), `json` (one JSON object on stdout) or `stream-json` (newline-delimited events) |
| `--input-format <FMT>` | `text` (default) or `stream-json` (parse stdin as JSON lines) |
| `--thinking` | Ask the endpoint for extended thinking (`reasoning_effort: high`) |
| `--ask` | Confirm every shell command before running (`untrusted` policy) |
| `--approval-policy <POLICY>` | When shell commands are confirmed: `never` \| `on-request` \| `untrusted` (default `never`, i.e. auto-approve) |
| `-y, --yes` | Auto-approve shell commands (`never` policy; default) |
| `--sandbox <MODE>` | Filesystem sandbox: `off` (default) \| `read-only` \| `workspace-write`. File tools enforce it everywhere; spawned commands use Landlock on Linux |
| `--permission-mode <MODE>` | `default` \| `accept-edits` \| `plan` \| `goal` \| `debug` (see [Permission modes](#permission-modes)) |
| `--allowed-tools <LIST>` | Comma-separated allow-list of tool names |
| `--disallowed-tools <LIST>` | Comma-separated deny-list of tool names; `*` denies every tool, including MCP and memory tools |
| `--mode <MODE>` | Preset bundle: `minimal` (no tools) \| `create` (file tools only, no shell) |
| `--config <FILE>` | Load settings from a JSON config file instead of `~/.nanohermes/config.json` |
| `--print-config` | Print the resolved configuration as JSON and exit |
| `--no-project-docs` | Do not load `AGENTS.md` into the system prompt |
| `--daemon` | Run as an HTTP daemon: `POST {"prompt":"..."}` → `{"response":"..."}` |
| `--daemon-port <PORT>` | Daemon listen port (default 8787) |
| `--daemon-bind <ADDR>` | Daemon bind address (default `127.0.0.1`) |
| `--daemon-token <TOKEN>` | Bearer token required for daemon requests (or env `NANOHERMES_DAEMON_TOKEN`) |
| `-h, --help` | Show help |
| `-V, --version` | Show version |

> HTTP requests honour standard proxy environment variables (`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY`). `localhost` and `NO_PROXY` hosts bypass the proxy automatically.

## REPL commands

When running interactively:

| Command | Action |
|---------|--------|
| `/clear` | Reset the conversation (keeps the system prompt) |
| `/undo [n]` | Rewind the last `n` full user turns (default 1) |
| `/model <name>` | Switch the active model |
| `/mode <mode>` | Switch permission mode (`default`, `accept-edits`, `plan`, `goal`, `debug`) |
| `/format <fmt>` | Switch output format (`text`, `json`, `stream-json`) |
| `/tools` | List available tools |
| `/status` | Show session stats |
| `/cost` | Show cumulative token usage |
| `/compact` | Summarize older messages to free context |
| `/image <path>` | Queue an image for the next task (vision models only) |
| `/images` | List queued images |
| `/drop-images` | Clear queued images |
| `/save <file>`, `/load <file>` | Save / load the conversation |
| `/help` | Show commands |
| `/exit`, `/quit`, `:q` | Quit |

Anything else is sent to the agent as a task. Prefix a line with `!` to run a shell command directly.

## Built-in tools

All paths are resolved relative to the workspace root unless absolute. When a sandbox is configured, the file-writing tools (`write_file`, `edit_file`, `apply_patch`) enforce it: `read-only` rejects writes and `workspace-write` rejects paths outside the workspace.

### `read_file`
Read a text file with line numbers.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `path` | string | _(required)_ | File path |
| `offset` | integer | 1 | 1-based first line to read |
| `limit` | integer | 2000 | Maximum number of lines |

Files larger than 4 MB are rejected; read them in chunks with `offset`/`limit`.

### `write_file`
Create or overwrite a file. Parent directories are created as needed.

| Parameter | Type | Description |
|-----------|------|-------------|
| `path` | string | File path |
| `content` | string | Full file content |

### `edit_file`
Replace an exact string in a file. `old_string` must match byte-for-byte, including indentation.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `path` | string | _(required)_ | File path |
| `old_string` | string | _(required)_ | Exact text to find |
| `new_string` | string | _(required)_ | Replacement text |
| `replace_all` | boolean | false | Replace every occurrence |

If `old_string` appears more than once and `replace_all` is false, the call fails — provide more context or set `replace_all=true`.

### `apply_patch`
Apply a unified diff (patch) to a file. The diff is matched by content (not line numbers), so it stays robust against small drift.

| Parameter | Type | Description |
|-----------|------|-------------|
| `path` | string | File path to patch |
| `diff` | string | Unified diff text with `@@` hunks |

### `list_dir`
List directory entries (directories suffixed with `/`, files shown with size).

| Parameter | Type | Default |
|-----------|------|---------|
| `path` | string | `.` (workspace root) |

### `grep`
Search file contents for a literal substring, recursively. Binary files and build/tool directories (`.git`, `target`, `node_modules`, etc.) are skipped.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `pattern` | string | _(required)_ | Literal text to search for |
| `path` | string | workspace root | File or directory to search |
| `include` | string | _(none)_ | Glob filter, e.g. `*.rs` or `src/**/*.ts` |
| `case_insensitive` | boolean | false | Case-insensitive match |

Results are capped at 200 matches.

### `glob`
Find files by glob pattern relative to the workspace root. Supports `*`, `?`, and `**`. Matching is per character, so `?` matches one CJK character; pathological patterns (many nested `**`) are bounded by a work budget instead of hanging.

| Parameter | Type | Description |
|-----------|------|-------------|
| `pattern` | string | Glob pattern, e.g. `src/**/*.rs` |

Results are capped at 500 files.

### `run_command`
Run a shell command via `sh -c` in the workspace.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `command` | string | _(required)_ | Shell command to execute |
| `cwd` | string | workspace root | Working directory |
| `timeout_secs` | integer | 120 | Kill the command after this many seconds (max 3600) |

Stdout and stderr are captured (capped at 256 KB each) and reported with the exit code. Use non-interactive flags since the terminal has no TTY. Set `background=true` to run long jobs in the background and poll them with `task_output`.

### `web_fetch`
Fetch a URL over HTTP(S) and return its content as readable plain text. HTML tags, `<script>`/`<style>` bodies and entities are removed; whitespace is collapsed. Requests honour the proxy environment variables.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `url` | string | _(required)_ | `http://` or `https://` URL |
| `max_chars` | integer | 20000 | Maximum characters of text to return (500–200000) |

Response bodies are capped at 2 MB; anything larger is truncated. Loopback, private, link-local and cloud-metadata addresses are refused, and every redirect hop is re-checked, so a prompt-injected page cannot turn the tool into an SSRF probe of the host or the local network.

### `web_search`
Search the web (DuckDuckGo HTML endpoint) and return the top results as title / URL / snippet. Follow up with `web_fetch` on a result URL to read the page.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `query` | string | _(required)_ | Search query |
| `count` | integer | 5 | Number of results (1–10) |

### `todo_write` / `todo_read`
Maintain an ordered task list for the current session. `todo_write` replaces the whole list and reports it as a status note (stderr in the terminal, a `Note` event in the GUI) so the model can lay out a plan and update progress without polluting `json` output; `todo_read` returns the current list.

| Parameter | Type | Description |
|-----------|------|-------------|
| `todos` | array of `{content, status}` | The full list, in order (`status` is `pending`, `in_progress` or `completed`; defaults to `pending`) |

Up to 50 items are kept; empty `content` is rejected.

### `git_status` / `git_diff` / `git_log`
Read-only git inspection in the workspace root (uses the `git` binary).

| Tool | Parameters | Description |
|------|------------|-------------|
| `git_status` | `path` (optional) | Short working-tree status plus the current branch |
| `git_diff` | `staged` (bool, optional), `path` (optional) | Unified diff; `staged=true` diffs the index |
| `git_log` | `count` (int, default 10, max 100), `path` (optional) | Recent commits, one line each |

### `git_commit`
Stage (optionally) and create a commit. Asks the user to confirm unless auto-approval is on, and is blocked in `plan` permission mode.

| Parameter | Type | Description |
|-----------|------|-------------|
| `message` | string | Commit message (required) |
| `all` | boolean | Stage all changes before committing (default false) |
| `files` | array of string | Paths to stage before committing (optional) |

### `task_output` / `task_list` / `task_kill`
Retrieve output from, list, or kill background tasks started by `run_command` with `background=true`.

### `memory_save` / `memory_search`
Available when long-term memory is enabled (`"memory": true`, the default).

| Tool | Parameters | Description |
|------|------------|-------------|
| `memory_save` | `kind` (`lesson` / `preference` / `correction` / `fact`), `text` | Store one durable note; an entry with the same text is skipped as a duplicate |
| `memory_search` | `query`, `top_k` (default 5, 1–20) | Search the memory file and return scored entries |

Entries live in `~/.nanohermes/memory.json` (override with `memory_dir` in the config file). Retrieval scores candidates with BM25 over lower-cased ASCII words and bigrams of CJK runs, so it stays fully local and needs no embedding service.

## MCP (Model Context Protocol)

NanoHermes includes a minimal MCP client supporting two transports. Configure servers with a JSON file:

```json
{
  "servers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"]
    },
    "custom": {
      "command": "python3",
      "args": ["my_mcp_server.py"]
    },
    "remote": {
      "url": "https://host.example/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

```bash
agent --mcp-config mcp.json "list the files the MCP server exposes"
```

A server entry with a `command` (+ optional `args`) is spoken to as a child process over stdio using JSON-RPC 2.0 with LSP-style `Content-Length` framing. An entry with a `url` uses the **streamable-HTTP** transport: each request is a synchronous POST with an optional `headers` map, and the returned session id is reused on subsequent calls. Tools returned by `tools/list` are merged into the agent's tool list and auto-dispatched by name.

### MCP resources & prompts

In addition to tools, NanoHermes exposes four auxiliary agent tools for MCP resources and prompts:

| Tool | Description |
|------|-------------|
| `mcp_list_resources` | List resources available from all MCP servers |
| `mcp_read_resource` | Read the text content of an MCP resource by URI |
| `mcp_list_prompts` | List prompts available from all MCP servers |
| `mcp_get_prompt` | Render an MCP prompt by name, with optional arguments |

These are only available when at least one MCP server is configured.

## Parallel tool calls

When the model returns multiple `tool_calls` in a single assistant message, NanoHermes executes them **concurrently** using scoped threads, in batches of at most 8. Results are collected in call order and appended together, so each `tool_call_id` is paired with the correct result regardless of which call finished first.

- Built-in tools run in parallel (they share no mutable state).
- MCP tool calls serialize per-server (on the server's stdin lock) but run in parallel across different servers.
- Calls beyond the first 8 run in the next batch, so one turn cannot spawn hundreds of OS threads and shell processes at once.
- A single tool call takes the direct path with no thread overhead.

## Project guidance (AGENTS.md)

If `project_docs` is enabled (the default), NanoHermes reads `AGENTS.md` files and injects them into the system prompt as project rules. Sources, in order:

1. Global guidance: `~/.nanohermes/AGENTS.md`.
2. Every directory from the filesystem root down to the workspace root, outermost first, so more specific guidance appears later.

Content is capped at 20 KB and repeated files are deduplicated. Disable it with `--no-project-docs` or `"project_docs": false` in the config file.

## Skills

A skill is a directory containing a `SKILL.md` with optional YAML frontmatter:

```markdown
---
name: release
description: Cut a release: bump the version, update the changelog, tag, and push.
---

Step 1: run `cargo test` ...
```

Skills are discovered from `.nanohermes/skills/<name>/SKILL.md` in the workspace and from `~/.nanohermes/skills/<name>/SKILL.md`; a workspace skill shadows a home skill with the same name. When at least one skill is found, NanoHermes advertises the `skill` tool and lists each skill's name and description in the system prompt. The model calls `skill` with a name to load that skill's full instructions (capped at 20 KB) only when it needs them — progressive disclosure keeps the base prompt small. When the frontmatter is absent, the directory name is used as the skill name and the first body line as the description.

## Lifecycle hooks

Hooks are shell commands run around every tool call. Define them in the config file:

```json
{
  "hooks": [
    { "event": "PreToolUse",  "matcher": "run_command", "command": "echo checking >&2" },
    { "event": "PostToolUse", "matcher": "*",           "command": "./scripts/audit.sh" }
  ]
}
```

- `event` is `PreToolUse` or `PostToolUse`.
- `matcher` is `*` / empty (all tools) or a comma-separated list of tool names.
- The hook receives the tool name and JSON arguments on stdin and runs in the workspace root.

A `PreToolUse` hook that exits non-zero **blocks** the tool call and returns its output to the model. A `PostToolUse` hook that exits non-zero only appends a warning to the tool result. Hooks are ignored if the config file cannot define them.

## Sub-agents

Define specialized sub-agents in a JSON file and pass it with `--subagents`:

```json
{
  "subagents": {
    "reviewer": {
      "prompt": "You are a meticulous code reviewer. Report concrete issues only.",
      "allowed_tools": ["read_file", "grep", "list_dir"]
    },
    "tester": {
      "prompt": "You write and run focused tests.",
      "model": "gpt-4o-mini"
    }
  }
}
```

Once configured, two tools become available:

| Tool | Description |
|------|-------------|
| `subagent` | Delegate a named sub-agent a `task`; it runs independently and returns a final result |
| `agent_list` | Inspect the session's sub-agent job board (id, status, name, task, duration, result preview) |

Sub-agents run in parallel, bounded by a concurrency gate (4 at a time). Each job is recorded on the board at start, so `agent_list` reports live status. A sub-agent may override the model and restrict its tool set with `allowed_tools`.

## Sandboxing & approval policies

Two independent settings control what spawned shell commands may do.

**Approval policy** decides when the user is asked to confirm a command:

| Policy | Behaviour |
|--------|-----------|
| `never` | Run every command without asking (implies auto-approve; the default via `-y`) |
| `on-request` | Ask only for commands that are **not** recognised read-only commands |
| `untrusted` | Ask for every command, even read-only ones |

Read-only detection recognises a small set of safe programs (`ls`, `cat`, `grep`, `git status`/`diff`/`log`, …). Any shell metacharacter (`|`, `&`, `;`, redirection) or command substitution makes a command untrusted regardless of the leading program.

**Sandbox mode** restricts the filesystem view of spawned commands (Linux only, via **Landlock**):

| Mode | Behaviour |
|------|-----------|
| `off` | No restrictions (default) |
| `read-only` | Read + execute anywhere; no writes at all |
| `workspace-write` | Read + execute anywhere; writes allowed only under the workspace |

The file-writing tools (`write_file`, `edit_file`, `apply_patch`) honour the same mode on every platform: `read-only` rejects writes and `workspace-write` rejects paths that resolve outside the workspace, so the agent cannot bypass the sandbox by editing a file instead of running a command.

The sandbox is installed in the child process after `fork` and before `exec`, so it also covers everything the command spawns. If the kernel does not expose Landlock, NanoHermes **refuses** to run the command rather than silently downgrading. It uses raw syscalls only — no external crates.

## Interrupt & rewind

Pressing Ctrl-C during a turn sets a flag instead of killing the process. The agent loop polls it at iteration boundaries, so the turn stops cleanly and control returns to the REPL with the conversation intact (no orphaned tool call). While idle, Ctrl-C still exits as usual. Nested turns (e.g. a sub-agent running inside a tool call) share one handler via reference counting.

Use `/undo [n]` to rewind: it removes the last `n` full user turns (each turn's user message plus everything after it) from the conversation history and persists the result. `/undo` alone rewinds one turn.

## HTTP daemon mode

Start NanoHermes as a long-running HTTP service instead of a one-shot CLI or REPL:

```bash
agent --daemon --daemon-port 8787 \
       --daemon-token "$(openssl rand -hex 16)" \
       --session /data/session.json
```

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/` | `POST` | Bearer token (if set) | Body `{"prompt": "..."}` (optional `"stream": true`, `"images": ["data:image/...;base64,..."]`) → `{"response", "session", "usage"}`; with `stream: true` the reply is newline-delimited JSON |
| `/health` | `GET` | none | Returns `{"status":"ok","sessions":N}` for liveness probes |

- The conversation is **isolated per session**. Send an `X-Session-Id: <name>` header to pick a conversation; requests with different ids never share context. Omit the header to use the `default` session. Each session is persisted to its own JSON file under the `--session` directory.
- Session ids are used as file names, so they are validated (`[A-Za-z0-9._-]`, up to 64 characters, no leading dot or Windows device names). Request bodies are capped at 64 MB and the process keeps at most 256 distinct sessions, so a misbehaving client cannot exhaust memory or the disk.
- With `"stream": true` the daemon emits NDJSON events: `{"type":"delta","text":...}`, `{"type":"tool","name","summary","output"}`, `{"type":"note","text"}`, `{"type":"usage","requests","prompt_tokens","completion_tokens"}`, then `{"type":"done","response",...}` or `{"type":"error","error"}`. A `stream` turn can be stopped from the desktop; the daemon notices at the next event boundary.
- The daemon binds to `127.0.0.1` by default. Set `--daemon-bind 0.0.0.0` only behind a trusted reverse proxy / tunnel.
- **Always set a token** (`--daemon-token` or `NANOHERMES_DAEMON_TOKEN`) when the port is reachable from outside localhost. Requests without a valid `Authorization: Bearer <token>` header get `401`.
- For production, put it behind a reverse proxy (e.g. Cloudflare Tunnel, Caddy, nginx) that terminates TLS; keep the daemon on loopback.

```bash
# Liveness probe
curl http://127.0.0.1:8787/health

# Run a task
curl -X POST http://127.0.0.1:8787/ \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "list the src directory"}'

# Different X-Session-Id = isolated conversation
curl -X POST http://127.0.0.1:8787/ \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Session-Id: project-alpha" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "continue the refactor"}'
```

## Deploying the daemon on a VPS

### One-shot install

`scripts/deploy-debian.sh` installs the daemon on Debian/Ubuntu in one shot (run as root):

```bash
sudo bash scripts/deploy-debian.sh                 # build `agent` from source (installs Rust if needed)
sudo bash scripts/deploy-debian.sh /path/to/agent  # …or install a prebuilt binary
```

| Step | What it does |
|------|--------------|
| Install | Binary at `/opt/nanohermes/nanohermes`, workspace `/opt/nanohermes/workspace`, sessions `/opt/nanohermes/sessions` (mode 700). |
| Token | Generates `openssl rand -hex 24` into `/opt/nanohermes/.daemon_token` (reused on re-runs) and exports it as `NANOHERMES_DAEMON_TOKEN`. |
| Service | Writes `/etc/systemd/system/nanohermes.service`: `--daemon --daemon-port 8787 -C <workspace> --session <dir>/default.json --sandbox workspace-write`, `Restart=always`, plus `NoNewPrivileges` and `ProtectSystem=strict` with `ReadWritePaths`. |
| Start | `systemctl enable --now`, then curls `http://127.0.0.1:8787/health` and prints the token. |

It prompts for the LLM API key (`NANOHERMES_API_KEY`) unless that variable is already set. On a 1 GB instance, add swap before building — compilation is the only memory-hungry step:

```bash
fallocate -l 2G /swapfile && chmod 600 /swapfile && mkswap /swapfile && swapon /swapfile
echo '/swapfile none swap sw 0 0' >> /etc/fstab
```

### Point the daemon at a model

The generated unit only carries the API key, so a non-OpenAI endpoint needs a drop-in that overrides the base URL and model, then a restart:

```bash
sudo mkdir -p /etc/systemd/system/nanohermes.service.d /root/.nanohermes
sudo tee /etc/systemd/system/nanohermes.service.d/env.conf >/dev/null <<'EOF'
[Service]
Environment="NANOHERMES_BASE_URL=https://openrouter.ai/api/v1"
Environment="NANOHERMES_MODEL=anthropic/claude-sonnet-4"
# Long-term memory writes ~/.nanohermes/memory.json, which ProtectSystem=strict blocks.
ReadWritePaths=/root/.nanohermes
EOF
sudo systemctl daemon-reload && sudo systemctl restart nanohermes
```

If long-term memory is disabled (`"memory": false`) the `ReadWritePaths` line can be dropped. The same drop-in accepts any other `NANOHERMES_*` variable, plus `ExecStart=` overrides (for example adding `--permission-mode goal`).

### Expose it safely

The daemon speaks plain HTTP on `127.0.0.1:8787`. Keep it there and put one of these in front:

- **SSH tunnel (single user, no open ports)** — `ssh -N -L 8787:127.0.0.1:8787 root@<host>`, then use `http://127.0.0.1:8787`.
- **TLS reverse proxy on a free port** — if 443 is already taken (another proxy, Xray, a web server), use 8443. Let's Encrypt HTTP-01 still uses port 80, which must stay reachable:
  ```bash
  sudo apt install -y caddy
  sudo tee /etc/caddy/Caddyfile >/dev/null <<'EOF'
  daemon.example.com:8443 {
      reverse_proxy 127.0.0.1:8787
  }
  EOF
  sudo systemctl reload caddy
  ```
  Without a domain, `<ip-with-dashes>.sslip.io` resolves to the host automatically (for example `203-0-113-7.sslip.io`).
- **Cloudflare Tunnel** — map `127.0.0.1:8787` to a hostname without opening any inbound port.

Never publish 8787 directly: the only credential is the Bearer token, and the traffic would be plaintext.

### Connect the desktop app

1. **Settings → Execution location**: pick **Cloud**, enter the URL (`https://daemon.example.com:8443` or the tunnel address) and the token from `/opt/nanohermes/.daemon_token`, then click **Check** (`GET /health`).
2. New tasks can then be created as cloud tasks; each one carries its own `X-Session-Id`, so conversations stay separate on the daemon.
3. Cloud turns run with the daemon's configuration: model profile, persona, advanced tuning and the permission mode are **not** sent from the desktop. Permission mode is fixed at daemon startup (`--permission-mode`), so changing it means restarting the service. A cloud turn can be stopped from the desktop, but the stop takes effect when the daemon emits its next stream event — the daemon owns the whole request and has no out-of-band cancel channel.

### Operations

| Task | Command |
|------|---------|
| Status / logs | `systemctl status nanohermes` · `journalctl -u nanohermes -f` |
| Update | Replace `/opt/nanohermes/nanohermes`, then `systemctl restart nanohermes` |
| Back up | Copy `/opt/nanohermes/sessions/` and `/opt/nanohermes/workspace/` |
| Rotate token | `rm /opt/nanohermes/.daemon_token`, re-run the deploy script, update the desktop setting |
| Show token | `cat /opt/nanohermes/.daemon_token` (mode 600, root only) |

> The daemon runs as root and its tools can read, write and execute commands in the workspace. Anyone with the token gets that access, so keep it private, prefer a non-public entry point, and keep the host patched.

> The source crate is `agent`, so the locally built binary is `target/release/agent`; the deployment script installs it on the server as `/opt/nanohermes/nanohermes`, matching the systemd unit `nanohermes.service` and the `NANOHERMES_*` environment variables.

## Session persistence

```bash
# Start a new session (file is created after the first turn)
agent --session work.json "refactor the search module"

# Resume later — history is loaded from the file
agent --session work.json

# Resume the most recent session recorded for this directory (no file to remember)
agent -c

# List saved sessions and pick one
agent -r
```

`-c` / `-r` use the sessions directory `~/.nanohermes/sessions/`, keyed by a hash of the workspace path; `--session` keeps using the path you give it.

- The session file is a JSON array of OpenAI-style messages (system, user, assistant, tool).
- It is written atomically (write to a `.tmp` sibling, then rename) so a crash mid-write cannot truncate the session.
- The file is saved after every completed turn and after `/clear` — including turns that fail at the API, so fixing the key and retrying keeps the conversation.
- If the file cannot be loaded on startup, NanoHermes falls back to a fresh conversation and prints a warning.

## Context management

The conversation is kept in memory as OpenAI message objects. When the total size exceeds ~120 KB, NanoHermes compacts it: the oldest messages (beyond the 10 most recent) are replaced by a single system message holding a model-generated summary of the dropped region. If summarization fails, a plain truncation notice is used instead. Run `/compact` to force this at any time.

## Multimodal input

Attach one or more images for vision-capable models:

```bash
agent -m gpt-4o -i screenshot.png "what does this UI show?"
agent -m gpt-4o -i a.png -i b.png "compare these two diagrams"
```

Images are read from disk, base64-encoded, and sent as `image_url` content parts alongside the text prompt.

In the REPL, queue images with `/image <path>` and they are attached to the next task:

```
› /image screenshot.png
› what does this UI show?
```

## Architecture & design notes

- **No async runtime.** HTTP uses the blocking `ureq` client; MCP I/O uses a dedicated reader thread per server; tool calls run on scoped OS threads. This avoids the memory overhead of a tokio/async runtime.
- **Minimal dependencies.** Only `serde_json`, `ureq` (with TLS), and `base64`. No `serde` derive proc-macros (values are handled as `serde_json::Value`). No `libc`: the Landlock sandbox and the SIGINT handler reach the kernel through a tiny raw-syscall shim (`agent/src/sys.rs`).
- **Bounded work and output.** Tool results are clamped to 20 KB; command output is capped at 256 KB per stream; file reads refuse files over 4 MB; project guidance and skill bodies are read at most 64 KB and injected up to 20 KB. Glob matching has a work budget (nested `**` cannot hang a turn), hooks time out after 30 s plus a bounded drain, and `git` calls are capped at 60 s. This keeps the context window, process memory and turn latency bounded.
- **Thread safety.** `Server` wraps its `Child` process and reader `JoinHandle` in `Mutex` so that `Manager` (and therefore `Agent`) is `Sync`, enabling safe concurrent tool execution. State mutexes in the GUI and MCP layers recover from poisoning, so one panicking thread does not brick later calls.

## Desktop performance notes

- **No blocking Tauri commands.** A synchronous `#[tauri::command]` runs on the window's main thread, so any disk or network work would freeze the UI for the whole turn. Everything that touches the filesystem, spawns a process or makes an HTTP request is an `async fn` that hands the work to `tauri::async_runtime::spawn_blocking`; only the two pure-memory commands (`get_settings`, `stop_task`) stay synchronous. Concurrent settings saves are serialised *and ordered*: each command records its arrival sequence and an older snapshot is discarded instead of overwriting a newer one, and the file is written temp-then-rename (a corrupt file is backed up rather than silently resetting every profile). Saving never rebuilds the agent mid-turn; a pending-rebind marker makes the next local turn pick up the new settings.
- **Streaming is isolated.** Deltas never enter the global store while a turn is running. They travel through a dedicated subscription channel that only the currently streaming bubble subscribes to, and are committed to the transcript once per phase (tool call, turn end), so a long answer does not re-render the sidebar, panels or title bar.
- **Virtualised transcript.** Only the messages in (and near) the viewport are mounted. Height measurement is skipped while the last bubble is growing, and follow-the-bottom scrolling is driven from the streaming channel without a React render.
- **Incremental Markdown.** During streaming, the answer is split at blank lines outside code fences: settled blocks are rendered once and kept, an unclosed fence only updates its `code` text node, and syntax highlighting runs per code block after the turn settles rather than over the whole answer at once.

## Rust test coverage

```bash
npm run test:all         # Rust workspace + frontend (from the repo root)
npm run build            # frontend: type-check + bundle
```

The frontend suite (Vitest + jsdom) is described above; the Rust workspace tests include:
- Patch application (simple, multi-hunk, pure insertion, non-ASCII context, missing-context errors)
- Glob matching (character-wise `?`, bounded nested `**`)
- File-tool sandbox: `read-only` refuses writes, `workspace-write` rejects paths outside the root
- MCP resource/prompt listing and reading against a mock stdio server
- MCP tool auto-dispatch
- MCP streamable-HTTP handshake and tool call
- MCP concurrency safety (concurrent mixed requests, stress bursts)
- Agent `execute()` concurrency (built-in tools, mixed MCP + built-in)
- Web tools: SSRF target rejection and redirect resolution, HTML-to-text conversion, search-result parsing
- Session task list (`todo_write` / `todo_read`)
- Git tools (status / diff / commit / log round-trip)
- Token-usage parsing and retry backoff computation
- Streaming and buffered LLM replies against a local mock endpoint, including transient-5xx retry and stream-truncation handling
- Multimodal content assembly (text + image parts)
- Session save/load round-trip, including failed turns
- Project guidance injection from `AGENTS.md` (and disabling it)
- Skill discovery (frontmatter, fallback naming, workspace shadowing)
- Hook execution (exit codes, tool-name matcher, stdin payload, a hook that writes before reading stdin does not deadlock)
- Sub-agent job board ordering and the concurrency gate
- Approval-policy decisions and trusted-command detection (git `-c`/`--git-dir` and write-capable "read-only" tools are untrusted)
- Landlock enforcement of the workspace boundary
- SIGINT flag set/clear
- Long-term memory (CJK unigram/bigram tokenisation, BM25 ranking, eviction order, deduplication, distillation gating)
- Desktop shell: NDJSON stream parsing (deltas, tool events, error and truncation detection), folder browsing, model/daemon URL normalisation, cloud-location parsing, session-id validation, export file-name sanitising, output clipping, and a regression test asserting that long-running commands never run synchronously on the main thread

The mock MCP server used by tests lives in `agent/tests/mock_mcp_server.py`.

## License

MIT
