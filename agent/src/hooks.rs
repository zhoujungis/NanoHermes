use std::io::Write;
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

    // Feed the tool call in, then close stdin so the hook sees EOF.
    if let Some(mut stdin) = child.stdin.take() {
        let payload = json!({ "tool": tool, "event": event, "arguments": args }).to_string();
        let _ = stdin.write_all(payload.as_bytes());
    }

    // Drain both pipes on separate threads so a chatty hook cannot deadlock.
    let out_pipe = child.stdout.take().expect("stdout piped");
    let err_pipe = child.stderr.take().expect("stderr piped");
    let out_thread = std::thread::spawn(move || read_capped(out_pipe));
    let err_thread = std::thread::spawn(move || read_capped(err_pipe));

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

    let (out, _) = out_thread.join().unwrap_or_default();
    let (err, _) = err_thread.join().unwrap_or_default();

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
