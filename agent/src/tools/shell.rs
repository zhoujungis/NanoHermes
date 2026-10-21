use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{resolve, Ctx};

/// Hard ceiling on bytes captured per stream before we stop keeping them.
const CAPTURE_BYTES: usize = 256 * 1024;

pub fn run_command(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `command`")?;
    if command.trim().is_empty() {
        return Err("`command` must not be empty".to_string());
    }

    // Background mode: spawn and return a task id immediately.
    if args
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return super::tasks::run_background(args, ctx);
    }

    let cwd = match args.get("cwd").and_then(|v| v.as_str()) {
        Some(c) if !c.is_empty() => resolve(&ctx.root, c),
        _ => ctx.root.clone(),
    };
    let timeout = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(120)
        .clamp(1, 3600);

    if crate::sandbox::needs_approval(ctx.auto_approve, ctx.approval, command) && !confirm(command)
    {
        return Ok(
            "Command denied by the user. Do not retry it; ask what to do instead.".to_string(),
        );
    }

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Apply the filesystem sandbox (no-op unless a sandbox mode is configured).
    crate::sandbox::attach(&mut cmd, ctx.sandbox, &ctx.root)
        .map_err(|e| format!("sandbox error: {e}"))?;
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn `sh`: {e}"))?;

    // Drain both pipes on separate threads so a chatty child can never deadlock
    // by filling the OS pipe buffer while we wait for it to exit.
    let out_pipe = child.stdout.take().expect("stdout piped");
    let err_pipe = child.stderr.take().expect("stderr piped");
    let out_thread = std::thread::spawn(move || read_capped(out_pipe));
    let err_thread = std::thread::spawn(move || read_capped(err_pipe));

    let start = Instant::now();
    let mut timed_out = false;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if start.elapsed() >= Duration::from_secs(timeout) {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(format!("failed to wait for command: {e}")),
        }
    };

    let (out, out_total) = out_thread.join().unwrap_or_default();
    let (err, err_total) = err_thread.join().unwrap_or_default();

    let mut report = String::new();
    if timed_out {
        report.push_str(&format!(
            "command timed out after {timeout}s and was killed\n"
        ));
    }
    report.push_str(&format!(
        "exit code: {}\n",
        exit.map(|c| c.to_string())
            .unwrap_or_else(|| "killed".into())
    ));
    append_stream(&mut report, "stdout", &out, out_total);
    append_stream(&mut report, "stderr", &err, err_total);
    Ok(report)
}

fn append_stream(report: &mut String, label: &str, bytes: &[u8], total: usize) {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim_end();
    report.push_str(&format!("--- {label} ---\n"));
    if trimmed.is_empty() {
        report.push_str("(empty)\n");
    } else {
        report.push_str(trimmed);
        report.push('\n');
    }
    if total > bytes.len() {
        report.push_str(&format!(
            "… {label} truncated ({} bytes not shown)\n",
            total - bytes.len()
        ));
    }
}

/// Read everything from `r`, but only *keep* the first `CAPTURE_BYTES` bytes so a
/// runaway command cannot balloon the agent's memory. The rest is drained and discarded.
pub(crate) fn read_capped(mut r: impl Read) -> (Vec<u8>, usize) {
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

pub fn confirm(command: &str) -> bool {
    eprint!("Run this command? [y/N] {command}\n> ");
    let _ = io::stderr().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}
