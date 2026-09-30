//! Milestone 9.1's git reads for the tier executor (decisions 15, 16 and 30): the paths
//! a tier job's change touches and the tree it judges. Blocking; call only from
//! `spawn_blocking` (AGENTS.md rule 2). Every call goes through [`Git`], so
//! `--no-optional-locks`, the hooks-off read flags and the scrubbed environment hold
//! (AGENTS.md rules 10 and 11).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::{DIFF_FLAGS, Git, PATCH_PREFIXES, nul_fields, os};

/// At most this much of decision 40's `-U0` diff is read; the rest is read and dropped
/// ([`unified_zero`] says it was cut).
pub const SIGNALS_DIFF_BYTES: usize = 16 * 1024 * 1024;

/// At most this many paths per `git grep` of [`cfg_test_files`], so no argument list
/// grows past the system's limit.
const GREP_PATHS_MAX: usize = 256;

/// `range`'s words, refused when empty or when one could be read as an option.
fn range_words(range: &str) -> Result<Vec<&str>, String> {
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

/// Decision 40: `git diff -U0 --no-renames --no-color <range>` in `dir` (with
/// [`DIFF_FLAGS`] and the `a/`/`b/` prefixes), where `range` is decision 15's
/// `<stage head>...<task head>`. At most [`SIGNALS_DIFF_BYTES`] are kept; the flag is
/// `true` when more were dropped.
pub fn unified_zero(
    git: &OsStr,
    dir: &Path,
    range: &str,
    timeout: Duration,
) -> Result<(String, bool), String> {
    let g = Git::new(git, timeout);
    let mut args = vec![os("diff")];
    args.extend(DIFF_FLAGS.map(os));
    args.extend(PATCH_PREFIXES.map(os));
    args.extend([os("-U0"), os("--no-renames")]);
    args.extend(range_words(range)?.into_iter().map(os));
    args.push(os("--"));
    let (output, kept) = g.read_head(dir, &args, SIGNALS_DIFF_BYTES)?;
    if !output.success {
        return Err(super::failure(&args, &output));
    }
    Ok((
        String::from_utf8_lossy(&kept.head).into_owned(),
        kept.dropped(),
    ))
}

/// The files `range` deletes (`git diff --name-only -z --no-renames --diff-filter=D`),
/// for a `-U0` diff that [`unified_zero`] cut.
pub fn deleted_paths(
    git: &OsStr,
    dir: &Path,
    range: &str,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let g = Git::new(git, timeout);
    let mut args = vec![os("diff")];
    args.extend(DIFF_FLAGS.map(os));
    args.extend([
        os("--name-only"),
        os("-z"),
        os("--no-renames"),
        os("--diff-filter=D"),
    ]);
    args.extend(range_words(range)?.into_iter().map(os));
    args.push(os("--"));
    let out = g.ok(dir, &args)?;
    Ok(nul_fields(&out).map(str::to_string).collect())
}

/// Decision 40: which of `files` hold `#[cfg(test)]` at `head`
/// (`git grep -l -z -F '#[cfg(test)]' <head> -- <files>`, each path literal), in
/// git's order. `files` are read in groups of [`GREP_PATHS_MAX`].
pub fn cfg_test_files(
    git: &OsStr,
    dir: &Path,
    head: &str,
    files: &[String],
    timeout: Duration,
) -> Result<Vec<String>, String> {
    if head.starts_with('-') {
        return Err(format!("not a commit: {head:?}"));
    }
    let g = Git::new(git, timeout);
    let prefix = format!("{head}:");
    let mut found = Vec::new();
    for group in files.chunks(GREP_PATHS_MAX) {
        let literal: Vec<String> = group.iter().map(|f| format!(":(literal){f}")).collect();
        let mut args = vec![
            os("grep"),
            os("-l"),
            os("-z"),
            os("-F"),
            os("--no-recurse-submodules"),
            os("-e"),
            os("#[cfg(test)]"),
            os(head),
            os("--"),
        ];
        args.extend(literal.iter().map(|f| os(f)));
        let output = g.read(dir, &args)?;
        // `git grep` exits 1, with nothing on stderr, when nothing matches.
        if !output.success && !output.stderr.trim().is_empty() {
            return Err(super::failure(&args, &output));
        }
        found.extend(
            nul_fields(&output.stdout)
                .map(|name| name.strip_prefix(&prefix).unwrap_or(name).to_string()),
        );
    }
    Ok(found)
}
