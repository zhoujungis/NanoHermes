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
    let diff_lines: Vec<&str> = diff.split('\n').collect();

    let mut i = 0;
    let mut applied = 0;
    while i < diff_lines.len() {
        let l = diff_lines[i];
        if !l.starts_with("@@ ") {
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
            let prefix = dl.chars().next().unwrap();
            let rest = dl[1..].to_string();
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
    fn missing_context_errors() {
        let orig = "hello\nworld\n";
        let diff = "@@ -1,1 +1,1 @@\n-nope\n+yes\n";
        assert!(apply_unified_diff(orig, diff).is_err());
    }
}
