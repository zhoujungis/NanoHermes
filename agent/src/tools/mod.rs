pub mod fs;
pub mod git;
pub mod patch;
pub mod search;
pub mod shell;
pub mod skill;
pub mod tasks;
pub mod todo;
pub mod web;

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use tasks::TaskManager;
use todo::TodoList;

use crate::board::{Board, Gate};

/// Shared context handed to every tool.
pub struct Ctx {
    pub root: PathBuf,
    /// When false, `run_command` asks the user to confirm on stdin first.
    pub auto_approve: bool,
    /// How strictly shell commands are gated behind confirmation.
    pub approval: crate::config::ApprovalPolicy,
    /// Filesystem sandbox applied to spawned shell commands (Linux only).
    pub sandbox: crate::config::SandboxMode,
    /// Max characters of tool output fed back to the model (keeps context small).
    pub max_output: usize,
    /// Registry of background tasks shared across all tool invocations.
    pub tasks: TaskManager,
    /// The session task list, shared across all tool invocations.
    pub todos: TodoList,
    /// Skills discovered for this workspace (loaded on demand by the `skill` tool).
    pub skills: Vec<crate::skills::Skill>,
    /// Sub-agent job registry, readable by the `agent_list` tool.
    pub board: Board,
    /// Bounds how many sub-agents may run at once.
    pub gate: Gate,
    /// Long-term memory store directory; `None` disables the memory tools.
    pub memory_dir: Option<PathBuf>,
}

