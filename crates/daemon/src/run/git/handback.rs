//! Decision 36 step 6: the conflict hand-back into a task worktree, and the undoing of
//! its merge (ruling T11-N1). Split from `merge.rs` for AGENTS.md rule 8 (fix round 4).
//!
//! No write here updates a ref through `HEAD` (final fix batch F1, fix round 3), and the
//! one ref written, the task's own branch, is written with `--no-deref`, so a branch a
//! worker made a symbolic ref is replaced, never followed (fix round 4, S1). A hand-back
//! refused after its merge ran undoes that merge before it returns, so the next one is
//! not blocked by it (fix round 4, S2).
//!
//! Blocking; call only from `spawn_blocking`, behind the caller's
//! [`super::GitQueue::write`].

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::merge::read;
use super::{DIFF_FLAGS, Git, failure, nul_fields, os};

/// What [`hand_back`] did (ruling T14-C1): `onto` is the task branch's tip it merged
/// the run head onto (the worktree's `HEAD` before the merge), `head` the worktree's
/// `HEAD` after it (the merge commit when clean, `onto` when conflicted), and `files`
/// the unmerged paths (empty when clean).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandBack {
    pub onto: String,
    pub head: String,
    pub files: Vec<String>,
}

/// Decision 36 step 6: the run head merged into the task worktree. A clean merge is
/// committed and gives no files. A conflict leaves its markers and `MERGE_HEAD` for the
/// worker and gives the unmerged files. Any other failure (untracked files in the way,
/// say), a `HEAD` not on the task's own branch, or a merge already in progress that is
/// not this hand-back's own untouched leftover, is an error. The result names the tip
/// the merge was made onto (ruling T14-C1), read after the merge ran (review N4): the
/// engine re-queues a hand-back only when that tip is the claimed commit.
///
/// Final fix batch F1, fix round 3: no ref is written through `HEAD`. `git merge
/// --no-commit --no-ff` writes only the index, the files and the merge state; the merge
/// commit is made with `write-tree` and `commit-tree` (parents explicit), and the task's
/// own branch, the one ref its pin names, is moved with a compare-and-swap `update-ref
/// --no-deref`. A merge not made onto that branch's tip, or a `HEAD` that no longer
/// names it, is an error, and nothing but that branch was written.
pub fn hand_back(
    git: &OsStr,
    worktree: &Path,
    run_head: &str,
    timeout: Duration,
) -> Result<HandBack, String> {
    let g = Git::new(git, timeout);
    let own = own_ref(worktree)?;
    let run_head = read(g, worktree, &format!("{run_head}^{{commit}}"))?
        .ok_or_else(|| format!("{run_head} is not a commit"))?;
    // Ruling T11-N1(a): a leftover MERGE_HEAD would make git refuse the merge and
    // `unmerged` report the old merge's files as if they were this one's conflict. Fix
    // round 4, S2: the engine's own leftover of this run head (a crash, or an undo that
    // failed) is recognised and cleared instead.
    if merge_in_progress(g, worktree)? {
        match leftover(g, worktree, &run_head)? {
            Leftover::Committed => quit_merge(g, worktree)?,
            Leftover::Untouched => undo_clean_merge(g, worktree)?,
            Leftover::Other => {
                return Err(format!(
                    "a merge is already in progress in {}; finish it or run git merge --abort",
                    worktree.display()
                ));
            }
        }
    }
    // Fix round 4, S2: a detached `HEAD` (a stopped rebase, a `checkout <sha>`) is
    // refused before anything is merged: the merge would be made onto a commit the
    // branch does not hold.
    let on = g.read(worktree, &[os("symbolic-ref"), os("-q"), os("HEAD")])?;
    if !on.success || on.stdout.trim() != own {
        return Err(format!(
            "{}'s HEAD is not on {own} (detached, or mid-rebase); check out {} there \
             before the run head can be handed back",
            worktree.display(),
            own.trim_start_matches("refs/heads/")
        ));
    }
    let args = [
        os("merge"),
        os("-q"),
        os("--no-ff"),
        os("--no-commit"),
        os("--no-autostash"),
        os(&run_head),
    ];
    let output = g.write_raw(worktree, &args)?;
    // Every refusal from here until the branch has moved undoes the merge first.
    let refuse = |why: String| Err(undone(g, worktree, why));
    // Review N4: `onto` is read after the merge ran, from the branch itself. Every read
    // below goes through the pin's checks: a `HEAD` that no longer names the task's
    // branch, or a branch made a symbolic ref, is refused there.
    let onto = match read(g, worktree, &own) {
        Ok(Some(onto)) => onto,
        Ok(None) => return refuse(format!("{own} does not exist")),
        Err(err) => return refuse(err),
    };
    match read(g, worktree, "HEAD") {
        Ok(head) if head.as_deref() == Some(onto.as_str()) => {}
        Ok(_) => return refuse(moved(worktree, &own)),
        Err(err) => return refuse(err),
    }
    if !output.success {
        let files = match unmerged(g, worktree) {
            Ok(files) => files,
            Err(err) => return refuse(err),
        };
        return if files.is_empty() {
            refuse(failure(&args, &output))
        } else {
            Ok(HandBack {
                head: onto.clone(),
                onto,
                files,
            })
        };
    }
    if !merge_in_progress(g, worktree)? {
        // Ruling T14-R3 (R2-2): a run head already in the task's history ("Already up
        // to date") leaves no merge state; the tip merged onto is the branch itself.
        return Ok(HandBack {
            head: onto.clone(),
            onto,
            files: Vec::new(),
        });
    }
    match read(g, worktree, "ORIG_HEAD") {
        Ok(orig) if orig.as_deref() == Some(onto.as_str()) => {}
        Ok(_) => return refuse(moved(worktree, &own)),
        Err(err) => return refuse(err),
    }
    let commit = match merge_commit(g, worktree, &own, &onto, &run_head) {
        Ok(commit) => commit,
        Err(err) => return refuse(err),
    };
    let cas = [
        os("update-ref"),
        os("--no-deref"),
        os(&own),
        os(&commit),
        os(&onto),
    ];
    let output = g.write_raw(worktree, &cas)?;
    if !output.success {
        return refuse(format!(
            "{}; {}",
            moved(worktree, &own),
            failure(&cas, &output)
        ));
    }
    // The index already holds the merge's tree; only the merge state goes.
    quit_merge(g, worktree)?;
    if read(g, worktree, "HEAD")?.as_deref() != Some(commit.as_str()) {
        return Err(moved(worktree, &own));
    }
    Ok(HandBack {
        onto,
        head: commit,
        files: Vec::new(),
    })
}

