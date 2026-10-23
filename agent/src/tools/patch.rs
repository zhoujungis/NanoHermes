use std::fs;

use serde_json::Value;

use super::{resolve, Ctx};

/// Apply a unified diff to a file. The model supplies `path` and `diff`; the
/// diff is matched by content (not line numbers) so it stays robust against
/// small upstream drift.
pub fn apply_patch(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `path`")?;
    let diff = args
        .get("diff")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `diff`")?;
    if diff.trim().is_empty() {
        return Err("`diff` must not be empty".to_string());
    }

    let full = resolve(&ctx.root, path);
    let original = fs::read_to_string(&full).map_err(|e| format!("{}: {e}", full.display()))?;

    let updated = apply_unified_diff(&original, diff)?;
    if updated == original {
        return Ok("patch applied but file content unchanged".to_string());
    }
    super::check_write(ctx, &full)?;
    fs::write(&full, &updated).map_err(|e| format!("{}: {e}", full.display()))?;

    let added = updated.lines().count() as isize - original.lines().count() as isize;
    Ok(format!(
        "patched {} ({} hunk(s), line delta {:+})",
        full.display(),
        count_hunks(diff),
        added
    ))
}

fn count_hunks(diff: &str) -> usize {
    diff.lines().filter(|l| l.starts_with("@@ ")).count()
}

/// Line-based unified diff applier. Matches each hunk's old side against the
/// current file contents and replaces it with the new side.
pub fn apply_unified_diff(original: &str, diff: &str) -> Result<String, String> {
    let mut lines: Vec<String> = original.split('\n').map(|s| s.to_string()).collect();
    let mut diff_lines: Vec<&str> = diff.split('\n').collect();
    // Drop the empty element produced by the trailing newline: treating it as a
    // context line turns a pure-insertion hunk (`-N,0`) into a search for an
    // empty line, which lands the insertion at the wrong place.
    if diff.ends_with('\n') && diff_lines.last() == Some(&"") {
        diff_lines.pop();
    }

    let mut i = 0;
    let mut applied = 0;
    while i < diff_lines.len() {
        let header = diff_lines[i];
        if !header.starts_with("@@ ") {
            i += 1;
            continue;
        }
        i += 1; // skip the @@ header

        let mut old_block: Vec<String> = Vec::new();
        let mut new_block: Vec<String> = Vec::new();
        while i < diff_lines.len() {
            let dl = diff_lines[i];
            if dl.starts_with("@@ ")
                || dl.starts_with("diff ")
                || dl.starts_with("--- ")
                || dl.starts_with("+++ ")
            {
                break;
            }
            if dl.starts_with("\\ No newline") {
                i += 1;
                continue;
            }
            // An empty diff line represents an empty context line.
            if dl.is_empty() {
                old_block.push(String::new());
                new_block.push(String::new());
                i += 1;
                continue;
            }
            // Never slice at byte 1: diff context can legitimately start with a
            // multi-byte character (CJK, emoji, ideographic space) and `dl[1..]`
            // would panic (and, with panic=abort, kill the whole process).
            let mut chars = dl.chars();
            let prefix = chars.next().unwrap();
            let rest = chars.as_str().to_string();
            match prefix {
                ' ' => {
                    old_block.push(rest.clone());
                    new_block.push(rest);
                }
                '-' => old_block.push(rest),
                '+' => new_block.push(rest),
                _ => { /* stray line, ignore */ }
            }
            i += 1;
        }

        // A pure insertion (`-0,0` or `-N,0`) has no context to search for; the
        // old-side start line from the header is the insertion point.
        if old_block.is_empty() {
            let pos = hunk_old_start(header)
                .unwrap_or(lines.len())
                .min(lines.len());
            lines.splice(pos..pos, new_block);
            applied += 1;
            continue;
        }
        let pos = find_block(&lines, &old_block).ok_or_else(|| {
            format!(
                "hunk context not found in file: {}",
                old_block.first().map(|s| s.as_str()).unwrap_or("(empty)")
            )
        })?;
        lines.splice(pos..pos + old_block.len(), new_block);
        applied += 1;
    }

    if applied == 0 {
        return Err("no `@@` hunks found in the diff".to_string());
    }
    Ok(lines.join("\n"))
}

/// Old-side start line from a hunk header: `@@ -12,3 +12,4 @@` → 12.
fn hunk_old_start(header: &str) -> Option<usize> {
    let token = header.split_whitespace().find(|t| t.starts_with('-'))?;
    token.get(1..)?.split(',').next()?.parse().ok()
}

fn find_block(hay: &[String], needle: &[String]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    'outer: for start in 0..=hay.len() - needle.len() {
        for j in 0..needle.len() {
            if hay[start + j] != needle[j] {
                continue 'outer;
            }
        }
        return Some(start);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::apply_unified_diff;

    #[test]
    fn simple_patch() {
        let orig = "line one\nline two\nline three\n";
        let diff = "@@ -1,3 +1,3 @@\n line one\n-line two\n+line TWO\n line three\n";
        let out = apply_unified_diff(orig, diff).unwrap();
        assert_eq!(out, "line one\nline TWO\nline three\n");
    }

    #[test]
    fn insertion() {
        let orig = "a\nb\nc\n";
        let diff = "@@ -2,1 +2,2 @@\n b\n+inserted\n c\n";
        let out = apply_unified_diff(orig, diff).unwrap();
        assert_eq!(out, "a\nb\ninserted\nc\n");
    }

    #[test]
    fn two_hunks() {
        let orig = "a\nb\nc\nd\ne\n";
        let diff = "@@ -1,2 +1,2 @@\n-a\n+A\n b\n@@ -4,2 +4,2 @@\n d\n-e\n+E\n";
        let out = apply_unified_diff(orig, diff).unwrap();
        assert_eq!(out, "A\nb\nc\nd\nE\n");
    }

    #[test]
    fn pure_insertion_hunk() {
        // `-2,0` means "insert after line 2", with no context to search for.
        let orig = "a\nb\n";
        let diff = "@@ -2,0 +3,1 @@\n+new\n";
        let out = apply_unified_diff(orig, diff).unwrap();
        assert_eq!(out, "a\nb\nnew\n");
    }

    #[test]
    fn append_at_start() {
        let orig = "a\n";
        let diff = "@@ -0,0 +1,1 @@\n+first\n";
        let out = apply_unified_diff(orig, diff).unwrap();
        assert_eq!(out, "first\na\n");
    }

    #[test]
    fn non_ascii_context_does_not_panic() {
        let orig = "第一行\n第二行\n";
        let diff = "@@ -1,2 +1,2 @@\n 第一行\n-第二行\n+改过的行\n";
        let out = apply_unified_diff(orig, diff).unwrap();
        assert_eq!(out, "第一行\n改过的行\n");
    }

    #[test]
    fn missing_context_errors() {
        let orig = "hello\nworld\n";
        let diff = "@@ -1,1 +1,1 @@\n-nope\n+yes\n";
        assert!(apply_unified_diff(orig, diff).is_err());
    }
}
