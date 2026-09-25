//! Decision 53's reads of the base commit's tree: the project settings a headless
//! session would load unasked, and (final fix batch F2, C-I1) the `.codex` entries every
//! Codex session's checkout must still match. Blocking; call from `spawn_blocking`.

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::{Git, os, tracked_at};

/// The files Claude Code reads as project settings (decision 53).
const CLAUDE_SETTINGS: [&str; 2] = [".claude/settings.json", ".claude/settings.local.json"];
const CLAUDE_MCP: &str = ".mcp.json";

/// Decision 53: the project settings in the base commit's tree that a headless session
/// would load without asking. With `claude`, each tracked `.claude/settings.json` or
/// `.claude/settings.local.json` with a non-empty `hooks` key (or that is not valid
/// JSON, so cannot be shown to be hook-free) and a tracked `.mcp.json`; with
/// `codex_paths`, each of those paths that is tracked. Sorted by path.
pub fn project_settings(
    git: &OsStr,
    root: &Path,
    base_sha: &str,
    claude: bool,
    codex_paths: Option<&[&str]>,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let mut candidates: Vec<&str> = Vec::new();
    if claude {
        candidates.extend(CLAUDE_SETTINGS);
        candidates.push(CLAUDE_MCP);
    }
    if let Some(paths) = codex_paths {
        candidates.extend(paths.iter().copied());
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let g = Git::new(git, timeout);
    let mut found = Vec::new();
    for path in tracked_at(g, root, base_sha, &candidates)? {
        if !candidates.contains(&path.as_str()) {
            continue;
        }
        let codex = codex_paths.is_some_and(|paths| paths.contains(&path.as_str()));
        if claude && CLAUDE_SETTINGS.contains(&path.as_str()) && !codex {
            let blob = format!("{base_sha}:{path}");
            let text = g.ok(root, &[os("cat-file"), os("blob"), os(&blob)])?;
            if !has_hooks(&text) {
                continue;
            }
        }
        found.push(path);
    }
    found.sort();
    found.dedup();
    Ok(found)
}

/// Final fix batch F2 (C-I1): `base_sha`'s `.codex` entries, which every Codex session's
/// checkout must match (`headless::codex_guard`).
pub fn codex_config_tree(
    git: &OsStr,
    root: &Path,
    base_sha: &str,
    timeout: Duration,
) -> Result<Vec<crate::headless::codex_guard::GuardEntry>, String> {
    use crate::headless::codex_guard::{CODEX_DIR, parse_ls_tree};
    let args = [
        "ls-tree",
        "-r",
        "-z",
        "--full-tree",
        base_sha,
        "--",
        CODEX_DIR,
    ];
    let listing = Git::new(git, timeout).ok(root, &args.map(os))?;
    parse_ls_tree(&listing)
}

/// A settings file has hooks unless it parses and its `hooks` key is absent, `null`,
/// or an empty object or array.
fn has_hooks(text: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(text) {
        Err(_) => true,
        Ok(value) => match value.get("hooks") {
            None | Some(serde_json::Value::Null) => false,
            Some(serde_json::Value::Object(map)) => !map.is_empty(),
            Some(serde_json::Value::Array(items)) => !items.is_empty(),
            Some(_) => true,
        },
    }
}
