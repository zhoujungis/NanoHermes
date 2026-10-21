use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{fs as fs_tool, resolve, Ctx};

const MAX_MATCHES: usize = 200;
const MAX_FILES: usize = 500;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_LINE_CHARS: usize = 240;

pub fn grep(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `pattern`")?;
    if pattern.is_empty() {
        return Err("`pattern` must not be empty".to_string());
    }
    let include = args.get("include").and_then(|v| v.as_str());
    let ci = args
        .get("case_insensitive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let base = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() && p != "." => resolve(&ctx.root, p),
        _ => ctx.root.clone(),
    };

    let needle = if ci {
        pattern.to_lowercase()
    } else {
        pattern.to_string()
    };

    let mut out = String::new();
    let mut hits = 0usize;
    let mut truncated = false;

    for_each_file(&base, |path, rel| {
        if truncated {
            return;
        }
        if let Some(inc) = include {
            if !path_matches(inc, rel) {
                return;
            }
        }
        let Ok(meta) = fs::metadata(path) else { return };
        if meta.len() > MAX_FILE_BYTES {
            return;
        }
        let Ok(bytes) = fs::read(path) else { return };
        if fs_tool::is_binary(&bytes) {
            return;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (i, line) in text.lines().enumerate() {
            let hay = if ci {
                line.to_lowercase()
            } else {
                line.to_string()
            };
            if hay.contains(&needle) {
                out.push_str(&format!("{}:{}:{}\n", rel, i + 1, clip(line)));
                hits += 1;
                if hits >= MAX_MATCHES {
                    truncated = true;
                    return;
                }
            }
        }
    });

    if hits == 0 {
        return Ok(format!("no matches for `{pattern}`"));
    }
    if truncated {
        out.push_str(&format!("… stopped after {MAX_MATCHES} matches\n"));
    }
    Ok(out)
}

pub fn glob(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `pattern`")?;
    if pattern.is_empty() {
        return Err("`pattern` must not be empty".to_string());
    }

    let mut matches: Vec<String> = Vec::new();
    for_each_file(&ctx.root, |_path, rel| {
        if matches.len() < MAX_FILES && glob_match(pattern, rel) {
            matches.push(rel.to_string());
        }
    });

    if matches.is_empty() {
        return Ok(format!("no files match `{pattern}`"));
    }
    matches.sort();
    let mut out = matches.join("\n");
    if matches.len() >= MAX_FILES {
        out.push_str(&format!("\n… capped at {MAX_FILES} files"));
    }
    Ok(out)
}

/// Walk the tree iteratively (no recursion) and call `f` for every regular file.
/// Build/tool directories are skipped so results stay relevant.
fn for_each_file(root: &Path, mut f: impl FnMut(&Path, &str)) {
    let (base, single) = if root.is_file() {
        let parent = root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        (parent, Some(root.to_path_buf()))
    } else {
        (root.to_path_buf(), None)
    };

    let mut stack = vec![base.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        // Collect first so we don't hold the read_dir handle while descending.
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if ft.is_dir() {
                if !is_ignored_dir(&entry.file_name().to_string_lossy()) {
                    dirs.push(path);
                }
            } else if ft.is_file() {
                files.push(path);
            }
        }
        for p in files {
            if let Some(only) = &single {
                if p != *only {
                    continue;
                }
            }
            let rel = p
                .strip_prefix(&base)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            f(&p, &rel);
        }
        stack.extend(dirs);
    }
}

fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "target"
            | "node_modules"
            | "__pycache__"
            | ".venv"
            | "venv"
            | "dist"
            | "build"
            | ".idea"
            | ".cache"
            | ".next"
            | "vendor"
            | ".tox"
            | ".mypy_cache"
            | ".ruff_cache"
            | ".pytest_cache"
    )
}

/// Match an `include` glob against either the relative path or the file name.
fn path_matches(pattern: &str, rel: &str) -> bool {
    if glob_match(pattern, rel) {
        return true;
    }
    let name = rel.rsplit('/').next().unwrap_or(rel);
    glob_match(pattern, name)
}

/// Glob matcher with `*` (within a path segment), `?` and `**` (across segments).
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pats: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match_segments(&pats, &segs)
}

fn match_segments(pats: &[&str], segs: &[&str]) -> bool {
    match pats.split_first() {
        None => segs.is_empty(),
        Some((&"**", rest)) => {
            // `**` may consume zero or more path segments.
            (0..=segs.len()).any(|i| match_segments(rest, &segs[i..]))
        }
        Some((head, rest)) => match segs.split_first() {
            None => false,
            Some((seg, seg_rest)) => {
                wildcard_match(head.as_bytes(), seg.as_bytes()) && match_segments(rest, seg_rest)
            }
        },
    }
}

/// Classic wildcard match with backtracking for `*`.
fn wildcard_match(pat: &[u8], s: &[u8]) -> bool {
    let (mut p, mut i) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while i < s.len() {
        if p < pat.len() && (pat[p] == b'?' || pat[p] == s[i]) {
            p += 1;
            i += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star = Some((p, i));
            p += 1;
        } else if let Some((sp, si)) = star {
            p = sp + 1;
            i = si + 1;
            star = Some((sp, si + 1));
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

fn clip(line: &str) -> String {
    let t = line.trim_end();
    if t.len() <= MAX_LINE_CHARS {
        return t.to_string();
    }
    let mut end = MAX_LINE_CHARS;
    while end > 0 && !t.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &t[..end])
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn globs() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(!glob_match("*.rs", "src/main.rs"));
        assert!(glob_match("src/**/*.rs", "src/a/b/main.rs"));
        assert!(glob_match("src/**/*.rs", "src/main.rs"));
        assert!(glob_match("**/*.rs", "a/b/c.rs"));
        assert!(glob_match("a?c.rs", "abc.rs"));
        assert!(!glob_match("a?c.rs", "abbc.rs"));
        assert!(glob_match("src/**/test?.rs", "src/x/y/test1.rs"));
    }
}
