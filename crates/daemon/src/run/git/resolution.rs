//! Ruling T14-R2 (the reviewer's rule for decision 36): whether a claim after a
//! conflicted hand-back is the conflict's resolution and nothing more. Only such a
//! claim goes straight back to the merge queue; any other passes every gate.

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::{DIFF_FLAGS, Git, LARGE_OUTPUT_BYTES, failure, nul_fields, os};

/// True when `head` is exactly one merge commit on top of the hand-back, with parents
/// exactly `(onto, run_head)` in that order (`git rev-list --parents -n 1`), and its tree differs from git's own automatic merge of the two
/// (`merge-tree --write-tree onto run_head`, conflict markers included) only in
/// `files`, the paths that conflicted. Reads only; `worktree` is any directory of the
/// repository.
pub fn resolution_only(
    git: &OsStr,
    worktree: &Path,
    head: &str,
    onto: &str,
    run_head: &str,
    files: &[String],
    timeout: Duration,
) -> Result<bool, String> {
    let g = Git::new(git, timeout);
    let parents = g.ok(
        worktree,
        &[
            os("rev-list"),
            os("--parents"),
            os("-n"),
            os("1"),
            os(head),
            os("--"),
        ],
    )?;
    let parents: Vec<&str> = parents.split_whitespace().skip(1).collect();
    if parents != [onto, run_head] {
        return Ok(false);
    }
    let args = [
        os("merge-tree"),
        os("--write-tree"),
        os("--name-only"),
        os("--no-messages"),
        os("-z"),
        os(onto),
        os(run_head),
    ];
    let (output, kept) = g.read_head(worktree, &args, LARGE_OUTPUT_BYTES)?;
    let text = String::from_utf8_lossy(&kept.head);
    let Some(tree) = nul_fields(&text).next().filter(|t| t.len() >= 40) else {
        return Err(failure(&args, &output));
    };
    let mut diff = vec![os("diff")];
    diff.extend(DIFF_FLAGS.map(os));
    diff.extend([
        os("--no-renames"),
        os("--name-only"),
        os("-z"),
        os(tree),
        os(head),
        os("--"),
    ]);
    let changed = g.ok(worktree, &diff)?;
    Ok(nul_fields(&changed).all(|path| files.iter().any(|f| f == path)))
}