/// OpenAI-style tool (function) declarations.
pub fn specs() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a text file from the workspace. Returns content with line numbers. Use offset/limit for large files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path":   { "type": "string", "description": "File path, absolute or relative to the workspace root." },
                        "offset": { "type": "integer", "description": "1-based first line to read (default 1)." },
                        "limit":  { "type": "integer", "description": "Maximum number of lines to read (default 2000)." }
                    },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Create or overwrite a file with the given content. Creates parent directories as needed. Prefer edit_file for small changes.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path":    { "type": "string", "description": "File path." },
                        "content": { "type": "string", "description": "Full file content to write." }
                    },
                    "required": ["path", "content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "Replace an exact string in a file. old_string must match the file byte-for-byte, including indentation. Set replace_all to replace every occurrence.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path":         { "type": "string", "description": "File path." },
                        "old_string":   { "type": "string", "description": "Exact text to find." },
                        "new_string":   { "type": "string", "description": "Replacement text." },
                        "replace_all":  { "type": "boolean", "description": "Replace all occurrences (default false)." }
                    },
                    "required": ["path", "old_string", "new_string"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_dir",
                "description": "List the entries of a directory (directories are suffixed with '/').",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path (default: workspace root)." }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "apply_patch",
                "description": "Apply a unified diff (patch) to a file. Use this for multi-line changes instead of edit_file. The diff must include @@ hunks with matching context lines.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path to patch." },
                        "diff": { "type": "string", "description": "Unified diff text, e.g. '@@ -1,3 +1,3 @@\\n context\\n-old\\n+new\\n context'." }
                    },
                    "required": ["path", "diff"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "grep",
                "description": "Search file contents for a literal substring, recursively. Returns 'path:line:text' matches. Build/tool directories are skipped.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern":          { "type": "string", "description": "Literal text to search for." },
                        "path":             { "type": "string", "description": "File or directory to search (default: workspace root)." },
                        "include":          { "type": "string", "description": "Glob filter, e.g. '*.rs' or 'src/**/*.ts'." },
                        "case_insensitive": { "type": "boolean", "description": "Case-insensitive match (default false)." }
                    },
                    "required": ["pattern"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "glob",
                "description": "Find files by glob pattern relative to the workspace root, e.g. 'src/**/*.rs'. Supports * ? and **.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Glob pattern." }
                    },
                    "required": ["pattern"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "Run a shell command in the workspace and return its exit code, stdout and stderr. Commands go through `sh -c` on Linux/macOS and `cmd.exe /C` on Windows, so write syntax for that shell. Use non-interactive flags. Set background=true for long-running tasks, then poll with task_output.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command":      { "type": "string", "description": "Shell command to execute." },
                        "cwd":          { "type": "string", "description": "Working directory (default: workspace root)." },
                        "timeout_secs": { "type": "integer", "description": "Kill the command after this many seconds (default 120)." },
                        "background":   { "type": "boolean", "description": "Run in the background and return a task id (default false)." }
                    },
                    "required": ["command"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "task_output",
                "description": "Retrieve the current stdout/stderr output of a background task by task id. If the task has finished, the exit code is included.",
                "parameters": {
                    "type": "object",
                    "properties": { "task_id": { "type": "string", "description": "The task id returned by run_command with background=true." } },
                    "required": ["task_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "task_list",
                "description": "List all background tasks with their id, command and status (running/finished).",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "task_kill",
                "description": "Kill a running background task by task id.",
                "parameters": {
                    "type": "object",
                    "properties": { "task_id": { "type": "string", "description": "The task id to kill." } },
                    "required": ["task_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "web_fetch",
                "description": "Fetch a URL over HTTP(S) and return its content as plain text. HTML pages are converted to readable text. Use this to read documentation, issues, or any web page.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url":       { "type": "string", "description": "http:// or https:// URL to fetch." },
                        "max_chars": { "type": "integer", "description": "Maximum characters of text to return (default 20000)." }
                    },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Search the web and return the top results with title, URL and snippet. Follow up with web_fetch on a result URL to read the page.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query." },
                        "count": { "type": "integer", "description": "Number of results to return (default 5, max 10)." }
                    },
                    "required": ["query"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "todo_write",
                "description": "Create or replace the session task list to plan and track multi-step work. Call it when starting a task and again as steps progress, marking items in_progress/completed.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "todos": {
                            "type": "array",
                            "description": "The full task list, in order. Replaces any previous list.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "content": { "type": "string", "description": "What the step is." },
                                    "status": { "type": "string", "enum": ["pending", "in_progress", "completed"], "description": "Step status (default pending)." }
                                },
                                "required": ["content"]
                            }
                        }
                    },
                    "required": ["todos"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "todo_read",
                "description": "Return the current session task list.",
                "parameters": { "type": "object", "properties": {} }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "git_status",
                "description": "Show the git working-tree status (short form) and current branch.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Limit to a path (optional)." }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "git_diff",
                "description": "Show a unified diff of changes. Set staged=true to diff the index instead of the working tree.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "staged": { "type": "boolean", "description": "Diff staged changes (default false)." },
                        "path":   { "type": "string", "description": "Limit to a path (optional)." }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "git_log",
                "description": "Show recent commits (one line each) with refs.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "count": { "type": "integer", "description": "Number of commits (default 10, max 100)." },
                        "path":  { "type": "string", "description": "Limit to a path (optional)." }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "git_commit",
                "description": "Create a git commit. Optionally stage changes first: set all=true to stage everything, or list paths in files. Asks the user to confirm unless auto-approval is enabled.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "message": { "type": "string", "description": "Commit message." },
                        "all":     { "type": "boolean", "description": "Stage all changes before committing (default false)." },
                        "files":   { "type": "array", "items": { "type": "string" }, "description": "Paths to stage before committing (optional)." }
                    },
                    "required": ["message"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "memory_save",
                "description": "Write a durable memory (lesson / preference / correction / fact) to the long-term memory store, so future sessions can retrieve it. Use for things worth remembering across tasks: user preferences, corrections of your behaviour, hard-won lessons, stable facts.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "enum": ["lesson", "preference", "correction", "fact"], "description": "What sort of memory this is." },
                        "text": { "type": "string", "description": "One concise, self-contained sentence (no conversation context needed to understand it)." }
                    },
                    "required": ["kind", "text"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "memory_search",
                "description": "Search the long-term memory store for entries relevant to a query. Returns the best matches from past sessions (preferences, corrections, lessons, facts).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "What to look for, e.g. '代码风格偏好' or '部署流程教训'." },
                        "top_k": { "type": "integer", "description": "Maximum number of entries to return (default 5, max 20)." }
                    },
                    "required": ["query"]
                }
            }
        }
    ])
}

