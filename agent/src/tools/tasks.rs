use std::collections::HashMap;
use std::io::{self, Read};
use std::process::{Child, Stdio};
use std::sync::Mutex;
use std::time::Instant;

use serde_json::Value;

use super::{resolve, Ctx};

/// Hard ceiling on bytes captured per background task output.
const CAPTURE_BYTES: usize = 512 * 1024;

/// A running or finished background shell task.
pub struct Task {
    pub id: String,
    pub command: String,
    pub started: Instant,
    child: Option<Child>,
    out_buf: Vec<u8>,
    out_total: usize,
    finished: bool,
    exit_code: Option<i32>,
}

/// Thread-safe registry of background tasks. Stored in `Ctx` so every tool
/// call (including concurrent ones) can start, query and kill tasks.
pub struct TaskManager {
    tasks: Mutex<HashMap<String, Task>>,
}

impl TaskManager {
    pub fn new() -> Self {
        TaskManager {
            tasks: Mutex::new(HashMap::new()),
        }
    }

    /// Spawn a shell command in the background and return its task id.
    pub fn spawn(
        &self,
        command: &str,
        cwd: &std::path::Path,
        sandbox: crate::config::SandboxMode,
        workspace: &std::path::Path,
    ) -> Result<String, String> {
        let mut cmd = super::shell::shell_command(command);
        cmd.current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::sandbox::attach(&mut cmd, sandbox, workspace)
            .map_err(|e| format!("sandbox error: {e}"))?;
        let child = cmd
            .spawn()
            .map_err(|e| format!("failed to start shell: {e}"))?;

        let id = format!("task-{}", std::process::id());
        // Append a short random-ish suffix based on the instant to avoid
        // collisions when several tasks are spawned in the same millisecond.
        let id = format!(
            "{}-{}",
            id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );

        let task = Task {
            id: id.clone(),
            command: command.to_string(),
            started: Instant::now(),
            child: Some(child),
            out_buf: Vec::new(),
            out_total: 0,
            finished: false,
            exit_code: None,
        };
        self.tasks.lock().unwrap().insert(id.clone(), task);
        Ok(id)
    }

    /// Drain any new output from a task's stdout/stderr and return it as text.
    /// If the task has finished, mark it and include the exit code.
    pub fn output(&self, task_id: &str) -> Result<String, String> {
        let mut map = self.tasks.lock().unwrap();
        let task = map
            .get_mut(task_id)
            .ok_or_else(|| format!("unknown task `{task_id}`"))?;

        // Drain stdout.
        if let Some(child) = task.child.as_mut() {
            if let Some(mut out) = child.stdout.take() {
                let (kept, total) = read_capped(&mut out);
                task.out_buf.extend_from_slice(&kept);
                task.out_total += total;
                child.stdout = Some(out);
            }
            // Check if finished.
            match child.try_wait() {
                Ok(Some(status)) => {
                    task.finished = true;
                    task.exit_code = status.code();
                }
                Ok(None) => {}
                Err(_) => {}
            }
        }

        let text = String::from_utf8_lossy(&task.out_buf);
        let mut report = String::new();
        if task.finished {
            report.push_str(&format!(
                "[task finished, exit code: {}]\n",
                task.exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "killed".into())
            ));
        }
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            report.push_str("(no output yet)\n");
        } else {
            report.push_str(trimmed);
            report.push('\n');
        }
        if task.out_total > task.out_buf.len() {
            report.push_str(&format!(
                "… output truncated ({} bytes not shown)\n",
                task.out_total - task.out_buf.len()
            ));
        }
        Ok(report)
    }

    /// List all tasks with their status.
    pub fn list(&self) -> String {
        let map = self.tasks.lock().unwrap();
        if map.is_empty() {
            return "(no background tasks)".to_string();
        }
        let mut out = String::new();
        for task in map.values() {
            let elapsed = task.started.elapsed().as_secs();
            let status = if task.finished {
                format!(
                    "finished (exit {})",
                    task.exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "?".into())
                )
            } else {
                format!("running ({elapsed}s)")
            };
            out.push_str(&format!("{}  {}  [{}]\n", task.id, task.command, status));
        }
        out.trim_end().to_string()
    }

    /// Kill a running task.
    pub fn kill(&self, task_id: &str) -> Result<String, String> {
        let mut map = self.tasks.lock().unwrap();
        let task = map
            .get_mut(task_id)
            .ok_or_else(|| format!("unknown task `{task_id}`"))?;
        if task.finished {
            return Ok(format!("task `{task_id}` already finished"));
        }
        if let Some(child) = task.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        task.finished = true;
        Ok(format!("killed task `{task_id}`"))
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

fn read_capped(mut r: impl Read) -> (Vec<u8>, usize) {
    let mut kept: Vec<u8> = Vec::new();
    let mut total = 0usize;
    let mut buf = [0u8; 16 * 1024];
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if kept.len() < CAPTURE_BYTES {
                    let room = CAPTURE_BYTES - kept.len();
                    kept.extend_from_slice(&buf[..n.min(room)]);
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    (kept, total)
}

// ---- Tool implementations called from dispatch ----

/// Start a command in the background. Returns a task id the model can later
/// poll with `task_output`.
pub fn run_background(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `command`")?;
    if command.trim().is_empty() {
        return Err("`command` must not be empty".to_string());
    }
    let cwd = match args.get("cwd").and_then(|v| v.as_str()) {
        Some(c) if !c.is_empty() => resolve(&ctx.root, c),
        _ => ctx.root.clone(),
    };
    if crate::sandbox::needs_approval(ctx.auto_approve, ctx.approval, command)
        && !super::shell::confirm(command)
    {
        return Ok(
            "Command denied by the user. Do not retry it; ask what to do instead.".to_string(),
        );
    }
    let id = ctx.tasks.spawn(command, &cwd, ctx.sandbox, &ctx.root)?;
    Ok(format!(
        "started background task `{id}`\nuse task_output to retrieve its output"
    ))
}

/// Retrieve the current output of a background task.
pub fn task_output(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let id = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `task_id`")?;
    ctx.tasks.output(id)
}

/// List all background tasks.
pub fn task_list(_args: &Value, ctx: &Ctx) -> Result<String, String> {
    Ok(ctx.tasks.list())
}

/// Kill a background task.
pub fn task_kill(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let id = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `task_id`")?;
    ctx.tasks.kill(id)
}
