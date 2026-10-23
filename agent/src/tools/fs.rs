use std::fs;

use serde_json::Value;

use super::{resolve, Ctx};

/// Refuse to slurp files bigger than this in one go.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

pub fn read_file(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let path = req_str(args, "path")?;
    let full = resolve(&ctx.root, path);
    // Clamp through `usize` before doing arithmetic: the model can send any
    // u64, and `offset - 1 + limit` used to overflow (panic in debug, wrap in
    // release followed by an out-of-range slice panic).
    let offset = args
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    let offset = usize::try_from(offset).unwrap_or(usize::MAX);
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000);
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);

    let meta = fs::metadata(&full).map_err(|e| format!("{}: {e}", full.display()))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!(
            "{} is {} bytes (> {}); read it in chunks with offset/limit",
            full.display(),
            meta.len(),
            MAX_FILE_BYTES
        ));
    }

    let bytes = fs::read(&full).map_err(|e| format!("{}: {e}", full.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let end = offset.saturating_sub(1).saturating_add(limit).min(total);

    let mut out = String::new();
    if offset > total {
        return Ok(format!(
            "{} has {total} lines; offset {offset} is past the end",
            full.display()
        ));
    }
    let width = end.to_string().len();
    for (i, line) in lines[(offset - 1)..end].iter().enumerate() {
        let n = offset + i;
        out.push_str(&format!("{n:>width$}→{line}\n"));
    }
    if end < total {
        out.push_str(&format!("… {} more lines (total {total})\n", total - end));
    }
    Ok(out)
}

pub fn write_file(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let path = req_str(args, "path")?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `content`")?;
    let full = resolve(&ctx.root, path);
    super::check_write(ctx, &full)?;
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::write(&full, content).map_err(|e| format!("{}: {e}", full.display()))?;
    Ok(format!(
        "wrote {} bytes to {} ({} lines)",
        content.len(),
        full.display(),
        content.lines().count()
    ))
}

pub fn edit_file(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let path = req_str(args, "path")?;
    let old = req_str(args, "old_string")?;
    let new = args
        .get("new_string")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `new_string`")?;
    let all = args
        .get("replace_all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if old.is_empty() {
        return Err("`old_string` must not be empty; use write_file to create a file".to_string());
    }

    let full = resolve(&ctx.root, path);
    let original = fs::read_to_string(&full).map_err(|e| format!("{}: {e}", full.display()))?;

    let count = original.matches(old).count();
    if count == 0 {
        return Err(format!(
            "`old_string` not found in {} — read the file and match it exactly, including whitespace",
            full.display()
        ));
    }
    if count > 1 && !all {
        return Err(format!(
            "`old_string` occurs {count} times in {}; pass replace_all=true or provide more context",
            full.display()
        ));
    }

    let updated = if all {
        original.replace(old, new)
    } else {
        original.replacen(old, new, 1)
    };
    super::check_write(ctx, &full)?;
    fs::write(&full, &updated).map_err(|e| format!("{}: {e}", full.display()))?;

    Ok(format!(
        "edited {} ({} replacement{})",
        full.display(),
        if all { count } else { 1 },
        if all && count != 1 { "s" } else { "" }
    ))
}

pub fn list_dir(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let full = resolve(&ctx.root, path);
    let rd = fs::read_dir(&full).map_err(|e| format!("{}: {e}", full.display()))?;

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        match entry.file_type() {
            Ok(t) if t.is_dir() => dirs.push(format!("{name}/")),
            Ok(_) => {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                files.push(format!("{name}  ({})", human_size(size)));
            }
            Err(_) => files.push(name),
        }
    }
    dirs.sort();
    files.sort();

    if dirs.is_empty() && files.is_empty() {
        return Ok(format!("{} is empty", full.display()));
    }
    let mut out = format!("{}\n", full.display());
    for d in dirs {
        out.push_str(&d);
        out.push('\n');
    }
    for f in files {
        out.push_str(&f);
        out.push('\n');
    }
    Ok(out)
}

fn human_size(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing string field `{key}`"))
}

/// Exposed for reuse by the search module.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|&b| b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ApprovalPolicy, SandboxMode};
    use serde_json::json;

    fn ctx(root: &std::path::Path) -> Ctx {
        Ctx {
            root: root.to_path_buf(),
            auto_approve: true,
            approval: ApprovalPolicy::Never,
            sandbox: SandboxMode::Off,
            max_output: 20_000,
            tasks: crate::tools::tasks::TaskManager::new(),
            todos: crate::tools::todo::TodoList::new(),
            skills: Vec::new(),
            board: crate::board::Board::new(),
            gate: crate::board::Gate::new(1),
            memory_dir: None,
        }
    }

    fn fixture() -> (std::path::PathBuf, Ctx) {
        let dir = std::env::temp_dir().join(format!(
            "nanohermes-fs-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let ctx = ctx(&dir);
        (dir, ctx)
    }

    #[test]
    fn offset_and_limit_select_a_window() {
        let (dir, ctx) = fixture();
        let out = read_file(&json!({ "path": "a.txt", "offset": 2, "limit": 1 }), &ctx).unwrap();
        assert!(out.contains("two"), "got: {out}");
        assert!(!out.contains("one"), "got: {out}");
        assert!(out.contains("1 more lines"), "got: {out}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn huge_limit_does_not_panic() {
        let (dir, ctx) = fixture();
        let out = read_file(&json!({ "path": "a.txt", "limit": u64::MAX }), &ctx).unwrap();
        assert!(out.contains("one") && out.contains("three"), "got: {out}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_only_sandbox_blocks_writes_but_not_reads() {
        let (dir, mut ctx) = fixture();
        ctx.sandbox = SandboxMode::ReadOnly;
        let err = write_file(&json!({ "path": "a.txt", "content": "x" }), &ctx).unwrap_err();
        assert!(err.contains("read-only"), "got: {err}");
        let err = edit_file(
            &json!({ "path": "a.txt", "old_string": "one", "new_string": "ONE" }),
            &ctx,
        )
        .unwrap_err();
        assert!(err.contains("read-only"), "got: {err}");
        assert!(read_file(&json!({ "path": "a.txt" }), &ctx).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn workspace_write_sandbox_confines_writes_to_the_root() {
        let (dir, mut ctx) = fixture();
        ctx.sandbox = SandboxMode::WorkspaceWrite;
        assert!(write_file(&json!({ "path": "inside.txt", "content": "ok" }), &ctx).is_ok());

        let outside = std::env::temp_dir().join(format!(
            "nanohermes-outside-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let err = write_file(
            &json!({ "path": outside.to_string_lossy(), "content": "nope" }),
            &ctx,
        )
        .unwrap_err();
        assert!(err.contains("outside the workspace"), "got: {err}");
        assert!(!outside.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn huge_offset_reports_past_the_end() {
        let (dir, ctx) = fixture();
        let out = read_file(
            &json!({ "path": "a.txt", "offset": u64::MAX, "limit": u64::MAX }),
            &ctx,
        )
        .unwrap();
        assert!(out.contains("past the end"), "got: {out}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
