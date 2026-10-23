use std::collections::HashMap;
use std::io::{self, Read};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{resolve, Ctx};

/// Hard ceiling on bytes captured per background task output.
const CAPTURE_BYTES: usize = 512 * 1024;

/// Output accumulated by the reader threads. `done` counts how many of the two
/// pipes have reached EOF, so `output()` can tell a finished process whose tail
/// is still in flight from one that is genuinely silent.
#[derive(Default)]
struct Captured {
    kept: Vec<u8>,
    total: usize,
    done: usize,
}

/// A running or finished background shell task.
pub struct Task {
    pub id: String,
    pub command: String,
    pub started: Instant,
    child: Option<Child>,
    captured: Arc<Mutex<Captured>>,
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
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to start shell: {e}"))?;

        // Drain both pipes from dedicated threads. Reading them lazily in
        // `output()` would block whenever the task is still running, and an
        // undrained stderr pipe fills up (64 KB) and stalls the child forever.
        let captured = Arc::new(Mutex::new(Captured::default()));
        let pipes: Vec<Box<dyn Read + Send>> = [
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .collect();
        for mut pipe in pipes {
            let captured = Arc::clone(&captured);
            std::thread::spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let mut c = captured.lock().unwrap();
                            c.total += n;
                            if c.kept.len() < CAPTURE_BYTES {
                                let room = CAPTURE_BYTES - c.kept.len();
                                c.kept.extend_from_slice(&buf[..n.min(room)]);
                            }
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                captured.lock().unwrap().done += 1;
            });
        }

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
            captured,
            finished: false,
            exit_code: None,
        };
        self.tasks.lock().unwrap().insert(id.clone(), task);
        Ok(id)
    }

    /// Return the output a task has produced so far. Never blocks: the reader
    /// threads own the pipes, so polling a long-running task is cheap.
    pub fn output(&self, task_id: &str) -> Result<String, String> {
        let mut map = self.tasks.lock().unwrap();
        let task = map
            .get_mut(task_id)
            .ok_or_else(|| format!("unknown task `{task_id}`"))?;

        if !task.finished {
            if let Some(status) = task
                .child
                .as_mut()
                .and_then(|c| c.try_wait().ok().flatten())
            {
                task.finished = true;
                task.exit_code = status.code();
            }
        }
        // The process exited but its pipes may not have reported EOF yet; a
        // short bounded wait avoids reporting "finished" before the tail lands.
        if task.finished {
            let deadline = Instant::now() + Duration::from_millis(100);
            while task.captured.lock().unwrap().done < 2 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        let (kept, total) = {
            let c = task.captured.lock().unwrap();
            (c.kept.clone(), c.total)
        };
        let text = String::from_utf8_lossy(&kept);

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
            report.push_str(if task.finished {
                "(no output)\n"
            } else {
                "(no output yet)\n"
            });
        } else {
            report.push_str(trimmed);
            report.push('\n');
        }
        if total > kept.len() {
            report.push_str(&format!(
                "… output truncated ({} bytes not shown)\n",
                total - kept.len()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SandboxMode;
    use std::path::Path;

    fn poll_until_finished(mgr: &TaskManager, id: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let out = mgr.output(id).expect("task exists");
            if out.contains("[task finished") || Instant::now() >= deadline {
                return out;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn polling_a_running_task_returns_immediately() {
        let mgr = TaskManager::new();
        let cmd = if cfg!(windows) {
            "ping -n 6 127.0.0.1 >nul"
        } else {
            "sleep 5"
        };
        let id = mgr
            .spawn(cmd, Path::new("."), SandboxMode::Off, Path::new("."))
            .expect("spawn");

        let start = Instant::now();
        let out = mgr.output(&id).expect("output");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "polling a running task must not block: {out}"
        );
        assert!(out.contains("no output yet") || !out.is_empty());

        let _ = mgr.kill(&id);
    }

    #[test]
    fn finished_task_reports_stdout_stderr_and_exit_code() {
        let mgr = TaskManager::new();
        let cmd = if cfg!(windows) {
            "echo out & echo err 1>&2"
        } else {
            "echo out; echo err 1>&2"
        };
        let id = mgr
            .spawn(cmd, Path::new("."), SandboxMode::Off, Path::new("."))
            .expect("spawn");

        let out = poll_until_finished(&mgr, &id);
        assert!(out.contains("[task finished, exit code: 0]"), "got: {out}");
        assert!(out.contains("out"), "stdout missing: {out}");
        assert!(out.contains("err"), "stderr missing: {out}");
    }
}
