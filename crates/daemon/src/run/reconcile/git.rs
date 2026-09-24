//! The git rows of decision 44's reconcile table: what a crash inside each git op
//! leaves, and whether that is the op's result or nothing at all. Blocking; every call
//! goes through `run::git`'s `Git` (scrubbed env, `--no-optional-locks`, decision 18's
//! flags on writes, a deadline per command).

use std::path::Path;

use super::Reconciled;
use crate::run::contract::sha7;
use crate::run::engine::OpResult;
use crate::run::git::{
    Git, failure, forget_missing, is_ancestor, listed_worktree_in, os, read, reattach_in, short,
    unmerged,
};

/// `CreateRunBranch`, `PrepareWorktree`: the path listed on its branch is the op's
/// result when it has no `setup` (a setup re-runs; it is idempotent). A directory git
/// does not list is what a `worktree add` that died part way leaves: it is removed and
/// `git worktree prune` runs, but only under `own` (the run's `<wt_dir>/runs/<run>`;
/// fix round 1, m1): any other path a corrupted `run.json` names is left alone.
pub(super) fn worktree(
    g: Git<'_>,
    root: &Path,
    branch: &str,
    path: &Path,
    has_setup: bool,
    own: &Path,
    notes: &mut Vec<String>,
) -> Result<Reconciled, String> {
    match listed_worktree_in(g, root, path)? {
        Some(entry) => {
            let on_branch = entry.branch.as_deref() == Some(&format!("refs/heads/{branch}"));
            Ok(match entry.head {
                Some(head) if on_branch && path.is_dir() && !has_setup => {
                    Reconciled::Replay(OpResult::Worktree { head })
                }
                _ => Reconciled::NotStarted,
            })
        }
        None => {
            if path.exists() && !inside(own, path) {
                notes.push(format!(
                    "left {} alone: it is outside the run's worktrees at {}",
                    path.display(),
                    own.display()
                ));
                return Ok(Reconciled::NotStarted);
            }
            if path.exists() {
                std::fs::remove_dir_all(path).map_err(|err| {
                    format!(
                        "cannot remove the partial worktree {}: {err}",
                        path.display()
                    )
                })?;
                notes.push(format!(
                    "removed {}, a worktree directory git never registered",
                    path.display()
                ));
            }
            g.write(root, &[os("worktree"), os("prune")])?;
            Ok(Reconciled::NotStarted)
        }
    }
}

/// Whether `path` is strictly below `own`, lexically, with no `..` or `.` anywhere.
fn inside(own: &Path, path: &Path) -> bool {
    use std::path::Component;
    let plain = |p: &Path| {
        p.components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
    };
    plain(own) && plain(path) && path != own && path.starts_with(own)
}

/// `MergeCandidate`: the run branch's head decides. Parents exactly `(expected run
/// head, task head)` → it landed (`Merged`); still the expected head → it did not, and
/// the integration worktree, possibly detached at an unmerged candidate, goes back on
/// its branch; anything else → the run ref moved (decision 21).
pub(super) fn merge_candidate(
    g: Git<'_>,
    root: &Path,
    integration: &Path,
    run_branch: &str,
    expected_run_head: &str,
    task_head: &str,
    notes: &mut Vec<String>,
) -> Result<Reconciled, String> {
    let run_ref = format!("refs/heads/{run_branch}");
    let Some(head) = read(g, root, &run_ref)? else {
        return Ok(Reconciled::Replay(OpResult::RefMoved {
            reason: format!("{run_ref} was deleted"),
        }));
    };
    if head == expected_run_head {
        reattach(g, integration, &run_ref, run_branch, notes);
        return Ok(Reconciled::NotStarted);
    }
    let line = g.ok(
        root,
        &[
            os("rev-list"),
            os("--parents"),
            os("-n"),
            os("1"),
            os(&head),
        ],
    )?;
    let parents: Vec<&str> = line.split_whitespace().skip(1).collect();
    if parents == [expected_run_head, task_head] {
        // The compare-and-swap landed; the daemon may have died before `reattach`.
        reattach(g, integration, &run_ref, run_branch, notes);
        return Ok(Reconciled::Replay(OpResult::Merged { commit: head }));
    }
    Ok(Reconciled::Replay(OpResult::RefMoved {
        reason: format!(
            "{run_ref} moved from {} to {}",
            short(expected_run_head),
            short(&head)
        ),
    }))
}

/// `git::reattach` unless the integration worktree is already on the run branch. A
/// failure is a note: the merge queue's next candidate materializes over it anyway.
fn reattach(
    g: Git<'_>,
    integration: &Path,
    run_ref: &str,
    run_branch: &str,
    notes: &mut Vec<String>,
) {
    let on_branch = g
        .read(integration, &[os("symbolic-ref"), os("-q"), os("HEAD")])
        .is_ok_and(|out| out.success && out.stdout.trim() == run_ref);
    if on_branch {
        return;
    }
    match reattach_in(g, integration, run_branch) {
        Ok(()) => notes.push(format!(
            "put the integration worktree {} back on {run_branch}",
            integration.display()
        )),
        Err(err) => notes.push(format!(
            "could not put the integration worktree {} back on {run_branch}: {err}",
            integration.display()
        )),
    }
}