/// The clean merge's commit: `write-tree`, then `commit-tree` with `(onto, run_head)`
/// as parents, the order `git merge` uses.
fn merge_commit(
    g: Git<'_>,
    worktree: &Path,
    own: &str,
    onto: &str,
    run_head: &str,
) -> Result<String, String> {
    let tree = g.write(worktree, &[os("write-tree")])?.trim().to_string();
    let branch = own.trim_start_matches("refs/heads/");
    let message = format!("Merge commit '{run_head}' into {branch}");
    Ok(g.write(
        worktree,
        &[
            os("commit-tree"),
            os(&tree),
            os("-p"),
            os(onto),
            os("-p"),
            os(run_head),
            os("-m"),
            os(&message),
        ],
    )?
    .trim()
    .to_string())
}

/// Fix round 4, S2: `why`, after the merge the refused hand-back made is undone (a
/// clean one), or with the reason it was left in place (a conflicted one, whose
/// resolution cannot be told from the worker's, or an undo that failed).
fn undone(g: Git<'_>, worktree: &Path, why: String) -> String {
    match merge_in_progress(g, worktree) {
        Ok(false) => why,
        Ok(true) => match unmerged(g, worktree) {
            Ok(files) if files.is_empty() => match undo_clean_merge(g, worktree) {
                Ok(()) => format!("{why}; the hand-back's merge was undone"),
                Err(err) => format!("{why}; the hand-back's merge could not be undone: {err}"),
            },
            Ok(_) => format!(
                "{why}; the hand-back's conflicted merge was left in {}; run git merge --abort there",
                worktree.display()
            ),
            Err(err) => format!("{why}; the merge left in progress could not be read: {err}"),
        },
        Err(err) => format!("{why}; whether a merge was left in progress could not be read: {err}"),
    }
}

/// Fix round 3: the branch ref the engine may move in `worktree` (its pin's), or an
/// error: the engine writes no ref it cannot name.
fn own_ref(worktree: &Path) -> Result<String, String> {
    crate::worktree::pinned::own_ref(worktree).ok_or_else(|| {
        format!(
            "{} is not an engine worktree on a branch of its own; refusing to write in it",
            worktree.display()
        )
    })
}

fn moved(worktree: &Path, own: &str) -> String {
    format!(
        "{own} or {}'s HEAD moved during the hand-back; the merge was not committed",
        worktree.display()
    )
}

/// A merge in progress in a task worktree, as the hand-back and reconcile see it (fix
/// round 3; fix round 4, S2 and S3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Leftover {
    /// The run head's clean merge, already committed on the branch (`HEAD^2` is the
    /// run head): only its merge state is left.
    Committed,
    /// The run head's clean merge, uncommitted, its index exactly the clean merge's
    /// tree: nobody has touched what the engine's merge staged.
    Untouched,
    /// Anything else: another merge, a conflict, or a conflict the worker resolved and
    /// staged. Never undone automatically.
    Other,
}

