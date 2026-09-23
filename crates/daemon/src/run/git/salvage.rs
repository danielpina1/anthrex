//! The end of a worktree and of a run (decision 20): salvaging a dirty worktree's work
//! to `refs/anthrex/salvage/<run>/<task>/<seq>` before it is removed, removing an
//! engine-owned (locked) worktree, deleting a run's branches, and accepting the run
//! branch into the base branch in the user's own checkout.
//!
//! Blocking; call only from `spawn_blocking`. Every function here writes, so each
//! belongs behind the caller's [`super::GitQueue::write`]. The engine-owned writes carry
//! decision 18's flags; [`accept`], the only write into the user's checkout, carries
//! neither (decision 18).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::merge::{AcceptOutcome, read, unmerged};
use super::worktrees::{forget_missing, listed};
use super::{Git, failure, os};

/// Decision 20's salvage. A worktree with nothing to save (`git status --porcelain`,
/// untracked files included, ignored ones not) writes nothing and gives `None`.
/// Otherwise its whole state — staged, unstaged and untracked, and a conflicted
/// merge's markers — is committed on top of `HEAD` with `message`, without moving any
/// branch (`add -A`, `write-tree`, `commit-tree -p HEAD`), and `reference` is created
/// at that commit; the result is `Some(reference)`.
///
/// A salvage ref is never overwritten. If `reference` already exists and holds the
/// same tree, this is the same salvage replayed (an op re-run after a crash) and gives
/// `Some(reference)` again; if it holds other work, this is an error, and the caller
/// should have taken the next `<seq>`.
pub fn salvage(
    git: &OsStr,
    worktree: &Path,
    reference: &str,
    message: &str,
    timeout: Duration,
) -> Result<Option<String>, String> {
    let g = Git::new(git, timeout);
    // `--untracked-files=normal` explicitly: a user's `status.showUntrackedFiles=no`
    // must not hide new files from the salvage.
    let args = [
        os("status"),
        os("--porcelain"),
        os("-z"),
        os("--untracked-files=normal"),
    ];
    // Only whether there is any output matters, so none of it is held: a worktree with
    // a vast untracked build tree is still salvaged, not failed over a cap.
    let (output, kept) = g.read_head(worktree, &args, 0)?;
    if !output.success {
        return Err(failure(&args, &output));
    }
    if kept.total == 0 {
        return Ok(None);
    }
    g.write(worktree, &[os("add"), os("-A")])?;
    let tree = g.write(worktree, &[os("write-tree")])?.trim().to_string();
    if let Some(existing) = read(g, worktree, reference)? {
        let spec = format!("{existing}^{{tree}}");
        let held = g.ok(worktree, &[os("rev-parse"), os(&spec)])?;
        return if held.trim() == tree {
            Ok(Some(reference.to_string()))
        } else {
            Err(format!("salvage ref {reference} already holds other work"))
        };
    }
    let commit = g
        .write(
            worktree,
            &[
                os("commit-tree"),
                os(&tree),
                os("-p"),
                os("HEAD"),
                os("-m"),
                os(message),
            ],
        )?
        .trim()
        .to_string();
    // An empty old value: the ref must not exist yet.
    g.write(
        worktree,
        &[os("update-ref"), os(reference), os(&commit), os("")],
    )?;
    Ok(Some(reference.to_string()))
}

