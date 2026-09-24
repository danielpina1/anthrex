//! Decision 36 step 6: the conflict hand-back into a task worktree. Split from
//! `merge.rs` for AGENTS.md rule 8 (fix round 4).
//!
//! Final fix batch F1, fix round 5 (post-breaker): the worker can write this worktree's
//! git directory, and a worker process may still be running while the engine acts, so
//! every check the engine makes before a git command can be raced. Four rounds each
//! closed one route by which an unsandboxed `git merge` here was redirected onto the
//! base (`HEAD`, `commondir`, the task's branch, `ORIG_HEAD`). The hand-back therefore
//! runs no git command that writes a ref or a pseudo-ref in this git directory:
//! - the merge is computed with `merge-tree --write-tree` (objects only; no index, no
//!   files, no pseudo-ref);
//! - a clean one is committed with `commit-tree` (explicit parents), the task's own
//!   branch, named explicitly, is moved with `update-ref --no-deref <own> <new> <old>`,
//!   and the index and files follow with a two-way `read-tree -m -u`;
//! - a conflicted one is put in with `read-tree -m -u` (its markers) and `update-index
//!   --index-info` (its unmerged stages), and the engine writes `MERGE_HEAD`,
//!   `MERGE_MSG` and `MERGE_MODE` itself, by rename ([`super::merge_state`]). The
//!   worker's own `git commit` concludes it, inside its sandbox.
//!
//! Blocking; call only from `spawn_blocking`, behind the caller's
//! [`super::GitQueue::write`].

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::merge::{read, short};
use super::merge_state::{
    self, Leftover, index_is, leftover, merge_in_progress, own_ref, undo_clean_merge,
};
use super::worktrees::is_ancestor;
use super::{Git, LARGE_OUTPUT_BYTES, failure, os};

/// What [`hand_back`] did (ruling T14-C1): `onto` is the task branch's tip it merged
/// the run head onto, `head` the branch's tip after it (the merge commit when clean,
/// `onto` when conflicted), and `files` the unmerged paths (empty when clean).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandBack {
    pub onto: String,
    pub head: String,
    pub files: Vec<String>,
}

