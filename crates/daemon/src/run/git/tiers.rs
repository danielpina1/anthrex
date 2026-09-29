//! Milestone 9.1's git reads for the tier executor (decisions 15, 16 and 30): the paths
//! a tier job's change touches and the tree it judges. Blocking; call only from
//! `spawn_blocking` (AGENTS.md rule 2). Every call goes through [`Git`], so
//! `--no-optional-locks`, the hooks-off read flags and the scrubbed environment hold
//! (AGENTS.md rules 10 and 11).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::{DIFF_FLAGS, Git, nul_fields, os};

/// `git diff --name-only -z --no-renames <range>` in `dir`, where `range` is decision
/// 15's three-dot `<stage head>...<task head>` (tier 1) or decision 16's two commits
/// `<stage head> <candidate>` (tier 2, given as two words). The paths in git's order.
pub fn changed_paths(
    git: &OsStr,
    dir: &Path,
    range: &str,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let g = Git::new(git, timeout);
    let mut args = vec![os("diff")];
    args.extend(DIFF_FLAGS.map(os));
    args.extend([os("--name-only"), os("-z"), os("--no-renames")]);
    let words: Vec<&str> = range.split_whitespace().collect();
    if words.is_empty() || words.iter().any(|w| w.starts_with('-')) {
        return Err(format!("not a diff range: {range:?}"));
    }
    args.extend(words.iter().map(|w| os(w)));
    args.push(os("--"));
    let out = g.ok(dir, &args)?;
    Ok(nul_fields(&out).map(str::to_string).collect())
}

/// The tree id of `commit` (`git rev-parse --verify <commit>^{tree}`), decision 30's key.
pub fn tree_of(git: &OsStr, dir: &Path, commit: &str, timeout: Duration) -> Result<String, String> {
    if commit.starts_with('-') {
        return Err(format!("not a commit: {commit:?}"));
    }
    let g = Git::new(git, timeout);
    let spec = format!("{commit}^{{tree}}");
    let out = g.ok(dir, &[os("rev-parse"), os("--verify"), os("-q"), os(&spec)])?;
    let tree = out.trim();
    if tree.is_empty() {
        return Err(format!("{commit} has no tree"));
    }
    Ok(tree.to_string())
}

/// The tree the checkout `dir` holds, when it holds exactly its `HEAD`: `HEAD^{tree}`
/// when `git status --porcelain --untracked-files=no` is empty, `None` when a tracked
/// file differs (milestone 9.1 ruling C-13 (1): a result is cached only for the tree
/// it ran on).
pub fn checkout_tree(git: &OsStr, dir: &Path, timeout: Duration) -> Result<Option<String>, String> {
    let g = Git::new(git, timeout);
    let status = g.ok(
        dir,
        &[
            os("status"),
            os("--porcelain"),
            os("--untracked-files=no"),
            os("-z"),
        ],
    )?;
    if !status.is_empty() {
        return Ok(None);
    }
    tree_of(git, dir, "HEAD", timeout).map(Some)
}
