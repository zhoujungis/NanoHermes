//! Filesystem sandbox for spawned shell commands, plus the approval policy that
//! decides when the user must confirm a command before it runs.
//!
//! On Linux the sandbox is implemented with **Landlock** (kernel >= 5.13),
//! installed in the child process right after `fork` and before `exec` via
//! `Command::pre_exec`, so the restriction covers the command and everything it
//! spawns. Landlock needs no external crates: it is driven through a handful of
//! raw syscalls. When the kernel (or an outer sandbox) does not expose Landlock
//! the requested sandbox is *refused* rather than silently downgraded, so a
//! command never runs unsandboxed by surprise.

use std::path::Path;
use std::process::Command;

use crate::config::{ApprovalPolicy, SandboxMode};

/// Decide whether a shell command must be confirmed by the user before running.
pub fn needs_approval(auto_approve: bool, policy: ApprovalPolicy, command: &str) -> bool {
    if auto_approve {
        return false;
    }
    match policy {
        ApprovalPolicy::Never => false,
        ApprovalPolicy::Untrusted => true,
        ApprovalPolicy::OnRequest => !is_trusted_command(command),
    }
}

/// Read-only shell commands that are safe to run without asking. Any control
/// operator (`|`, `&`, `;`), redirection or command substitution marks the
/// command untrusted regardless of the program it starts with.
pub fn is_trusted_command(command: &str) -> bool {
    let cmd = command.trim();
    if cmd.is_empty() {
        return false;
    }
    if cmd.chars().any(|c| {
        matches!(
            c,
            '|' | '&' | ';' | '<' | '>' | '`' | '$' | '(' | ')' | '\n'
        )
    }) {
        return false;
    }
    let mut tokens = cmd.split_whitespace();
    // Skip leading `KEY=VALUE` environment assignments.
    let program = loop {
        match tokens.next() {
            Some(t) if is_env_assignment(t) => continue,
            Some(t) => break t,
            None => return false,
        }
    };
    let program = program.rsplit('/').next().unwrap_or(program);
    if program == "git" {
        return git_subcommand_is_read_only(&mut tokens);
    }
    TRUSTED_BINARIES.contains(&program)
}

fn is_env_assignment(token: &str) -> bool {
    match token.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_alphabetic() || c == '_')
                    .unwrap_or(false)
                && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

fn git_subcommand_is_read_only<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> bool {
    let mut pending_value = false;
    for token in tokens {
        if pending_value {
            pending_value = false;
            continue;
        }
        // `-c`/`--config-env` can execute commands through core.fsmonitor/pager;
        // `--git-dir`/`--work-tree` point at a repo whose config does the same;
        // `-p`/`--paginate` run the configured pager. None of them are read-only.
        if token == "-c"
            || token == "-p"
            || token == "--paginate"
            || token.starts_with("--config-env")
            || token.starts_with("--git-dir")
            || token.starts_with("--work-tree")
            || token.starts_with("--namespace")
        {
            return false;
        }
        if token.starts_with('-') {
            // `-C <dir>` only changes the working directory; the repo config is
            // read anyway when git runs there.
            if token == "-C" {
                pending_value = true;
            }
            continue;
        }
        return TRUSTED_GIT_SUBCOMMANDS.contains(&token);
    }
    false
}

/// Purely read-only executables that never write or spawn other programs.
const TRUSTED_BINARIES: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "wc",
    "grep",
    "rg",
    "egrep",
    "fgrep",
    "stat",
    "file",
    "pwd",
    "echo",
    "printf",
    "which",
    "whereis",
    "printenv",
    // `sort` is intentionally absent: `sort -o FILE` writes a file.
    "uniq",
    "cut",
    "tr",
    "nl",
    "du",
    "df",
    // `date` is absent: `date -s` sets the system clock.
    "whoami",
    "id",
    "uname",
    "hostname",
    // `tree` is absent: `tree -o FILE` writes a file.
    "jq",
    "true",
    "false",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "md5sum",
    "sha1sum",
    "sha256sum",
    "cksum",
    "diff",
    "comm",
    "cmp",
    "seq",
    "strings",
    // `xxd` is absent: `xxd -r` writes a file.
    "od",
    "hexdump",
    "column",
    "nproc",
    "groups",
    "tac",
    "rev",
    "fold",
    "expand",
    "unexpand",
    "paste",
    "join",
];

