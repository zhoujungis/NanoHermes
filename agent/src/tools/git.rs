//! Git tools: read-only inspection (status/diff/log) plus a confirmed `commit`.
//! These wrap the `git` binary in the workspace root rather than reimplementing
//! plumbing, so behaviour matches what the user would run by hand.

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

use super::{shell, Ctx};

/// `git_status` — short status of the working tree.
pub fn git_status(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let mut a = vec!["status".into(), "--short".into(), "--branch".into()];
    if let Some(p) = path_arg(args) {
        a.push("--".into());
        a.push(p);
    }
    let out = run_git(&ctx.root, &a)?;
    if out.trim().is_empty() {
        Ok("(clean working tree)".to_string())
    } else {
        Ok(out)
    }
}

/// `git_diff` — unstaged changes, or staged changes when `staged` is true.
pub fn git_diff(args: &Value, ctx: &Ctx) -> Result<String, String> {
    // `diff.external` / textconv drivers come from the repo config and can run
    // commands; the model should only see the plain diff.
    let mut a = vec![
        "diff".into(),
        "--no-ext-diff".into(),
        "--no-textconv".into(),
        "--no-color".into(),
    ];
    if args
        .get("staged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        a.push("--staged".into());
    }
    if let Some(p) = path_arg(args) {
        a.push("--".into());
        a.push(p);
    }
    let out = run_git(&ctx.root, &a)?;
    if out.trim().is_empty() {
        Ok("(no changes)".to_string())
    } else {
        Ok(out)
    }
}

/// `git_log` — recent commits, one line each.
pub fn git_log(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let n = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .clamp(1, 100);
    let mut a = vec![
        "log".into(),
        format!("-n{n}"),
        "--oneline".into(),
        "--decorate".into(),
    ];
    if let Some(p) = path_arg(args) {
        a.push("--".into());
        a.push(p);
    }
    let out = run_git(&ctx.root, &a)?;
    // `git log` fails on a repo with no commits; treat that as empty, not an error.
    if out.trim().is_empty() {
        Ok("(no commits)".to_string())
    } else {
        Ok(out)
    }
}

/// `git_commit` — optionally stage (`all` or `files`) and commit with `message`.
/// Asks the user to confirm first unless auto-approval is on.
pub fn git_commit(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `message`")?;
    if message.trim().is_empty() {
        return Err("`message` must not be empty".to_string());
    }
    let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    let files: Vec<String> = args
        .get("files")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if crate::sandbox::needs_approval(ctx.auto_approve, ctx.approval, "git commit")
        && !shell::confirm(&format!("git commit -m {message:?}"))
    {
        return Ok(
            "Command denied by the user. Do not retry it; ask what to do instead.".to_string(),
        );
    }

    if all {
        run_git(&ctx.root, &["add".into(), "-A".into()])?;
    } else if !files.is_empty() {
        let mut a = vec!["add".into(), "--".into()];
        a.extend(files);
        run_git(&ctx.root, &a)?;
    }

    run_git(
        &ctx.root,
        &["commit".into(), "-m".into(), message.to_string()],
    )
}

fn path_arg(args: &Value) -> Option<String> {
    args.get("path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Run `git` with `args` in `root`, returning combined output on success and a
/// descriptive error (including git's own message) on failure.
fn run_git(root: &Path, args: &[String]) -> Result<String, String> {
    // A git call on a huge repo or a stale network mount can block forever;
    // the tool runs on a turn thread, so bound it like `run_command` does.
    const GIT_TIMEOUT_SECS: u64 = 60;
    let mut child = Command::new("git")
        // Neutralise repo-controlled config that can execute commands.
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("diff.external=")
        .arg("--no-pager")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run `git`: {e} (is git installed?)"))?;

    let out_pipe = child.stdout.take().expect("stdout piped");
    let err_pipe = child.stderr.take().expect("stderr piped");
    let out_thread = std::thread::spawn(move || shell::read_capped(out_pipe));
    let err_thread = std::thread::spawn(move || shell::read_capped(err_pipe));

    let start = std::time::Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if start.elapsed() >= std::time::Duration::from_secs(GIT_TIMEOUT_SECS) {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => return Err(format!("failed to wait for git: {e}")),
        }
    };
    let (out, _) = out_thread.join().unwrap_or_default();
    let (err, _) = err_thread.join().unwrap_or_default();
    if timed_out {
        return Err(format!(
            "git {:?} timed out after {GIT_TIMEOUT_SECS}s and was killed",
            args.first().map(String::as_str).unwrap_or("")
        ));
    }
    let status = status.expect("status present unless timed out");
    let stdout = String::from_utf8_lossy(&out).trim().to_string();
    let stderr = String::from_utf8_lossy(&err).trim().to_string();

    if !status.success() {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".into());
        let mut msg = format!("git {} exited with {code}", args.join(" "));
        if !stdout.is_empty() {
            msg.push_str(&format!("\n{stdout}"));
        }
        if !stderr.is_empty() {
            msg.push_str(&format!("\n{stderr}"));
        }
        return Err(msg);
    }

    let mut combined = stdout;
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }
    if combined.is_empty() {
        combined = format!("git {}: ok", args.join(" "));
    }
    Ok(combined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{tasks::TaskManager, todo::TodoList};
    use serde_json::json;
    use std::process::Command;

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn run_raw(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn ctx_in(dir: &Path) -> Ctx {
        Ctx {
            root: dir.to_path_buf(),
            auto_approve: true,
            approval: crate::config::ApprovalPolicy::Never,
            sandbox: crate::config::SandboxMode::Off,
            max_output: 200_000,
            tasks: TaskManager::new(),
            todos: TodoList::new(),
            skills: Vec::new(),
            board: crate::board::Board::new(),
            gate: crate::board::Gate::new(1),
            memory_dir: None,
        }
    }

    #[test]
    fn status_diff_commit_log_round_trip() {
        if !git_available() {
            eprintln!("skipping: git not installed");
            return;
        }
        let dir = std::env::temp_dir().join(format!("nanohermes-git-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run_raw(&dir, &["init", "-q"]);
        run_raw(&dir, &["config", "user.email", "test@example.com"]);
        run_raw(&dir, &["config", "user.name", "NanoHermes Test"]);
        std::fs::write(dir.join("hello.txt"), "hi\n").unwrap();

        let ctx = ctx_in(&dir);
        let status = git_status(&json!({}), &ctx).unwrap();
        assert!(status.contains("hello.txt"), "status: {status}");

        let commit = git_commit(&json!({ "message": "initial", "all": true }), &ctx).unwrap();
        assert!(commit.contains("initial"), "commit: {commit}");

        let log = git_log(&json!({ "count": 5 }), &ctx).unwrap();
        assert!(log.contains("initial"), "log: {log}");

        std::fs::write(dir.join("hello.txt"), "hi2\n").unwrap();
        let diff = git_diff(&json!({}), &ctx).unwrap();
        assert!(
            diff.contains("-hi") && diff.contains("+hi2"),
            "diff: {diff}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_requires_message() {
        let ctx = ctx_in(Path::new("."));
        assert!(git_commit(&json!({}), &ctx).is_err());
        assert!(git_commit(&json!({ "message": "   " }), &ctx).is_err());
    }
}
