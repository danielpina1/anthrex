//! The end of a worktree and of a run (decision 20): salvaging a dirty worktree's work
//! to `refs/anthrex/salvage/<run>/<task>/<seq>` before it is removed, removing an
//! engine-owned (locked) worktree, deleting a run's branches, and accepting the run
//! branch into the base branch in the user's own checkout.
//!
//! Blocking; call only from `spawn_blocking`. Every function here writes, so each
//! belongs behind the caller's [`super::GitQueue::write`]. The engine-owned writes carry
//! decision 18's flags; [`accept`], the only write into the user's checkout, carries
//! only [`super::NO_HOOKS`] (decision 18, as amended by final fix batch F1).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::merge::{AcceptOutcome, read, unmerged};
use super::worktrees::{forget_missing, listed};
use super::{DIFF_FLAGS, Git, failure, os};

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
    // Whether this salvage is refused is decided before `add -A` touches the index
    // (ruling T9-m4): a refused salvage leaves a hand-back's unmerged entries unmerged.
    if read(g, worktree, reference)?.is_some() {
        return if matches_existing(g, worktree, reference)? {
            // The same salvage replayed (an op re-run after its `update-ref`).
            Ok(Some(reference.to_string()))
        } else {
            Err(format!("salvage ref {reference} already holds other work"))
        };
    }
    g.write(worktree, &[os("add"), os("-A")])?;
    let tree = g.write(worktree, &[os("write-tree")])?.trim().to_string();
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
    // An empty old value: the ref must not exist yet. The read above already refused an
    // existing one, so this guards only against another writer creating it since.
    g.write(
        worktree,
        &[os("update-ref"), os(reference), os(&commit), os("")],
    )?;
    Ok(Some(reference.to_string()))
}

/// Whether the worktree holds exactly the salvage `reference` already saved, read
/// without writing the index: no untracked (unignored) file, and no difference between
/// the working tree and the ref's tree over the paths either has. A first salvage's
/// `add -A` made every untracked file tracked, so a replay of it answers yes.
fn matches_existing(g: Git<'_>, worktree: &Path, reference: &str) -> Result<bool, String> {
    let others = [
        os("ls-files"),
        os("-z"),
        os("--others"),
        os("--exclude-standard"),
    ];
    let (output, kept) = g.read_head(worktree, &others, 0)?;
    if !output.success {
        return Err(failure(&others, &output));
    }
    if kept.total > 0 {
        return Ok(false);
    }
    let mut diff = vec![os("diff")];
    diff.extend(DIFF_FLAGS.map(os));
    diff.extend([os("--quiet"), os(reference), os("--")]);
    let output = g.read(worktree, &diff)?;
    if output.success {
        Ok(true)
    } else if output.stderr.trim().is_empty() {
        Ok(false)
    } else {
        Err(failure(&diff, &output))
    }
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
/// is `update-ref --no-deref -d <ref> <listed sha>`: a compare-and-swap that removes a
/// symbolic ref itself, never the branch it points at (ruling T9-m2).
///
/// A branch still checked out in any worktree is skipped, and returned: by the time
/// branches are deleted the engine's own worktrees are gone, so a worktree still on
/// one is the user's (they checked the run out to try it), and deleting the branch
/// would leave that checkout on an unborn branch.
pub fn delete_branches(
    git: &OsStr,
    root: &Path,
    prefix: &str,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        return Err("refusing to delete branches under an empty prefix".to_string());
    }
    let g = Git::new(git, timeout);
    let checked_out = checked_out_branches(g, root)?;
    let pattern = format!("refs/heads/{prefix}/");
    let listing = g.ok(
        root,
        &[
            os("for-each-ref"),
            os("--format=%(objectname) %(refname)"),
            os(&pattern),
        ],
    )?;
    let mut skipped = Vec::new();
    for line in listing.lines() {
        let Some((sha, refname)) = line.split_once(' ') else {
            continue;
        };
        if checked_out.iter().any(|branch| branch == refname) {
            skipped.push(refname.trim_start_matches("refs/heads/").to_string());
            continue;
        }
        g.write(
            root,
            &[
                os("update-ref"),
                os("--no-deref"),
                os("-d"),
                os(refname),
                os(sha),
            ],
        )?;
    }
    Ok(skipped)
}

