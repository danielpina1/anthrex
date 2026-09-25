//! Decision 20's accept: the run branch merged into the base branch in the user's own
//! checkout (split from `salvage.rs` in final fix batch F1c).
//!
//! Blocking; call only from `spawn_blocking`, behind the caller's
//! [`super::GitQueue::write`]. It carries only [`super::NO_HOOKS`] (decision 18, as
//! amended by final fix batch F1).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::merge::{AcceptOutcome, read, short};
use super::merge_state::unmerged;
use super::worktrees::is_ancestor;
use super::{Git, NO_NESTED, failure, os};

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
///   or the advanced head the user confirmed);
/// - `refs/heads/<run_branch>` is still at `expected_run_head`, the run head the engine
///   verified (final fix batch F1, finding D-6), refused otherwise with decision 21's
///   `refs/heads/<run_branch> moved from <old7> to <new7>`.
///
/// Then `git merge --no-ff --no-edit -m <message> <expected_run_head>` (the verified
/// commit itself, not whatever the branch holds by then), with [`ACCEPT_MERGE_TIMEOUT`].
/// A merge commit with parents exactly `(expected_base, expected_run_head)` on the base
/// is [`AcceptOutcome::Merged`], however git exited, even killed at its deadline (D-3).
/// Otherwise, whatever way that merge fails — a conflict (possible only against an
/// advanced base), a refusal, or its deadline — a `MERGE_HEAD` it left is aborted with a
/// fresh deadline, so the base branch and `root` are as they were; a conflict gives
/// [`AcceptOutcome::Conflict`]. A base already containing the run head (the user merged
/// it by hand, as decision 20's conflict text advises) is merged, and left exactly as it
/// is (D-1). A merge accept itself made whose first parent is not `expected_base` (a
/// commit landed between the check and the merge) is undone with `git reset --keep
/// HEAD^1`, keeping that commit, and refused as a base that moved again (ruling T9-m1);
/// no other commit is ever undone.
///
/// Not decision 18's [`super::WRITE_FLAGS`]: this is the user's checkout, and their
/// signing applies to their merge. Their hooks do not run ([`super::NO_HOOKS`], final
/// fix batch F1: a sandboxed worker could once plant one in the repository's hooks
/// directory).
#[allow(clippy::too_many_arguments)]
pub fn accept(
    git: &OsStr,
    root: &Path,
    base_branch: &str,
    expected_base: &str,
    run_branch: &str,
    expected_run_head: &str,
    message: &str,
    timeout: Duration,
) -> Result<AcceptOutcome, String> {
    accept_with_merge_timeout(
        git,
        root,
        base_branch,
        expected_base,
        run_branch,
        expected_run_head,
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
    expected_run_head: &str,
    message: &str,
    merge_timeout: Duration,
    timeout: Duration,
) -> Result<AcceptOutcome, String> {
    // Fix round 1, review N5: the message exactly as git will store it, so accept can
    // recognise its own merge by it.
    let message = &cleaned(message);
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
    let status = [
        os("status"),
        os("--porcelain"),
        os("--untracked-files=no"),
        os(NO_NESTED),
    ];
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
    match read(g, root, &run_ref)? {
        Some(head) if head == expected_run_head => {}
        Some(head) => {
            return Err(format!(
                "{run_ref} moved from {} to {}",
                short(expected_run_head),
                short(&head)
            ));
        }
        None => return Err(format!("{run_ref} does not exist")),
    }
    let run_head = expected_run_head;

    let args = [
        os("merge"),
        os("-q"),
        os("--no-ff"),
        os("--no-edit"),
        os("-m"),
        os(message),
        os(run_head),
    ];
    // From here on, a `MERGE_HEAD` in `root` is this merge's: one of the user's would
    // have refused above.
    let output = match Git::new(git, merge_timeout).user_write(root, &args) {
        Ok(output) => output,
        Err(err) => {
            // D-3: killed at its deadline after the merge commit was made (post-merge
            // work, or a process holding its output open): that is a merge.
            if let Some(commit) = landed(g, root, &base_ref, expected_base, run_head)? {
                return Ok(AcceptOutcome::Merged { commit });
            }
            // Killed before that (or never started): git may already have written the
            // merged index, the tree and `MERGE_HEAD`.
            abort_own_merge(g, root, &err)?;
            return Err(err);
        }
    };
    if output.success {
        return check_first_parent(g, root, expected_base, run_head, message);
    }
    if let Some(commit) = landed(g, root, &base_ref, expected_base, run_head)? {
        return Ok(AcceptOutcome::Merged { commit });
    }
    let files = unmerged(g, root)?;
    abort_own_merge(g, root, &failure(&args, &output))?;
    if files.is_empty() {
        Err(failure(&args, &output))
    } else {
        Ok(AcceptOutcome::Conflict { files })
    }
}

/// D-3: the base branch's head, when it is exactly accept's merge: a commit whose
/// parents are `(expected_base, run_head)`.
fn landed(
    g: Git<'_>,
    root: &Path,
    base_ref: &str,
    expected_base: &str,
    run_head: &str,
) -> Result<Option<String>, String> {
    let Some(head) = read(g, root, base_ref)? else {
        return Ok(None);
    };
    let (_, parents) = parents_of(g, root, &head)?;
    Ok((parents == [expected_base, run_head]).then_some(head))
}

/// `commit` and its parents.
fn parents_of(g: Git<'_>, root: &Path, commit: &str) -> Result<(String, Vec<String>), String> {
    let line = g.ok(
        root,
        &[
            os("rev-list"),
            os("--parents"),
            os("-n"),
            os("1"),
            os(commit),
        ],
    )?;
    let mut shas = line.split_whitespace().map(str::to_string);
    let head = shas.next().unwrap_or_default();
    Ok((head, shas.collect()))
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

/// Ruling T9-m1, and final fix batch F1's D-1: what a successful `git merge` did.
/// - `HEAD` still at `expected_base`: git merged nothing, because the base already
///   contains the run head (the user merged it by hand). That is the accepted state, and
///   nothing is undone.
/// - A merge of `(expected_base, run_head)`: accept's own merge.
/// - A merge of `(other, run_head)` carrying accept's own `message`: a commit landed on
///   the base between the check and the merge, so accept's merge sits on it. That merge
///   is undone (`reset --keep HEAD^1`, which keeps the user's commit and refuses to lose
///   local changes) and refused.
/// - Anything else is left alone and refused: it is not a commit accept made.
fn check_first_parent(
    g: Git<'_>,
    root: &Path,
    expected_base: &str,
    run_head: &str,
    message: &str,
) -> Result<AcceptOutcome, String> {
    let (head, parents) = parents_of(g, root, "HEAD")?;
    if head == expected_base {
        return if is_ancestor(g, root, run_head, &head)? {
            Ok(AcceptOutcome::Merged { commit: head })
        } else {
            Err(MOVED_AGAIN.to_string())
        };
    }
    match parents.as_slice() {
        [first, second] if second == run_head && first == expected_base => {
            Ok(AcceptOutcome::Merged { commit: head })
        }
        [_, second] if second == run_head && own_message(g, root, &head, message)? => {
            let reset = [os("reset"), os("-q"), os("--keep"), os("HEAD^1")];
            let output = g.user_write(root, &reset)?;
            if !output.success {
                return Err(format!(
                    "the base branch moved during accept, and undoing the merge failed \
                     ({}); {} is on the merge commit {}",
                    failure(&reset, &output),
                    root.display(),
                    short(&head)
                ));
            }
            Err(MOVED_AGAIN.to_string())
        }
        _ => Err(MOVED_AGAIN.to_string()),
    }
}

/// Whether `commit`'s message is exactly `message`, accept's own.
fn own_message(g: Git<'_>, root: &Path, commit: &str, message: &str) -> Result<bool, String> {
    let text = g.ok(root, &[os("log"), os("-1"), os("--format=%B"), os(commit)])?;
    Ok(text.trim_end() == message.trim_end())
}

/// `message` as `git commit`'s default `whitespace` clean-up stores it: trailing
/// whitespace stripped from every line, runs of blank lines collapsed to one, and
/// leading and trailing blank lines dropped.
fn cleaned(message: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in message.lines().map(str::trim_end) {
        if line.is_empty() && lines.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|last| last.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}
