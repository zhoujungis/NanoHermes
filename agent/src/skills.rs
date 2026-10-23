//! Project skills: reusable instruction bundles discovered from `SKILL.md`
//! files. Skills use progressive disclosure — only the name and description are
//! injected into the system prompt; the full instructions are loaded by the
//! model on demand through the `skill` tool.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Cap on how many characters of a skill body are handed to the model.
const MAX_BODY_CHARS: usize = 20_000;

/// A discovered skill. `body` is read lazily so startup only pays for parsing
/// the frontmatter.
#[derive(Clone, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

/// Directories scanned for skills, most specific first (workspace wins over home).
pub fn roots(workspace: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = vec![workspace.join(".nanohermes").join("skills")];
    if let Some(home) = home {
        roots.push(home.join(".nanohermes").join("skills"));
    }
    roots
}

/// Discover every `<dir>/SKILL.md` under the workspace and `$HOME` skill roots.
pub fn discover(workspace: &Path) -> Vec<Skill> {
    let home = env::var_os("HOME").map(PathBuf::from);
    discover_in(workspace, home.as_deref())
}

/// Discover skills with an explicit home directory (used by tests, which must
/// not mutate the process-wide `$HOME`).
pub fn discover_in(workspace: &Path, home: Option<&Path>) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for root in roots(workspace, home) {
        let entries = match fs::read_dir(&root) {
            Ok(e) => e,
            Err(_) => continue, // no skills directory here
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let path = dir.join("SKILL.md");
            if !path.is_file() {
                continue;
            }
            let fallback = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("skill")
                .to_string();
            let (name, description) = match read_manifest(&path, &fallback) {
                Some(v) => v,
                None => continue,
            };
            if !seen.insert(name.clone()) {
                continue;
            }
            out.push(Skill {
                name,
                description,
                path,
            });
        }
    }
    out
}

/// Read at most `limit` bytes: SKILL.md comes from the workspace, so a huge file
/// must not be fully read before the size cap is applied.
fn read_head(path: &Path, limit: u64) -> Option<String> {
    use std::io::Read as _;
    let file = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(limit).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Read a skill's name and description from its frontmatter. Falls back to the
/// directory name and the first body line when the frontmatter is missing.
fn read_manifest(path: &Path, fallback_name: &str) -> Option<(String, String)> {
    let text = read_head(path, 64 * 1024)?;
    let (fields, body) = split_frontmatter(&text);
    let field = |key: &str| {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    let name = field("name").unwrap_or_else(|| fallback_name.to_string());
    let description = field("description").or_else(|| {
        body.lines()
            .map(|l| l.trim())
            .find(|l| !l.is_empty())
            .map(|l| l.to_string())
    })?;
    Some((name, description))
}

/// Read a skill's instructions (frontmatter stripped, size-capped).
pub fn load_body(path: &Path) -> Result<String, String> {
    let text = read_head(path, 64 * 1024)
        .ok_or_else(|| format!("{}: could not read skill", path.display()))?;
    let (_, body) = split_frontmatter(&text);
    let body = body.trim();
    if body.is_empty() {
        return Err(format!("skill {} has no instructions", path.display()));
    }
    let mut body = body.to_string();
    if body.len() > MAX_BODY_CHARS {
        let mut end = MAX_BODY_CHARS;
        while end > 0 && !body.is_char_boundary(end) {
            end -= 1;
        }
        body.truncate(end);
        body.push_str("\n… skill truncated");
    }
    Ok(body)
}

/// One-line-per-skill listing for the system prompt.
pub fn listing(skills: &[Skill]) -> String {
    let mut out = String::from(
        "Available skills — when one matches the task, call the `skill` tool with its name \
         to load the full instructions before proceeding:\n",
    );
    for s in skills {
        let desc = if s.description.is_empty() {
            "(no description)"
        } else {
            s.description.as_str()
        };
        out.push_str(&format!("- {}: {}\n", s.name, desc));
    }
    out
}

/// Split an optional `---`-delimited frontmatter block from the body. Returns
/// the parsed `key: value` fields (empty when there is no frontmatter) and the
/// remaining body.
fn split_frontmatter(text: &str) -> (Vec<(String, String)>, &str) {
    let rest = match text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    {
        Some(r) => r,
        None => return (Vec::new(), text),
    };

    let mut idx = 0usize;
    for line in rest.split_inclusive('\n') {
        if line.trim() == "---" {
            let fields = parse_fields(&rest[..idx]);
            return (fields, &rest[idx + line.len()..]);
        }
        idx += line.len();
    }
    // Unterminated frontmatter: treat the whole file as body.
    (Vec::new(), text)
}

fn parse_fields(frontmatter: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in frontmatter.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            let value = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            out.push((key.to_string(), value));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nanohermes-skills-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_skill(root: &Path, name: &str, contents: &str) {
        let dir = root.join(".nanohermes").join("skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), contents).unwrap();
    }

    #[test]
    fn discovers_skill_with_frontmatter() {
        let dir = scratch("discover");
        write_skill(
            &dir,
            "rustfmt",
            "---\nname: rustfmt\ndescription: Format Rust the project way.\n---\nRun `cargo fmt`.",
        );

        let skills = discover(&dir);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "rustfmt");
        assert_eq!(skills[0].description, "Format Rust the project way.");
        assert_eq!(load_body(&skills[0].path).unwrap(), "Run `cargo fmt`.");
        assert!(listing(&skills).contains("rustfmt"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn falls_back_to_directory_name_and_first_line() {
        let dir = scratch("fallback");
        write_skill(
            &dir,
            "release",
            "Cut a release with `cargo release`.\nMore detail.",
        );

        let skills = discover(&dir);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "release");
        assert_eq!(skills[0].description, "Cut a release with `cargo release`.");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn workspace_skill_shadows_home_skill() {
        let dir = scratch("shadow");
        let ws = dir.join("ws");
        write_skill(
            &ws,
            "lint",
            "---\nname: lint\ndescription: Workspace lint.\n---\nuse ws lint",
        );
        // Same name defined in the global (home) root.
        write_skill(
            &dir,
            "lint",
            "---\nname: lint\ndescription: Home lint.\n---\nuse home lint",
        );

        let skills = discover_in(&ws, Some(&dir));
        assert_eq!(
            skills.len(),
            1,
            "expected the shadowed home skill to be skipped"
        );
        assert_eq!(skills[0].description, "Workspace lint.");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_skills_directory_yields_nothing() {
        let dir = scratch("empty");
        assert!(discover(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
