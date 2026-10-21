use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::Config;

/// Cap on how many characters of project guidance are injected, so that a huge
/// AGENTS.md cannot blow up the context window.
const MAX_PROJECT_DOCS: usize = 20_000;
/// Guidance file names looked for in each directory. (`NANOHERMES.md` is handled
/// separately as project context, so it is not listed here.)
const DOC_NAMES: [&str; 1] = ["AGENTS.md"];

/// The agent's system prompt. Kept short on purpose: a compact prompt keeps the
/// context small, which is the main driver of memory use while streaming.
pub fn system_prompt(cfg: &Config) -> String {
    let mut out = format!(
        r#"You are NanoHermes, a lightweight terminal coding agent working inside the user's workspace.

You help with software engineering tasks: reading and understanding code, editing files,
running commands, searching the codebase, and answering questions about the project.

Operating rules:
- You have tools. Use them to inspect the code instead of guessing about file contents or command output.
- Always read a file before editing it. Never rewrite a file you have not read.
- Prefer targeted edits (edit_file) over whole-file rewrites; keep changes minimal and focused.
- Do not add features, refactors, or "improvements" beyond what the user asked for.
- Prefer non-interactive commands (pass auto-confirm flags) since this terminal has no TTY.
- Briefly explain what you are doing, then act. Do not over-narrate.
- After each tool result, decide the next step. When the task is complete, reply with a
  concise summary and stop calling tools.

Workspace root: {root}
"#,
        root = cfg.workspace.display()
    );

    if let Some(guide) = mode_guidance(cfg.permission_mode) {
        out.push_str("\nMode guidance: ");
        out.push_str(guide);
        out.push('\n');
    }

    // A user-selected persona adds to the built-in rules rather than replacing
    // them, so the safety and tool-usage guidance above is always in force.
    if let Some(extra) = cfg.system_prompt.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(
            "\nAdditional instructions from the user (a persona) — follow them on top of the rules above:\n",
        );
        out.push_str(extra);
        out.push('\n');
    }

    if cfg.project_docs {
        if let Some(docs) = project_docs(&cfg.workspace) {
            out.push_str(
                "\nProject guidance (from AGENTS.md files — follow it unless the user says otherwise):\n",
            );
            out.push_str(&docs);
            out.push('\n');
        }
    }
    out
}

/// A one-line hint describing the active permission mode, so the model behaves
/// according to how it was launched rather than only relying on tool gating.
fn mode_guidance(mode: crate::config::PermissionMode) -> Option<&'static str> {
    use crate::config::PermissionMode;
    match mode {
        PermissionMode::Goal => Some(
            "Goal mode — work autonomously until the objective is fully met. Every tool is \
             available and nothing needs confirmation; keep iterating, verify the result, then stop.",
        ),
        PermissionMode::Debug => Some(
            "Debug mode — diagnose and reproduce the reported problem. You may read files and \
             run commands, but you must not modify files or commit; stop once the root cause is clear.",
        ),
        PermissionMode::Plan => Some(
            "Plan mode — read-only analysis. Describe the change you would make and the files \
             involved; never edit files or run mutating commands.",
        ),
        PermissionMode::Default | PermissionMode::AcceptEdits => None,
    }
}

/// Collect project guidance from AGENTS.md files. Global guidance
/// (`~/.nanohermes/AGENTS.md`) is read first, then each directory from the
/// filesystem root down to the workspace, so the most specific file is
/// appended last and effectively takes precedence. Returns `None` when no
/// guidance file exists.
fn project_docs(workspace: &Path) -> Option<String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".nanohermes").join("AGENTS.md"));
    }

    // `ancestors()` yields the workspace first and the filesystem root last;
    // reverse it so outer directories are read before inner ones.
    let mut chain: Vec<&Path> = workspace.ancestors().collect();
    chain.reverse();
    for dir in chain {
        for name in DOC_NAMES {
            candidates.push(dir.join(name));
        }
    }

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut parts: Vec<String> = Vec::new();
    for file in candidates {
        // Dedupe identical files reached through different paths (e.g. symlinks).
        let key = std::fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
        if !seen.insert(key) {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&file) {
            if !text.trim().is_empty() {
                parts.push(format!("--- {} ---\n{}", file.display(), text.trim()));
            }
        }
    }
    if parts.is_empty() {
        return None;
    }

    let mut joined = parts.join("\n\n");
    if joined.len() > MAX_PROJECT_DOCS {
        let mut end = MAX_PROJECT_DOCS;
        while end > 0 && !joined.is_char_boundary(end) {
            end -= 1;
        }
        joined.truncate(end);
        joined.push_str("\n… project guidance truncated");
    }
    Some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a unique scratch directory for a test.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nanohermes-prompt-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn injects_agents_md_from_workspace() {
        let dir = scratch("workspace");
        std::fs::write(
            dir.join("AGENTS.md"),
            "Always run `cargo fmt` before committing.",
        )
        .unwrap();

        let cfg = Config {
            workspace: dir.clone(),
            ..Default::default()
        };
        let prompt = system_prompt(&cfg);
        assert!(prompt.contains("cargo fmt"), "prompt: {prompt}");
        assert!(prompt.contains("Project guidance"));
        assert!(prompt.contains("AGENTS.md"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn project_docs_can_be_disabled() {
        let dir = scratch("disabled");
        std::fs::write(dir.join("AGENTS.md"), "Never push to main.").unwrap();

        let cfg = Config {
            workspace: dir.clone(),
            project_docs: false,
            ..Default::default()
        };
        let prompt = system_prompt(&cfg);
        assert!(!prompt.contains("Never push to main"), "prompt: {prompt}");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prompt_introduces_itself_as_nanohermes() {
        // 模型自报家门时回显的就是这段身份，改回旧名字会让用户看到错误的项目名。
        let cfg = Config {
            workspace: scratch("identity"),
            ..Default::default()
        };
        let prompt = system_prompt(&cfg);
        assert!(
            prompt.starts_with("You are NanoHermes,"),
            "prompt: {prompt}"
        );
    }

    #[test]
    fn missing_guidance_is_omitted() {
        let dir = scratch("missing");
        let cfg = Config {
            workspace: dir.clone(),
            ..Default::default()
        };
        let prompt = system_prompt(&cfg);
        assert!(!prompt.contains("Project guidance"), "prompt: {prompt}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_persona_is_appended_without_replacing_the_base_prompt() {
        let cfg = Config {
            workspace: scratch("with-cfg"),
            system_prompt: Some("回答一律用中文，并在结尾给出下一步建议。".to_string()),
            ..Default::default()
        };
        let prompt = system_prompt(&cfg);
        assert!(prompt.starts_with("You are NanoHermes,"), "基底提示词仍在");
        assert!(prompt.contains("回答一律用中文"), "人设内容被追加");
        assert!(
            prompt.contains("Additional instructions from the user"),
            "追加段落要有明确说明"
        );
    }

    #[test]
    fn a_blank_persona_is_ignored() {
        for blank in ["", "   ", "\n\t "] {
            let cfg = Config {
                workspace: scratch("no-cfg"),
                system_prompt: Some(blank.to_string()),
                ..Default::default()
            };
            let prompt = system_prompt(&cfg);
            assert!(
                !prompt.contains("Additional instructions from the user"),
                "空白人设不该产生空段落：{blank:?}"
            );
        }
    }
}