/// Decision 36 step 6: the run head merged into the task worktree. A clean merge is
/// committed and gives no files. A conflict leaves its markers, its unmerged stages and
/// `MERGE_HEAD` for the worker and gives the unmerged files. Any other failure
/// (untracked files in the way, say), staged changes, a `HEAD` not on the task's own
/// branch, or a merge already in progress that is not this hand-back's own untouched
/// leftover, is an error. The result names the tip the merge was made onto (ruling
/// T14-C1), the one value of the branch the engine read and compared-and-swapped from
/// (review N4): the engine re-queues a hand-back only when that tip is the claimed
/// commit.
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
    // Ruling T11-N1(a): a leftover MERGE_HEAD would make `unmerged` report the old
    // merge's files as if they were this one's conflict. Fix round 4, S2: the engine's
    // own leftover of this run head (a crash, or an undo that failed) is recognised and
    // cleared instead.
    if merge_in_progress(g, worktree)? {
        match leftover(g, worktree, &run_head)? {
            Leftover::Committed => merge_state::clear(worktree)?,
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
    // refused: the worker's commit of the merge would not land on its branch.
    let on = g.read(worktree, &[os("symbolic-ref"), os("-q"), os("HEAD")])?;
    if !on.success || on.stdout.trim() != own {
        return Err(format!(
            "{}'s HEAD is not on {own} (detached, or mid-rebase); check out {} there \
             before the run head can be handed back",
            worktree.display(),
            own.trim_start_matches("refs/heads/")
        ));
    }
    // Review N4: the tip is read from the branch itself, once; every write below is
    // made from it, and the branch moves only if it still holds it.
    let onto = read(g, worktree, &own)?.ok_or_else(|| format!("{own} does not exist"))?;
    if is_ancestor(g, worktree, &run_head, &onto)? {
        // Ruling T14-R3 (R2-2): a run head already in the task's history ("Already up
        // to date") makes no merge; the tip merged onto is the branch itself.
        return Ok(HandBack {
            head: onto.clone(),
            onto,
            files: Vec::new(),
        });
    }
    // `git merge` refuses staged changes; so does the hand-back, which would otherwise
    // have the worker's staged work ride along in the merge.
    if !index_is(g, worktree, &onto)? {
        return Err(format!(
            "{} has staged changes (or unmerged paths); commit or unstage them before \
             the run head can be handed back",
            worktree.display()
        ));
    }
    match merged(g, worktree, &onto, &run_head)? {
        Merged::Clean(tree) => clean(g, worktree, &own, onto, &run_head, &tree),
        Merged::Conflicted { tree, entries } => {
            conflicted(g, worktree, &own, onto, &run_head, &tree, &entries)
        }
    }
}

/// `merge-tree --write-tree`'s answer for `onto` and the run head.
enum Merged {
    Clean(String),
    /// The tree with conflict markers, and each unmerged index entry as `merge-tree
    /// -z` prints it (`<mode> <object> <stage>\t<path>`), in index order.
    Conflicted {
        tree: String,
        entries: Vec<Vec<u8>>,
    },
}

fn merged(g: Git<'_>, worktree: &Path, onto: &str, run_head: &str) -> Result<Merged, String> {
    let args = [
        os("merge-tree"),
        os("--write-tree"),
        os("-z"),
        os("--no-messages"),
        os(onto),
        os(run_head),
    ];
    let (output, kept) = g.read_head(worktree, &args, LARGE_OUTPUT_BYTES)?;
    if kept.dropped() {
        return Err(format!(
            "git merge-tree printed more than {LARGE_OUTPUT_BYTES} bytes of conflicts"
        ));
    }
    let mut fields = kept.head.split(|b| *b == 0).filter(|f| !f.is_empty());
    let tree = fields
        .next()
        .map(|t| String::from_utf8_lossy(t).into_owned())
        .filter(|t| t.len() >= 40 && t.bytes().all(|b| b.is_ascii_hexdigit()));
    let Some(tree) = tree else {
        return Err(failure(&args, &output));
    };
    if output.success {
        return Ok(Merged::Clean(tree));
    }
    // Exit 1 with a tree: the conflict. Its unmerged entries come first; anything
    // after them would be messages, which `--no-messages` leaves out.
    let entries: Vec<Vec<u8>> = fields
        .take_while(|f| stage_path(f).is_some())
        .map(<[u8]>::to_vec)
        .collect();
    if entries.is_empty() {
        return Err(failure(&args, &output));
    }
    Ok(Merged::Conflicted { tree, entries })
}

/// The path of an unmerged entry `<mode> <object> <stage>\t<path>`.
fn stage_path(entry: &[u8]) -> Option<&[u8]> {
    let tab = entry.iter().position(|b| *b == b'\t')?;
    let head = std::str::from_utf8(&entry[..tab]).ok()?;
    let mut parts = head.split(' ');
    let well_formed = parts.next().is_some_and(|m| m.len() == 6)
        && parts.next().is_some_and(|o| o.len() >= 40)
        && parts.next().is_some_and(|s| matches!(s, "1" | "2" | "3"))
        && parts.next().is_none();
    well_formed.then_some(&entry[tab + 1..])
}

/// A clean merge: committed, the branch moved from `onto` to it, then the index and
/// files. Whether the files can follow (no untracked file in the way, no local edit
/// over a merged path) is checked before the branch moves, and a `read-tree` that
/// still fails moves the branch back.
fn clean(
    g: Git<'_>,
    worktree: &Path,
    own: &str,
    onto: String,
    run_head: &str,
    tree: &str,
) -> Result<HandBack, String> {
    let branch = own.trim_start_matches("refs/heads/");
    let message = format!("Merge commit '{run_head}' into {branch}");
    let commit = g
        .write(
            worktree,
            &[
                os("commit-tree"),
                os(tree),
                os("-p"),
                os(&onto),
                os("-p"),
                os(run_head),
                os("-m"),
                os(&message),
            ],
        )?
        .trim()
        .to_string();
    g.ok(
        worktree,
        &[
            os("read-tree"),
            os("-n"),
            os("-m"),
            os("-u"),
            os(&onto),
            os(&commit),
        ],
    )?;
    cas(g, worktree, own, &commit, &onto)?;
    settle(g, worktree, own, &onto, &commit)?;
    Ok(HandBack {
        onto,
        head: commit,
        files: Vec::new(),
    })
}

/// The index and files moved from `onto` to `commit`, the branch having just moved
/// there: a two-way `read-tree -m -u`, which keeps the worker's edits to paths the merge
/// did not change. If it fails, the branch goes back to `onto`, so it never holds a
/// commit its worktree does not show.
pub(crate) fn settle(
    g: Git<'_>,
    worktree: &Path,
    own: &str,
    onto: &str,
    commit: &str,
) -> Result<(), String> {
    let args = [os("read-tree"), os("-m"), os("-u"), os(onto), os(commit)];
    let Err(err) = g.write(worktree, &args) else {
        return Ok(());
    };
    Err(match cas(g, worktree, own, onto, commit) {
        Ok(()) => format!("{err}; {own} was moved back to {}", short(onto)),
        Err(back) => format!(
            "{err}; {own} could not be moved back to {}: {back}",
            short(onto)
        ),
    })
}

/// `update-ref --no-deref <own> <new> <old>`: the task's own branch, named, moved only
/// if it still holds `old`; a symbolic ref there is replaced, never followed.
fn cas(g: Git<'_>, worktree: &Path, own: &str, new: &str, old: &str) -> Result<(), String> {
    let args = [
        os("update-ref"),
        os("--no-deref"),
        os(own),
        os(new),
        os(old),
    ];
    let output = g.write_raw(worktree, &args)?;
    if output.success {
        return Ok(());
    }
    let mut why = format!(
        "{own} moved during the hand-back; the merge was not committed; {}",
        failure(&args, &output)
    );
    if output.stderr.contains(".lock") && output.stderr.contains("File exists") {
        // Re-review 3, N2: a lock a killed git left fails every retry the same way.
        why.push_str(
            "; if no git process is running in the worktree, remove the stale lock file \
             git names",
        );
    }
    Err(why)
}

/// A conflict: the tree with its markers put in the index and files, the merge state
/// written, then the unmerged stages staged, so `git status` and `git commit` see a
/// real unmerged merge. `read-tree -m -u` refuses, changing nothing, where an
/// untracked file or a local edit is in the way. A failure after it is backed out.
fn conflicted(
    g: Git<'_>,
    worktree: &Path,
    own: &str,
    onto: String,
    run_head: &str,
    tree: &str,
    entries: &[Vec<u8>],
) -> Result<HandBack, String> {
    let mut files: Vec<String> = Vec::new();
    for entry in entries {
        let path = String::from_utf8_lossy(stage_path(entry).unwrap_or_default()).into_owned();
        if files.last() != Some(&path) {
            files.push(path);
        }
    }
    g.write(
        worktree,
        &[os("read-tree"), os("-m"), os("-u"), os(&onto), os(tree)],
    )?;
    let message = merge_state::message(own, run_head, &files);
    let staged = merge_state::write(worktree, run_head, &message).and_then(|()| {
        g.write_input(
            worktree,
            &[os("update-index"), os("-z"), os("--index-info")],
            &index_info(onto.len(), entries),
        )
    });
    if let Err(err) = staged {
        // `update-index` writes the index whole or not at all: it still holds `tree`.
        let back = merge_state::clear(worktree).and_then(|()| {
            g.write(
                worktree,
                &[os("read-tree"), os("-m"), os("-u"), os(tree), os(&onto)],
            )
            .map(|_| ())
        });
        return Err(match back {
            Ok(()) => format!("{err}; the hand-back's merge was undone"),
            Err(back) => format!("{err}; the hand-back's merge could not be undone: {back}"),
        });
    }
    Ok(HandBack {
        head: onto.clone(),
        onto,
        files,
    })
}

/// `update-index -z --index-info` input: each conflicted path removed (mode 0, which
/// drops its stage-0 marker entry), then its unmerged stages.
fn index_info(oid_len: usize, entries: &[Vec<u8>]) -> Vec<u8> {
    let zeros = "0".repeat(oid_len);
    let mut input = Vec::new();
    let mut last: Option<&[u8]> = None;
    for entry in entries {
        let path = stage_path(entry).unwrap_or_default();
        if last != Some(path) {
            input.extend_from_slice(format!("0 {zeros} 0\t").as_bytes());
            input.extend_from_slice(path);
            input.push(0);
            last = Some(path);
        }
        input.extend_from_slice(entry);
        input.push(0);
    }
    input
}

/// Reconcile (fix round 5): a clean hand-back that died between moving the branch and
/// updating the index and files. `head` is the branch's tip, a merge commit of the run
/// head; when the index is still exactly its first parent's tree (the state before the
/// hand-back) and that differs from `head`'s, the index and files are moved on
/// ([`settle`], which moves the branch back if they cannot be). `Ok(true)` when it did.
pub(crate) fn finish_clean(g: Git<'_>, worktree: &Path, head: &str) -> Result<bool, String> {
    let own = own_ref(worktree)?;
    let Some(onto) = read(g, worktree, &format!("{head}^1"))? else {
        return Ok(false);
    };
    let tree = |commit: &str| read(g, worktree, &format!("{commit}^{{tree}}"));
    if tree(&onto)? == tree(head)? || !index_is(g, worktree, &onto)? {
        return Ok(false);
    }
    settle(g, worktree, &own, &onto, head).map(|()| true)
}

/// Reconcile (fix round 5): a conflicted hand-back that died after putting its marker
/// tree in, before its merge state was written: `head` and the run head conflict, and
/// the index is exactly `merge-tree`'s marker tree, nothing unmerged. The tree, or
/// `None`.
pub(crate) fn interrupted_conflict(
    g: Git<'_>,
    worktree: &Path,
    head: &str,
    run_head: &str,
) -> Result<Option<String>, String> {
    let args = [
        os("merge-tree"),
        os("--write-tree"),
        os("--no-messages"),
        os("--name-only"),
        os(head),
        os(run_head),
    ];
    let (output, kept) = g.read_head(worktree, &args, 1024)?;
    if output.success {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&kept.head);
    let tree = text.lines().next().unwrap_or_default().trim().to_string();
    let is_tree = tree.len() >= 40 && tree.bytes().all(|b| b.is_ascii_hexdigit());
    if !is_tree || !index_is(g, worktree, &tree)? {
        return Ok(None);
    }
    Ok(Some(tree))
}