/// The `refs/heads/…` every worktree of the repository has checked out.
fn checked_out_branches(g: Git<'_>, root: &Path) -> Result<Vec<String>, String> {
    let list = g.ok(
        root,
        &[os("worktree"), os("list"), os("--porcelain"), os("-z")],
    )?;
    Ok(list
        .split('\0')
        .filter_map(|field| field.strip_prefix("branch "))
        .map(str::to_string)
        .collect())
}

/// How long `run accept`'s own `git merge` may take. Longer than an engine git call's
/// `git_timeout`, because the user's signing runs inside it (decision 18): a signing
/// key waiting for a touch (fix round 1, ruling T9-I2). Their hooks no longer run there
/// (final fix batch F1, [`super::NO_HOOKS`]).
pub const ACCEPT_MERGE_TIMEOUT: Duration = Duration::from_secs(600);

/// Decision 20's accept, in `root`, the user's own checkout. Refused unless:
/// - no merge, cherry-pick, revert or rebase of the user's is in progress there (ruling
///   T9-I1), so accept never merges over one or aborts one;
/// - `base_branch` is checked out there and its tracked tree is clean;
/// - `refs/heads/<base_branch>` is still at `expected_base` (the recorded `base_sha`,
///   or the advanced head the user confirmed).
///
/// Then `git merge --no-ff --no-edit -m <message> refs/heads/<run branch>`, with
/// [`ACCEPT_MERGE_TIMEOUT`]. Whatever way that merge fails — a conflict (possible only
/// against an advanced base), a hook's refusal, or its deadline — a `MERGE_HEAD` it
/// left is aborted with a fresh deadline, so the base branch and `root` are as they
/// were; a conflict gives [`AcceptOutcome::Conflict`]. A merge whose first parent turns
/// out not to be `expected_base` (a commit landed between the check and the merge) is
/// undone with `git reset --keep HEAD^1`, keeping that commit, and refused as a base
/// that moved again (ruling T9-m1).
///
/// Not decision 18's [`super::WRITE_FLAGS`]: this is the user's checkout, and their
/// signing applies to their merge. Their hooks do not run ([`super::NO_HOOKS`], final
/// fix batch F1: a sandboxed worker could once plant one in the repository's hooks
/// directory).
pub fn accept(
    git: &OsStr,
    root: &Path,
    base_branch: &str,
    expected_base: &str,
    run_branch: &str,
    message: &str,
    timeout: Duration,
) -> Result<AcceptOutcome, String> {
    accept_with_merge_timeout(
        git,
        root,
        base_branch,
        expected_base,
        run_branch,
        message,
        ACCEPT_MERGE_TIMEOUT,
        timeout,
    )
}

/// [`accept`] with the merge's deadline given: the test seam for a merge that
/// outlives it, without a ten-minute test.
#[allow(clippy::too_many_arguments)]
pub fn accept_with_merge_timeout(
    git: &OsStr,
    root: &Path,
    base_branch: &str,
    expected_base: &str,
    run_branch: &str,
    message: &str,
    merge_timeout: Duration,
    timeout: Duration,
) -> Result<AcceptOutcome, String> {
    let g = Git::new(git, timeout);
    if let Some(kind) = operation_in_progress(g, root)? {
        return Err(format!(
            "a {kind} is in progress in {}; finish or abort it first",
            root.display()
        ));
    }
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
        return Err(MOVED_AGAIN.to_string());
    }
    let run_ref = format!("refs/heads/{run_branch}");
    let Some(run_head) = read(g, root, &run_ref)? else {
        return Err(format!("{run_ref} does not exist"));
    };

    let args = [
        os("merge"),
        os("-q"),
        os("--no-ff"),
        os("--no-edit"),
        os("-m"),
        os(message),
        os(&run_ref),
    ];
    // From here on, a `MERGE_HEAD` in `root` is this merge's: one of the user's would
    // have refused above.
    let output = match Git::new(git, merge_timeout).user_write(root, &args) {
        Ok(output) => output,
        Err(err) => {
            // Killed at its deadline (or never started): git may already have written
            // the merged index, the tree and `MERGE_HEAD` before the hook it was in.
            abort_own_merge(g, root, &err)?;
            return Err(err);
        }
    };
    if output.success {
        return check_first_parent(g, root, expected_base, &run_head);
    }
    let files = unmerged(g, root)?;
    abort_own_merge(g, root, &failure(&args, &output))?;
    if files.is_empty() {
        Err(failure(&args, &output))
    } else {
        Ok(AcceptOutcome::Conflict { files })
    }
}