/// Decision 20's removal: `git worktree unlock`, `git worktree remove --force`, `git
/// worktree prune`. It removes whatever is there, dirty or not, so the caller salvages
/// first. A worktree git no longer lists, or whose directory is already gone, is not
/// an error: the removal is already done, or is finished by the prune.
pub fn remove_worktree(
    git: &OsStr,
    root: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<(), String> {
    let g = Git::new(git, timeout);
    match listed(g, root, path)? {
        None => {}
        Some(entry) if !path.exists() => forget_missing(g, root, path, &entry)?,
        Some(entry) => {
            if entry.locked {
                g.write(root, &[os("worktree"), os("unlock"), path.as_os_str()])?;
            }
            g.write(
                root,
                &[
                    os("worktree"),
                    os("remove"),
                    os("--force"),
                    path.as_os_str(),
                ],
            )?;
        }
    }
    g.write(root, &[os("worktree"), os("prune")])?;
    Ok(())
}

/// Decision 20's discard and accept: delete every branch under `refs/heads/<prefix>/`
/// (`anthrex/<run>/`, with or without its trailing slash, so `anthrex/r1` never reaches
/// `anthrex/r10/…`). Salvage refs live outside `refs/heads` and are kept. Each delete
/// is a compare-and-swap on the value just listed.
pub fn delete_branches(
    git: &OsStr,
    root: &Path,
    prefix: &str,
    timeout: Duration,
) -> Result<(), String> {
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        return Err("refusing to delete branches under an empty prefix".to_string());
    }
    let g = Git::new(git, timeout);
    let pattern = format!("refs/heads/{prefix}/");
    let listing = g.ok(
        root,
        &[
            os("for-each-ref"),
            os("--format=%(objectname) %(refname)"),
            os(&pattern),
        ],
    )?;
    for line in listing.lines() {
        let Some((sha, refname)) = line.split_once(' ') else {
            continue;
        };
        g.write(root, &[os("update-ref"), os("-d"), os(refname), os(sha)])?;
    }
    Ok(())
}

/// Decision 20's accept, in `root`, the user's own checkout: `base_branch` must be
/// checked out there, its tracked tree clean, and `refs/heads/<base_branch>` still at
/// `expected_base` (the recorded `base_sha`, or the advanced head the user confirmed).
/// Then `git merge --no-ff --no-edit -m <message> <run branch>`. A conflict (possible
/// only against an advanced base) is aborted with `git merge --abort`, leaving the base
/// branch and `root` as they were, and gives [`AcceptOutcome::Conflict`].
///
/// No decision 18 flags: this is the user's checkout, and their hooks and signing
/// apply to their merge.
pub fn accept(
    git: &OsStr,
    root: &Path,
    base_branch: &str,
    expected_base: &str,
    run_branch: &str,
    message: &str,
    timeout: Duration,
) -> Result<AcceptOutcome, String> {
    let g = Git::new(git, timeout);
    let head_ref = g.read(root, &[os("symbolic-ref"), os("-q"), os("HEAD")])?;
    let current = head_ref
        .stdout
        .trim()
        .strip_prefix("refs/heads/")
        .filter(|_| head_ref.success)
        .map(str::to_string);
    if current.as_deref() != Some(base_branch) {
        return Err(format!(
            "check out {base_branch} in {} first (currently {})",
            root.display(),
            current.as_deref().unwrap_or("a detached HEAD")
        ));
    }
    let status = [os("status"), os("--porcelain"), os("--untracked-files=no")];
    let (output, kept) = g.read_head(root, &status, 0)?;
    if !output.success {
        return Err(failure(&status, &output));
    }
    if kept.total > 0 {
        return Err(format!(
            "the working tree at {} has uncommitted changes; commit or stash them first",
            root.display()
        ));
    }
    let base_ref = format!("refs/heads/{base_branch}");
    if read(g, root, &base_ref)?.as_deref() != Some(expected_base) {
        return Err("the base branch moved again; run accept again".to_string());
    }

    let run_ref = format!("refs/heads/{run_branch}");
    let args = [
        os("merge"),
        os("-q"),
        os("--no-ff"),
        os("--no-edit"),
        os("-m"),
        os(message),
        os(&run_ref),
    ];
    let output = g.user_write(root, &args)?;
    if output.success {
        let head = g.ok(root, &[os("rev-parse"), os("HEAD")])?;
        return Ok(AcceptOutcome::Merged {
            commit: head.trim().to_string(),
        });
    }
    let files = unmerged(g, root)?;
    let merging = read(g, root, "MERGE_HEAD")?.is_some();
    if merging {
        let abort = [os("merge"), os("--abort")];
        let aborted = g.user_write(root, &abort)?;
        if !aborted.success {
            return Err(failure(&abort, &aborted));
        }
    }
    if files.is_empty() {
        Err(failure(&args, &output))
    } else {
        Ok(AcceptOutcome::Conflict { files })
    }
}