/// Route a tool call to its implementation. Errors are returned as strings so
/// they can be handed back to the model as a tool result.
pub fn dispatch(name: &str, args: &Value, ctx: &Ctx) -> Result<String, String> {
    let r = match name {
        "read_file" => fs::read_file(args, ctx),
        "write_file" => fs::write_file(args, ctx),
        "edit_file" => fs::edit_file(args, ctx),
        "apply_patch" => patch::apply_patch(args, ctx),
        "list_dir" => fs::list_dir(args, ctx),
        "grep" => search::grep(args, ctx),
        "glob" => search::glob(args, ctx),
        "run_command" => shell::run_command(args, ctx),
        "task_output" => tasks::task_output(args, ctx),
        "task_list" => tasks::task_list(args, ctx),
        "task_kill" => tasks::task_kill(args, ctx),
        "web_fetch" => web::web_fetch(args, ctx),
        "web_search" => web::web_search(args, ctx),
        "todo_write" => todo::todo_write(args, ctx),
        "todo_read" => todo::todo_read(args, ctx),
        "git_status" => git::git_status(args, ctx),
        "git_diff" => git::git_diff(args, ctx),
        "git_log" => git::git_log(args, ctx),
        "git_commit" => git::git_commit(args, ctx),
        "skill" => skill::load(args, ctx),
        "memory_save" => match &ctx.memory_dir {
            Some(dir) => crate::memory::save_tool(args, dir),
            None => Err("long-term memory is disabled (settings)".to_string()),
        },
        "memory_search" => match &ctx.memory_dir {
            Some(dir) => crate::memory::search_tool(args, dir),
            None => Err("long-term memory is disabled (settings)".to_string()),
        },
        "agent_list" => Ok(render_agent_board(ctx)),
        other => Err(format!("unknown tool `{other}`")),
    };
    r.map(|s| clamp(s, ctx.max_output))
}

/// Resolve a user-supplied path against the workspace root.
pub fn resolve(root: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Enforce the configured filesystem sandbox for the *file tools*. The shell
/// sandbox only covers spawned commands, so without this `write_file` could edit
/// `~/.bashrc` even under `--sandbox read-only`.
pub fn check_write(ctx: &Ctx, path: &Path) -> Result<(), String> {
    match ctx.sandbox {
        crate::config::SandboxMode::ReadOnly => Err(
            "sandbox is read-only: file writes are disabled (use run_command with --sandbox off to allow them)"
                .to_string(),
        ),
        crate::config::SandboxMode::WorkspaceWrite => {
            let root = std::fs::canonicalize(&ctx.root)
                .map_err(|e| format!("workspace unavailable: {e}"))?;
            let target = canonical_with_missing(path)
                .ok_or_else(|| format!("cannot resolve {}", path.display()))?;
            if target.starts_with(&root) {
                Ok(())
            } else {
                Err(format!(
                    "sandbox is workspace-write: {} is outside the workspace",
                    path.display()
                ))
            }
        }
        // Off / danger-full-access: explicitly unrestricted.
        _ => Ok(()),
    }
}

/// Canonicalise a path whose final components may not exist yet (write_file
/// creates parent directories). Walks up to the nearest existing ancestor and
/// re-appends the missing tail.
fn canonical_with_missing(path: &Path) -> Option<PathBuf> {
    let mut cur = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(base) = std::fs::canonicalize(&cur) {
            let mut out = base;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return Some(out);
        }
        tail.push(cur.file_name()?.to_os_string());
        if !cur.pop() {
            return None;
        }
    }
}

/// Render the sub-agent job board for the `agent_list` tool.
fn render_agent_board(ctx: &Ctx) -> String {
    let jobs = ctx.board.list();
    if jobs.is_empty() {
        return "No sub-agent jobs have been started in this session.".to_string();
    }
    let mut out = String::from("Sub-agent jobs:\n");
    for job in jobs {
        let duration = match job.duration {
            Some(d) => format!("{:.1}s", d.as_secs_f64()),
            None => "…".to_string(),
        };
        out.push_str(&format!(
            "- {} [{}] {} ({}) — {}\n",
            job.id,
            job.status.as_str(),
            job.name,
            duration,
            job.task
        ));
        if !job.result.is_empty() {
            out.push_str(&format!("  result: {}\n", job.result));
        }
    }
    out
}

fn clamp(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n… output truncated ({} chars omitted)",
        &s[..end],
        s.len() - end
    )
}