const MOVED_AGAIN: &str = "the base branch moved again; run accept again";

/// Which of the user's own operations is in progress in `root`'s git directory (its
/// own, for a linked worktree), if any: `merge`, `cherry-pick`, `revert` or `rebase`.
fn operation_in_progress(g: Git<'_>, root: &Path) -> Result<Option<&'static str>, String> {
    let dir = g.ok(root, &[os("rev-parse"), os("--absolute-git-dir")])?;
    let dir = Path::new(dir.trim_end_matches('\n'));
    let markers = [
        ("MERGE_HEAD", "merge"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REVERT_HEAD", "revert"),
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
    ];
    Ok(markers
        .iter()
        .find(|(name, _)| dir.join(name).exists())
        .map(|(_, kind)| *kind))
}

/// Aborts the merge accept started, if it left `MERGE_HEAD`: `git merge --abort` with
/// a fresh deadline. If that fails too (a killed git can leave `index.lock`), the
/// error says what state `root` is in and what to run.
fn abort_own_merge(g: Git<'_>, root: &Path, cause: &str) -> Result<(), String> {
    if read(g, root, "MERGE_HEAD")?.is_none() {
        return Ok(());
    }
    let abort = [os("merge"), os("--abort")];
    let aborted = g.user_write(root, &abort);
    match aborted {
        Ok(output) if output.success => Ok(()),
        Ok(output) => Err(mid_merge(root, cause, &failure(&abort, &output))),
        Err(err) => Err(mid_merge(root, cause, &err)),
    }
}

fn mid_merge(root: &Path, cause: &str, abort_error: &str) -> String {
    format!(
        "{root} is mid-merge: accept's merge failed ({cause}) and could not be aborted \
         ({abort_error}); run git merge --abort in {root}",
        root = root.display()
    )
}

/// Ruling T9-m1: the merge commit accept just made must sit on `expected_base`. A
/// commit that landed on the base between the check and the merge makes it the first
/// parent instead; that merge is undone (`reset --keep HEAD^1`, which keeps the
/// user's commit and refuses to lose local changes) and refused.
fn check_first_parent(
    g: Git<'_>,
    root: &Path,
    expected_base: &str,
    run_head: &str,
) -> Result<AcceptOutcome, String> {
    let line = g.ok(
        root,
        &[
            os("rev-list"),
            os("--parents"),
            os("-n"),
            os("1"),
            os("HEAD"),
        ],
    )?;
    let mut shas = line.split_whitespace();
    let head = shas.next().unwrap_or_default().to_string();
    let parents: Vec<&str> = shas.collect();
    match parents.as_slice() {
        [first, second] if *second == run_head => {
            if *first == expected_base {
                return Ok(AcceptOutcome::Merged { commit: head });
            }
            let reset = [os("reset"), os("-q"), os("--keep"), os("HEAD^1")];
            let output = g.user_write(root, &reset)?;
            if !output.success {
                return Err(format!(
                    "the base branch moved during accept, and undoing the merge failed \
                     ({}); {} is on the merge commit {}",
                    failure(&reset, &output),
                    root.display(),
                    super::merge::short(&head)
                ));
            }
            Err(MOVED_AGAIN.to_string())
        }
        // Not a merge accept made: git found nothing to merge. Only a base still at
        // `expected_base` is the accepted state.
        _ if head == expected_base => Ok(AcceptOutcome::Merged { commit: head }),
        _ => Err(MOVED_AGAIN.to_string()),
    }
}