/// `HandBack`: a `MERGE_HEAD` of the run head is the conflicted hand-back (its files
/// are the worker's to resolve); a `HEAD` whose second parent is the run head is the
/// clean one. `head` and `onto` are what `git::hand_back` would have reported.
pub(super) fn hand_back(
    g: Git<'_>,
    worktree: &Path,
    run_head: &str,
    notes: &mut Vec<String>,
) -> Result<Reconciled, String> {
    let target = read(g, worktree, &format!("{run_head}^{{commit}}"))?;
    let head = read(g, worktree, "HEAD")?;
    if let Some(merge_head) = read(g, worktree, "MERGE_HEAD")? {
        let files = unmerged(g, worktree)?;
        if Some(&merge_head) != target.as_ref() || files.is_empty() {
            notes.push(format!(
                "{} has a merge of {} in progress that is not the run head's conflicted \
                 hand-back; left for the hand-back to report",
                worktree.display(),
                short(&merge_head)
            ));
            return Ok(Reconciled::NotStarted);
        }
        return Ok(Reconciled::Replay(OpResult::HandedBack {
            files,
            onto: head.clone(),
            head,
        }));
    }
    if target.is_some() && read(g, worktree, "HEAD^2")? == target {
        return Ok(Reconciled::Replay(OpResult::HandedBack {
            files: Vec::new(),
            onto: read(g, worktree, "HEAD^1")?,
            head,
        }));
    }
    Ok(Reconciled::NotStarted)
}

/// `AbortMerge` (carry to M8a.21, ruling on task 15): no `MERGE_HEAD` means the abort
/// happened. A merge still in progress is left to the re-emitted, idempotent abort.
pub(super) fn abort_merge(g: Git<'_>, worktree: &Path) -> Result<Reconciled, String> {
    Ok(match read(g, worktree, "MERGE_HEAD")? {
        None => Reconciled::Replay(OpResult::MergeAborted),
        Some(_) => Reconciled::NotStarted,
    })
}

/// `RemoveWorktree`: the path gone is the removal (a registration left behind, the
/// daemon having died before the prune, is forgotten now); its salvage ref, if the
/// salvage made one, is reported.
pub(super) fn remove_worktree(
    g: Git<'_>,
    root: &Path,
    path: &Path,
    salvage_ref: &str,
    notes: &mut Vec<String>,
) -> Result<Reconciled, String> {
    if path.exists() {
        return Ok(Reconciled::NotStarted);
    }
    if let Some(entry) = listed_worktree_in(g, root, path)? {
        match forget_missing(g, root, path, &entry) {
            Ok(()) => notes.push(format!("pruned the removed worktree {}", path.display())),
            Err(err) => notes.push(format!(
                "could not prune the removed worktree {}: {err}",
                path.display()
            )),
        }
    }
    let salvage_ref = read(g, root, salvage_ref)?.map(|_| salvage_ref.to_string());
    Ok(Reconciled::Replay(OpResult::Removed { salvage_ref }))
}

/// `Accept`: a base that already contains the run head was accepted. The run head is
/// the one the op was to merge (`expected_run_head`, `Run.run_head`), not the run
/// branch's: accept's clean-up deletes that branch, so a crash during the clean-up
/// leaves no branch to read (final fix batch F1, findings B-I1 and D-6). A
/// `MERGE_HEAD` in `root` equal to the run head is a crash inside a conflicted accept:
/// that merge is aborted, and the accept is not started.
pub(super) fn accept(
    g: Git<'_>,
    root: &Path,
    base_branch: &str,
    run_branch: &str,
    expected_run_head: &str,
    branch_prefix: &str,
    notes: &mut Vec<String>,
) -> Result<Reconciled, String> {
    let base_ref = format!("refs/heads/{base_branch}");
    let run_head = if expected_run_head.is_empty() {
        // A journal written before the op carried its run head.
        read(g, root, &format!("refs/heads/{run_branch}"))?
    } else {
        Some(expected_run_head.to_string())
    };
    let (Some(run_head), Some(base_head)) = (run_head, read(g, root, &base_ref)?) else {
        return Ok(Reconciled::NotStarted);
    };
    if is_ancestor(g, root, &run_head, &base_head)? {
        // Fix round 1, m2: the salvage, removal and branch deletion that follow accept's
        // merge did not run (or not all of it), so branches may still be there;
        // `OpKind::Accept`'s contract says so in the outcome and `kept_branches`.
        return Ok(Reconciled::Replay(OpResult::Finished {
            outcome: format!(
                "accepted as {}; clean-up did not run before the restart",
                sha7(&base_head)
            ),
            kept_branches: branches_under(g, root, branch_prefix)?,
        }));
    }
    if read(g, root, "MERGE_HEAD")?.as_deref() == Some(run_head.as_str()) {
        // Fix round 1, m5: the user's checkout, as `git::accept` treats it: the scrubbed
        // environment, `--no-optional-locks` and no hooks, without decision 18's
        // signing override.
        let args = [os("merge"), os("--abort")];
        let output = g.user_write(root, &args)?;
        if !output.success {
            return Err(failure(&args, &output));
        }
        notes.push(format!(
            "aborted the accept's conflicted merge of {run_branch} left in {}",
            root.display()
        ));
    }
    Ok(Reconciled::NotStarted)
}

/// The branches under `refs/heads/<prefix>`, without `refs/heads/`, in git's order.
fn branches_under(g: Git<'_>, root: &Path, prefix: &str) -> Result<Vec<String>, String> {
    let pattern = format!("refs/heads/{}", prefix.trim_end_matches('/'));
    let list = g.ok(
        root,
        &[os("for-each-ref"), os("--format=%(refname)"), os(&pattern)],
    )?;
    Ok(list
        .lines()
        .filter_map(|line| line.strip_prefix("refs/heads/"))
        .map(str::to_string)
        .collect())
}