/// `git` subcommands that only read repository state.
const TRUSTED_GIT_SUBCOMMANDS: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "rev-parse",
    "ls-files",
    "blame",
    "describe",
    "cat-file",
    "shortlog",
    "whatchanged",
    "diff-tree",
    "diff-files",
];

/// Attach the configured sandbox to `cmd`. Must be called before the command is
/// spawned. Returns an error when a sandbox is requested but unavailable.
pub fn attach(cmd: &mut Command, mode: SandboxMode, workspace: &Path) -> Result<(), String> {
    if !mode.is_active() {
        return Ok(());
    }
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        imp::attach(cmd, mode, workspace)
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        let _ = (cmd, workspace);
        Err(format!(
            "sandbox mode {mode:?} requires Linux with Landlock support; refusing to run unsandboxed"
        ))
    }
}

/// Whether the running kernel exposes Landlock. Callers can use this to warn
/// before a command is refused.
pub fn is_supported() -> bool {
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        imp::landlock_abi().is_some()
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        false
    }
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod imp {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::process::Command;

    use crate::config::SandboxMode;
    use crate::sys::{nr, syscall};

    // ---- Landlock syscall numbers (arch-generic, shared across arches) ----
    const SYS_LANDLOCK_CREATE_RULESET: usize = 444;
    const SYS_LANDLOCK_ADD_RULE: usize = 445;
    const SYS_LANDLOCK_RESTRICT_SELF: usize = 446;

    const PR_SET_NO_NEW_PRIVS: usize = 38;
    const LANDLOCK_CREATE_RULESET_VERSION: usize = 1;
    const LANDLOCK_RULE_PATH_BENEATH: usize = 1;

    const AT_FDCWD: usize = (-100i64) as usize;
    const O_PATH: usize = 0o10000000;
    const O_CLOEXEC: usize = 0o2000000;
    const O_DIRECTORY: usize = 0o200000;

    // Landlock filesystem access rights.
    const FS_EXECUTE: u64 = 1 << 0;
    const FS_WRITE_FILE: u64 = 1 << 1;
    const FS_READ_FILE: u64 = 1 << 2;
    const FS_READ_DIR: u64 = 1 << 3;
    const FS_REMOVE_DIR: u64 = 1 << 4;
    const FS_REMOVE_FILE: u64 = 1 << 5;
    const FS_MAKE_CHAR: u64 = 1 << 6;
    const FS_MAKE_DIR: u64 = 1 << 7;
    const FS_MAKE_REG: u64 = 1 << 8;
    const FS_MAKE_SOCK: u64 = 1 << 9;
    const FS_MAKE_FIFO: u64 = 1 << 10;
    const FS_MAKE_BLOCK: u64 = 1 << 11;
    const FS_MAKE_SYM: u64 = 1 << 12;
    const FS_REFER: u64 = 1 << 13; // ABI >= 2
    const FS_TRUNCATE: u64 = 1 << 14; // ABI >= 3
    const FS_IOCTL_DEV: u64 = 1 << 15; // ABI >= 5

    const ALL_FS: u64 = FS_EXECUTE
        | FS_WRITE_FILE
        | FS_READ_FILE
        | FS_READ_DIR
        | FS_REMOVE_DIR
        | FS_REMOVE_FILE
        | FS_MAKE_CHAR
        | FS_MAKE_DIR
        | FS_MAKE_REG
        | FS_MAKE_SOCK
        | FS_MAKE_FIFO
        | FS_MAKE_BLOCK
        | FS_MAKE_SYM
        | FS_REFER
        | FS_TRUNCATE
        | FS_IOCTL_DEV;

    const READ_EXEC: u64 = FS_EXECUTE | FS_READ_FILE | FS_READ_DIR;

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
        handled_access_net: u64,
        scoped: u64,
    }

    #[repr(C, packed)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    /// Query the Landlock ABI version supported by the running kernel.
    pub fn landlock_abi() -> Option<u32> {
        let r = unsafe {
            syscall(
                SYS_LANDLOCK_CREATE_RULESET,
                0,
                0,
                LANDLOCK_CREATE_RULESET_VERSION,
                0,
                0,
                0,
            )
        };
        if r > 0 {
            Some(r as u32)
        } else {
            None
        }
    }

    /// Mask of filesystem rights the kernel knows about for this ABI.
    fn handled_fs_bits(abi: u32) -> u64 {
        let mut bits = ALL_FS;
        if abi < 2 {
            bits &= !FS_REFER;
        }
        if abi < 3 {
            bits &= !FS_TRUNCATE;
        }
        if abi < 5 {
            bits &= !FS_IOCTL_DEV;
        }
        bits
    }

    /// Size of `struct landlock_ruleset_attr` for this ABI (forward-compatible:
    /// the kernel accepts a structure as small as the fields it understands).
    fn ruleset_size(abi: u32) -> usize {
        if abi >= 6 {
            24
        } else if abi >= 4 {
            16
        } else {
            8
        }
    }

    pub fn attach(cmd: &mut Command, mode: SandboxMode, workspace: &Path) -> Result<(), String> {
        let abi = landlock_abi().ok_or_else(|| {
            "this kernel does not expose Landlock (Linux >= 5.13 required); \
             refusing to run unsandboxed"
                .to_string()
        })?;
        let handled = handled_fs_bits(abi);
        let root = CString::new("/").expect("literal has no NUL");
        let ws = if mode == SandboxMode::WorkspaceWrite {
            Some(
                CString::new(workspace.as_os_str().as_bytes())
                    .map_err(|_| format!("workspace path {workspace:?} contains a NUL byte"))?,
            )
        } else {
            None
        };

        use std::os::unix::process::CommandExt;
        // SAFETY: the closure runs in the freshly forked child before `exec`;
        // it issues raw syscalls and touches only stack-allocated structs, so
        // it performs no allocation and takes no locks.
        unsafe {
            cmd.pre_exec(move || {
                apply(handled, abi, root.as_c_str(), ws.as_deref())
                    .map_err(std::io::Error::from_raw_os_error)
            });
        }
        Ok(())
    }

    /// Install the Landlock restriction in the current (child) process.
    /// Returns the errno on failure. Must run after `fork`, before `exec`.
    unsafe fn apply(handled: u64, abi: u32, root: &CStr, ws: Option<&CStr>) -> Result<(), i32> {
        // 1. A Landlock ruleset can only be enforced under no_new_privs.
        let r = syscall(nr::PRCTL, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0, 0);
        if r < 0 {
            return Err(-(r as i32));
        }

        // 2. Create the ruleset governing every filesystem right we handle.
        let attr = RulesetAttr {
            handled_access_fs: handled,
            handled_access_net: 0,
            scoped: 0,
        };
        let rs = syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            &attr as *const _ as usize,
            ruleset_size(abi),
            0,
            0,
            0,
            0,
        );
        if rs < 0 {
            return Err(-(rs as i32));
        }
        let rs_fd = rs as i32;

        // 3. Read + execute everywhere; write access is denied by default.
        if let Err(e) = add_path_rule(rs_fd, root, READ_EXEC) {
            syscall(nr::CLOSE, rs_fd as usize, 0, 0, 0, 0, 0);
            return Err(e);
        }
        // 4. Full access inside the workspace (workspace-write only).
        if let Some(ws) = ws {
            if let Err(e) = add_path_rule(rs_fd, ws, handled) {
                syscall(nr::CLOSE, rs_fd as usize, 0, 0, 0, 0, 0);
                return Err(e);
            }
        }

        // 5. Enforce the ruleset on this process and its future children.
        let r = syscall(SYS_LANDLOCK_RESTRICT_SELF, rs_fd as usize, 0, 0, 0, 0, 0);
        syscall(nr::CLOSE, rs_fd as usize, 0, 0, 0, 0, 0);
        if r < 0 {
            return Err(-(r as i32));
        }
        Ok(())
    }

    unsafe fn add_path_rule(rs_fd: i32, path: &CStr, allowed: u64) -> Result<(), i32> {
        let fd = syscall(
            nr::OPENAT,
            AT_FDCWD,
            path.as_ptr() as usize,
            O_PATH | O_CLOEXEC | O_DIRECTORY,
            0,
            0,
            0,
        );
        if fd < 0 {
            return Err(-(fd as i32));
        }
        let fd = fd as i32;
        let rule = PathBeneathAttr {
            allowed_access: allowed,
            parent_fd: fd,
        };
        let r = syscall(
            SYS_LANDLOCK_ADD_RULE,
            rs_fd as usize,
            LANDLOCK_RULE_PATH_BENEATH,
            &rule as *const _ as usize,
            0,
            0,
            0,
        );
        syscall(nr::CLOSE, fd as usize, 0, 0, 0, 0, 0);
        if r < 0 {
            Err(-(r as i32))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn read_only_commands_are_trusted() {
        assert!(is_trusted_command("ls -la"));
        assert!(is_trusted_command("cat src/main.rs"));
        assert!(is_trusted_command("grep -n foo src/agent.rs"));
        assert!(is_trusted_command("git status"));
        assert!(is_trusted_command("git -C /repo diff --stat"));
        assert!(is_trusted_command("FOO=bar ls /tmp"));
    }

    #[test]
    fn mutating_commands_are_not_trusted() {
        assert!(!is_trusted_command("rm -rf /"));
        assert!(!is_trusted_command("touch newfile"));
        assert!(!is_trusted_command("git commit -m x"));
        assert!(!is_trusted_command("git push"));
        assert!(!is_trusted_command("echo hi > f"));
        assert!(!is_trusted_command("ls | rm"));
        assert!(!is_trusted_command("ls && rm"));
        assert!(!is_trusted_command("$(rm x)"));
        assert!(!is_trusted_command(""));
        // Config/pager flags can execute commands through a hostile repo.
        assert!(!is_trusted_command(
            "git -c core.fsmonitor='sh -c \"id\"' status"
        ));
        assert!(!is_trusted_command("git --git-dir /tmp/x/.git status"));
        assert!(!is_trusted_command("git -p status"));
        // Write-capable "read-only" tools must not be auto-approved.
        assert!(!is_trusted_command("sort -o out in"));
        assert!(!is_trusted_command("tree -o out"));
        assert!(!is_trusted_command("xxd -r in out"));
        assert!(!is_trusted_command("date -s 2020-01-01"));
    }

    #[test]
    fn approval_policy_decisions() {
        // Never / auto-approve run everything.
        assert!(!needs_approval(true, ApprovalPolicy::Untrusted, "rm -rf /"));
        assert!(!needs_approval(false, ApprovalPolicy::Never, "rm -rf /"));
        // Untrusted asks for everything.
        assert!(needs_approval(false, ApprovalPolicy::Untrusted, "ls"));
        // OnRequest only asks for non-read-only commands.
        assert!(!needs_approval(false, ApprovalPolicy::OnRequest, "ls -la"));
        assert!(needs_approval(false, ApprovalPolicy::OnRequest, "rm -rf /"));
    }

    #[test]
    fn landlock_enforces_workspace_boundaries() {
        if !is_supported() {
            eprintln!("skipping: Landlock unavailable in this environment");
            return;
        }
        let base = std::env::temp_dir().join(format!(
            "nanohermes-sandbox-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let ws = base.join("ws");
        let outside = base.join("outside");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        // read-only: writes inside the workspace are denied.
        assert_eq!(
            run_sandboxed(
                &format!("touch {}/a", ws.display()),
                SandboxMode::ReadOnly,
                &ws
            ),
            Some(false),
            "read-only sandbox must deny writes"
        );
        assert!(!ws.join("a").exists());

        // workspace-write: writes inside the workspace succeed.
        assert_eq!(
            run_sandboxed(
                &format!("touch {}/b", ws.display()),
                SandboxMode::WorkspaceWrite,
                &ws
            ),
            Some(true),
            "workspace-write must allow writes inside the workspace"
        );
        assert!(ws.join("b").exists());

        // workspace-write: writes outside the workspace are denied.
        assert_eq!(
            run_sandboxed(
                &format!("touch {}/c", outside.display()),
                SandboxMode::WorkspaceWrite,
                &ws
            ),
            Some(false),
            "workspace-write must deny writes outside the workspace"
        );
        assert!(!outside.join("c").exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    fn run_sandboxed(command: &str, mode: SandboxMode, ws: &Path) -> Option<bool> {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        attach(&mut cmd, mode, ws).expect("attach sandbox");
        cmd.status().ok().map(|s| s.success())
    }
}
