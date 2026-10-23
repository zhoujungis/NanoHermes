use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::config::{HookDef, HookEvent};
use crate::tools::shell::read_capped;

/// Hooks are meant to be quick guards/notifications; anything slower than this
/// is treated as a failure so a stuck hook cannot stall the agent forever.
const HOOK_TIMEOUT_SECS: u64 = 30;

/// Result of running one hook command.
pub struct HookOutcome {
    /// Process exit code (`-1` if killed by a signal, `-2` if we timed it out).
    pub code: i32,
    /// Combined stdout + stderr, trimmed.
    pub output: String,
}

impl HookOutcome {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Run a single hook via the platform shell (`sh -c` on Unix, `cmd.exe /C` on
/// Windows) in the workspace root. The tool call is written to the hook's stdin
/// as JSON, and the tool name / event are also exported as
/// `NANOHERMES_TOOL_NAME` and `NANOHERMES_HOOK_EVENT`.
pub fn run(def: &HookDef, tool: &str, args: &Value, root: &Path) -> Result<HookOutcome, String> {
    let event = match def.event {
        HookEvent::PreToolUse => "PreToolUse",
        HookEvent::PostToolUse => "PostToolUse",
    };

    let mut child = crate::tools::shell::shell_command(&def.command)
        .current_dir(root)
        .env("NANOHERMES_TOOL_NAME", tool)
        .env("NANOHERMES_HOOK_EVENT", event)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn hook `{}`: {e}", def.command))?;

    // Start draining before writing stdin: a hook that writes output before it
    // reads its input would otherwise fill the stdout pipe and deadlock while
    // the parent blocks in `write_all` (the timeout would never be reached).
    let out_pipe = child.stdout.take().expect("stdout piped");
    let err_pipe = child.stderr.take().expect("stderr piped");
    let (tx, rx) = std::sync::mpsc::channel::<(bool, Vec<u8>)>();
    let pipes: [(bool, Box<dyn Read + Send>); 2] =
        [(true, Box::new(out_pipe)), (false, Box::new(err_pipe))];
    for (is_stdout, pipe) in pipes {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let (kept, _) = read_capped(pipe);
            let _ = tx.send((is_stdout, kept));
        });
    }
    drop(tx);

    // Feed the tool call from its own thread and close stdin afterwards (drop
    // of the handle). Payloads can be large (file contents) and a hook is free
    // not to read them at all.
    if let Some(mut stdin) = child.stdin.take() {
        let payload = json!({ "tool": tool, "event": event, "arguments": args }).to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(payload.as_bytes());
        });
    }

    let start = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) => {
                if start.elapsed() >= Duration::from_secs(HOOK_TIMEOUT_SECS) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break -2;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("failed to wait for hook: {e}")),
        }
    };

    // Collect with a bound instead of joining the reader threads. On timeout the
    // killed shell may leave grandchildren holding the pipes open, and joining
    // them would hang far past the intended deadline.
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut got_out = false;
    let mut got_err = false;
    let deadline = Instant::now() + Duration::from_secs(2);
    while !(got_out && got_err) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(left) {
            Ok((true, kept)) => {
                out = kept;
                got_out = true;
            }
            Ok((false, kept)) => {
                err = kept;
                got_err = true;
            }
            Err(_) => break,
        }
    }

    let mut output = String::new();
    let stdout = String::from_utf8_lossy(&out);
    let stderr = String::from_utf8_lossy(&err);
    if !stdout.trim().is_empty() {
        output.push_str(stdout.trim());
    }
    if !stderr.trim().is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(stderr.trim());
    }
    if code == -2 {
        output.push_str(&format!("\n(hook timed out after {HOOK_TIMEOUT_SECS}s)"));
    }

    Ok(HookOutcome { code, output })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HookEvent;

    fn hook(event: HookEvent, command: &str) -> HookDef {
        HookDef {
            event,
            matcher: "*".to_string(),
            command: command.to_string(),
        }
    }

    // Hook commands are executed via `sh -c`, so these behaviour tests are
    // POSIX-only. The matcher test below is platform-independent.
    #[cfg(unix)]
    #[test]
    fn successful_hook_returns_zero() {
        let def = hook(HookEvent::PostToolUse, "echo checked");
        let out = run(&def, "run_command", &json!({}), Path::new("/")).unwrap();
        assert!(out.ok());
        assert_eq!(out.output, "checked");
    }

    #[cfg(unix)]
    #[test]
    fn hook_exit_code_is_reported() {
        let def = hook(HookEvent::PreToolUse, "exit 3");
        let out = run(&def, "run_command", &json!({}), Path::new("/")).unwrap();
        assert!(!out.ok());
        assert_eq!(out.code, 3);
    }

    #[cfg(unix)]
    #[test]
    fn hook_receives_tool_name_and_arguments() {
        let def = hook(
            HookEvent::PreToolUse,
            r#"test "$NANOHERMES_TOOL_NAME" = read_file && grep -q '"path":"a.txt"' && echo ok"#,
        );
        let out = run(
            &def,
            "read_file",
            &json!({ "path": "a.txt" }),
            Path::new("/"),
        )
        .unwrap();
        assert!(out.ok(), "hook output: {}", out.output);
        assert!(out.output.contains("ok"), "hook output: {}", out.output);
    }

    #[cfg(unix)]
    #[test]
    fn hook_that_writes_before_reading_stdin_does_not_deadlock() {
        // 200 KB to stdout (> pipe buffer) before touching stdin, while stdin
        // carries a payload larger than the pipe buffer too.
        let def = hook(
            HookEvent::PreToolUse,
            "yes x | head -c 200000; cat >/dev/null; echo done",
        );
        let big = "a".repeat(200_000);
        let start = Instant::now();
        let out = run(
            &def,
            "write_file",
            &json!({ "content": big }),
            Path::new("/"),
        )
        .unwrap();
        assert!(out.ok(), "hook output: {}", out.output);
        assert!(out.output.contains("done"), "hook output: {}", out.output);
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "hook deadlocked: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn matcher_filters_tool_names() {
        let def = HookDef {
            event: HookEvent::PreToolUse,
            matcher: "run_command, write_file".to_string(),
            command: "true".to_string(),
        };
        assert!(def.matches("run_command"));
        assert!(def.matches("write_file"));
        assert!(!def.matches("read_file"));

        let all = hook(HookEvent::PreToolUse, "true");
        assert!(all.matches("anything"));
        let exact = HookDef {
            event: HookEvent::PreToolUse,
            matcher: "grep".to_string(),
            command: "true".to_string(),
        };
        assert!(!exact.matches("grep_all"));
    }
}