/// Classifies the merge in progress in `worktree` against the run head `target` (a
/// full sha). Reads only, except that `merge-tree` writes objects.
pub(crate) fn leftover(g: Git<'_>, worktree: &Path, target: &str) -> Result<Leftover, String> {
    if read(g, worktree, "MERGE_HEAD")?.as_deref() != Some(target)
        || !unmerged(g, worktree)?.is_empty()
    {
        return Ok(Leftover::Other);
    }
    if read(g, worktree, "HEAD^2")?.as_deref() == Some(target) {
        return Ok(Leftover::Committed);
    }
    let Some(head) = read(g, worktree, "HEAD")? else {
        return Ok(Leftover::Other);
    };
    // A conflicted hand-back the worker resolved and staged looks the same up to here;
    // only a clean merge has a tree to compare with, and only an untouched index
    // matches it exactly (S3).
    let args = [
        os("merge-tree"),
        os("--write-tree"),
        os("--no-messages"),
        os(&head),
        os(target),
    ];
    let clean = g.read(worktree, &args)?;
    let tree = clean.stdout.lines().next().unwrap_or_default().trim();
    if !clean.success || tree.is_empty() {
        return Ok(Leftover::Other);
    }
    let same = g.read(
        worktree,
        &[
            os("diff-index"),
            os("--cached"),
            os("--quiet"),
            os(tree),
            os("--"),
        ],
    )?;
    Ok(if same.success {
        Leftover::Untouched
    } else {
        Leftover::Other
    })
}

/// Fix round 4, S2: a clean merge in progress undone without writing a ref and without
/// touching the worker's own uncommitted edits: a two-way `read-tree -m -u` from the
/// merged index's tree back to `HEAD` (what `git checkout` does), then `merge --quit`.
/// It fails, changing nothing, where an edit would be overwritten.
pub(crate) fn undo_clean_merge(g: Git<'_>, worktree: &Path) -> Result<(), String> {
    own_ref(worktree)?;
    let head = read(g, worktree, "HEAD")?
        .ok_or_else(|| format!("{}'s HEAD does not name a commit", worktree.display()))?;
    let merged = g.write(worktree, &[os("write-tree")])?.trim().to_string();
    g.write(
        worktree,
        &[os("read-tree"), os("-m"), os("-u"), os(&merged), os(&head)],
    )?;
    quit_merge(g, worktree)
}

/// Ruling T11-N1(b): undoes a hand-back's conflicted merge in a task worktree so the
/// next hand-back starts from the task's own commit. Without a `MERGE_HEAD` there is
/// nothing to undo. Fix round 3: not `git merge --abort`, whose reset writes `HEAD`'s
/// ref; the index and files are read back from `HEAD` (`read-tree --reset -u`), and
/// `merge --quit` drops the merge state. No ref is written.
pub fn abort_merge(git: &OsStr, worktree: &Path, timeout: Duration) -> Result<(), String> {
    let g = Git::new(git, timeout);
    // Refused up front in a worktree without a branch of its own, merge or not.
    own_ref(worktree)?;
    if !merge_in_progress(g, worktree)? {
        return Ok(());
    }
    drop_merge(g, worktree)
}

/// Fix round 3: the merge in progress in `worktree` undone without writing a ref: the
/// index and files read back from `HEAD`, then the merge state dropped. Fix round 4,
/// S4: `HEAD`'s commit, as `git merge --abort` would, not the branch's tip, which a
/// detached `HEAD` does not hold.
pub(crate) fn drop_merge(g: Git<'_>, worktree: &Path) -> Result<(), String> {
    own_ref(worktree)?;
    let head = read(g, worktree, "HEAD")?
        .ok_or_else(|| format!("{}'s HEAD does not name a commit", worktree.display()))?;
    g.write(
        worktree,
        &[os("read-tree"), os("-u"), os("--reset"), os(&head)],
    )?;
    quit_merge(g, worktree)
}

/// `git merge --quit`: the merge state (`MERGE_HEAD`, `MERGE_MSG`, …) dropped; the
/// index, the files and every ref are left as they are.
pub(crate) fn quit_merge(g: Git<'_>, worktree: &Path) -> Result<(), String> {
    g.write(worktree, &[os("merge"), os("--quit")]).map(|_| ())
}

fn merge_in_progress(g: Git<'_>, worktree: &Path) -> Result<bool, String> {
    let args = [os("rev-parse"), os("-q"), os("--verify"), os("MERGE_HEAD")];
    Ok(g.read(worktree, &args)?.success)
}

/// The unmerged paths in `dir`'s index, in git's order.
pub(crate) fn unmerged(g: Git<'_>, dir: &Path) -> Result<Vec<String>, String> {
    let mut args = vec![os("diff")];
    args.extend(DIFF_FLAGS.map(os));
    args.extend([os("--name-only"), os("-z"), os("--diff-filter=U")]);
    let text = g.ok(dir, &args)?;
    let mut files: Vec<String> = nul_fields(&text).map(str::to_string).collect();
    files.dedup();
    Ok(files)
}
