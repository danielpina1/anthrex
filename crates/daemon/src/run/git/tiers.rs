//! Milestone 9.1's git reads for the tier executor (decisions 15, 16 and 30): the paths
//! a tier job's change touches and the tree it judges. Blocking; call only from
//! `spawn_blocking` (AGENTS.md rule 2). Every call goes through [`Git`], so
//! `--no-optional-locks`, the hooks-off read flags and the scrubbed environment hold
//! (AGENTS.md rules 10 and 11).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::{DIFF_FLAGS, Git, nul_fields, os};

/// `range`'s words, refused when empty or when one could be read as an option.
pub(super) fn range_words(range: &str) -> Result<Vec<&str>, String> {
    let words: Vec<&str> = range.split_whitespace().collect();
    if words.is_empty() || words.iter().any(|w| w.starts_with('-')) {
        return Err(format!("not a diff range: {range:?}"));
    }
    Ok(words)
}

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
    args.extend(range_words(range)?.into_iter().map(os));
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

/// Decision 37: `git show --stat --format=%h%x20%s <commit>` of a merge the engine
/// wrote, for a bisect fix task's brief (task M9.1.15); `None` when `commit` is not a
/// merge. The stat is against the first parent (`--diff-merges=first-parent`), the
/// stage's own line, so it lists what the merged task changed.
pub fn show_stat(
    git: &OsStr,
    dir: &Path,
    commit: &str,
    timeout: Duration,
) -> Result<Option<String>, String> {
    if commit.starts_with('-') {
        return Err(format!("not a commit: {commit:?}"));
    }
    let g = Git::new(git, timeout);
    let parents = g.ok(
        dir,
        &[
            os("rev-list"),
            os("--parents"),
            os("-n"),
            os("1"),
            os(commit),
        ],
    )?;
    if parents.split_whitespace().count() < 3 {
        return Ok(None);
    }
    let mut args = vec![os("show")];
    args.extend(DIFF_FLAGS.map(os));
    args.extend([
        os("--stat"),
        os("--diff-merges=first-parent"),
        os("--format=%h%x20%s"),
        os(commit),
        os("--"),
    ]);
    let out = g.ok(dir, &args)?;
    Ok(Some(out.trim_end().to_string()))
}

/// Controller ruling C-21 (3, 5): what a sync task's claim is checked against. `kept`:
/// its head still contains `onto`, the lower stage's head it merged. `upper`: the paths
/// the upper stage changed between `to_head` (where the task's worktree started) and
/// the latest head a later hand-back brought in, and that the head still holds as
/// `upper` does (ruling C-22): those are not the task's changes.
pub fn sync_read(
    git: &OsStr,
    worktree: &Path,
    head: &str,
    (onto, to_head, upper): (&str, &str, Option<&str>),
    timeout: Duration,
) -> Result<(bool, Vec<String>), String> {
    let g = Git::new(git, timeout);
    let kept = super::worktrees::is_ancestor(g, worktree, onto, head)?;
    // Ruling C-22 (3): only a path the head still holds exactly as `upper` has it is the
    // upper stage's; one the task changed since is its own.
    let paths = match upper {
        Some(upper) => {
            let brought = changed_paths(git, worktree, &format!("{to_head} {upper}"), timeout)?;
            let since = changed_paths(git, worktree, &format!("{upper} {head}"), timeout)?;
            brought.into_iter().filter(|p| !since.contains(p)).collect()
        }
        None => Vec::new(),
    };
    Ok((kept, paths))
}

/// Controller ruling C-21 (6): `git diff <tree> <head>`, clamped as a review diff is.
pub fn tree_patch(
    git: &OsStr,
    dir: &Path,
    tree: &str,
    head: &str,
    timeout: Duration,
) -> Result<String, String> {
    let words = format!("{tree} {head}");
    range_words(&words)?;
    super::diff(Git::new(git, timeout), dir, &words)
}
