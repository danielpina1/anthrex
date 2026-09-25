//! The merge state of a task worktree (final fix batch F1, fix round 5): the
//! `MERGE_HEAD`, `MERGE_MSG` and `MERGE_MODE` a conflicted hand-back leaves for the
//! worker, which the engine writes and removes itself, as files; how a merge found in
//! progress is classified ([`leftover`]); and how one is undone ([`abort_merge`],
//! [`undo_clean_merge`]). No git command here writes a ref or a pseudo-ref: the worker
//! can write this git directory (its sandbox grant), and a still-running worker process
//! could redirect any such write, whatever was checked just before (four fix rounds
//! each closed one route: `HEAD`, `commondir`, the task's branch, `ORIG_HEAD`).
//!
//! Each file is written to a name the worker cannot write (`anthrex-<name>.tmp`,
//! created exclusively) and renamed over its place: a rename replaces a symbolic link
//! the worker planted there, never writes through it. Removal is `unlink`, which
//! removes a link itself.
//!
//! Blocking; call only from `spawn_blocking`, behind the caller's
//! [`super::GitQueue::write`].

use std::ffi::OsStr;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::merge::read;
use super::{DIFF_FLAGS, Git, nul_fields, os};

/// What a merge in progress leaves in a worktree's git directory, `MERGE_HEAD` first:
/// what [`clear`] removes (the set `git merge --quit` removes). `AUTO_MERGE` is never
/// written by the engine; one a worker's own merge left is dropped with the rest.
const STATE_FILES: [&str; 5] = [
    "MERGE_HEAD",
    "MERGE_MSG",
    "MERGE_MODE",
    "MERGE_RR",
    "AUTO_MERGE",
];

/// The git directory of the engine worktree `worktree`, from its pin (never from its
/// `.git` file, which the worker can rewrite).
fn git_dir(worktree: &Path) -> Result<PathBuf, String> {
    match crate::worktree::pinned::pinned(worktree) {
        Some(pin) if pin.broken.is_none() => Ok(pin.git_dir),
        Some(pin) => Err(pin.broken.unwrap_or_default()),
        None => Err(format!(
            "{} is not an engine worktree; refusing to write its merge state",
            worktree.display()
        )),
    }
}

/// The conflicted hand-back's merge state, as `git merge --no-ff` of `run_head` leaves
/// it: `MERGE_MSG` (`message`), `MERGE_MODE` (`no-ff`), then `MERGE_HEAD`, whose
/// presence is what makes the merge in progress. The worker's `git commit` then makes
/// the merge commit, with parents `(HEAD, run_head)`, inside its own sandbox.
pub(crate) fn write(worktree: &Path, run_head: &str, message: &str) -> Result<(), String> {
    let dir = git_dir(worktree)?;
    put(&dir, "MERGE_MSG", message.as_bytes())?;
    put(&dir, "MERGE_MODE", b"no-ff")?;
    put(&dir, "MERGE_HEAD", format!("{run_head}\n").as_bytes())
}

/// `content` written to `<dir>/anthrex-<name>.tmp` (created exclusively, so never
/// through a link) and renamed over `<dir>/<name>`.
pub(crate) fn put(dir: &Path, name: &str, content: &[u8]) -> Result<(), String> {
    let temp = dir.join(format!("anthrex-{name}.tmp"));
    let failed = |err: std::io::Error| format!("cannot write {}: {err}", dir.join(name).display());
    remove(&temp).map_err(failed)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(failed)?;
    file.write_all(content).map_err(failed)?;
    drop(file);
    std::fs::rename(&temp, dir.join(name)).map_err(failed)
}

/// `path` unlinked; already gone is fine.
fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// The merge state dropped, `MERGE_HEAD` first (what `git merge --quit` does, without
/// running git): the index, the files and every ref are left as they are.
pub(crate) fn clear(worktree: &Path) -> Result<(), String> {
    let dir = git_dir(worktree)?;
    for name in STATE_FILES {
        let path = dir.join(name);
        remove(&path).map_err(|err| format!("cannot remove {}: {err}", path.display()))?;
    }
    Ok(())
}

/// The conflicted hand-back's `MERGE_MSG`, worded as `git merge` words it for a commit:
/// the subject, then the conflicted paths as comment lines.
pub(crate) fn message(own: &str, run_head: &str, files: &[String]) -> String {
    let branch = own.trim_start_matches("refs/heads/");
    let mut text = format!("Merge commit '{run_head}' into {branch}\n\n# Conflicts:\n");
    for file in files {
        text.push_str(&format!("#\t{file}\n"));
    }
    text
}

/// A merge in progress in a task worktree, as the hand-back and reconcile see it (fix
/// round 3; fix round 4, S2 and S3; fix round 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Leftover {
    /// The run head's clean merge, already committed on the branch (`HEAD^2` is the
    /// run head): only its merge state is left.
    Committed,
    /// The run head's merge, nothing unmerged and the index exactly the merge's tree
    /// (clean, or with its conflict markers): nobody has touched what was staged. A
    /// conflicted hand-back interrupted before its conflicts were staged looks so.
    Untouched,
    /// Anything else: another merge, a conflict, or a conflict the worker resolved and
    /// staged. Never undone automatically.
    Other,
}

/// Classifies the merge in progress in `worktree` against the run head `target` (a
/// full sha), its `HEAD` being `head` (imported, final fix batch F1b). Reads only,
/// except that `merge-tree` writes objects.
pub(crate) fn leftover(
    g: Git<'_>,
    worktree: &Path,
    target: &str,
    head: &str,
) -> Result<Leftover, String> {
    if read(g, worktree, "MERGE_HEAD")?.as_deref() != Some(target)
        || !unmerged(g, worktree)?.is_empty()
    {
        return Ok(Leftover::Other);
    }
    if read(g, worktree, &format!("{head}^2"))?.as_deref() == Some(target) {
        return Ok(Leftover::Committed);
    }
    let head = head.to_string();
    // A conflicted hand-back the worker resolved and staged looks the same up to here;
    // only an index that is exactly the merge's own tree, markers and all, is untouched
    // (S3).
    Ok(match merge_tree_of(g, worktree, &head, target)? {
        Some(tree) if index_is(g, worktree, &tree)? => Leftover::Untouched,
        _ => Leftover::Other,
    })
}

/// The tree `merge-tree --write-tree` makes of `ours` and `theirs`, conflict markers
/// included, or `None` when it made none.
pub(crate) fn merge_tree_of(
    g: Git<'_>,
    dir: &Path,
    ours: &str,
    theirs: &str,
) -> Result<Option<String>, String> {
    let args = [
        os("merge-tree"),
        os("--write-tree"),
        os("--no-messages"),
        os(ours),
        os(theirs),
    ];
    let output = g.read_head(dir, &args, 1024)?.1;
    let text = String::from_utf8_lossy(&output.head);
    let tree = text.lines().next().unwrap_or_default().trim();
    let is_tree = tree.len() >= 40 && tree.bytes().all(|b| b.is_ascii_hexdigit());
    Ok(is_tree.then(|| tree.to_string()))
}

/// Whether `dir`'s index is exactly `tree` (nothing unmerged, nothing else staged).
pub(crate) fn index_is(g: Git<'_>, dir: &Path, tree: &str) -> Result<bool, String> {
    let mut args = vec![os("diff-index")];
    args.extend(DIFF_FLAGS.map(os));
    args.extend([os("--cached"), os("--quiet"), os(tree), os("--")]);
    let output = g.read(dir, &args)?;
    if output.success {
        Ok(true)
    } else if output.stderr.trim().is_empty() {
        Ok(false)
    } else {
        Err(super::failure(&args, &output))
    }
}

/// Fix round 4, S2: a merge in progress whose index is exactly its merge's tree undone
/// without writing a ref and without touching the worker's own uncommitted edits: a
/// two-way `read-tree -m -u` from the index's tree back to `HEAD` (what `git checkout`
/// does), then the merge state dropped. It fails, changing nothing, where an edit would
/// be overwritten. `head` is the worktree's `HEAD`, imported (final fix batch F1b).
pub(crate) fn undo_clean_merge(g: Git<'_>, worktree: &Path, head: &str) -> Result<(), String> {
    own_ref(worktree)?;
    let merged = g.write(worktree, &[os("write-tree")])?.trim().to_string();
    g.write(
        worktree,
        &[os("read-tree"), os("-m"), os("-u"), os(&merged), os(head)],
    )?;
    clear(worktree)
}

/// Ruling T11-N1(b): undoes a hand-back's conflicted merge in a task worktree so the
/// next hand-back starts from the task's own commit. Without a `MERGE_HEAD` there is
/// nothing to undo. Fix round 3: not `git merge --abort`, whose reset writes `HEAD`'s
/// ref; the index and files are read back from `HEAD` (`read-tree --reset -u`), and
/// the merge state is dropped. No ref is written.
pub fn abort_merge(git: &OsStr, worktree: &Path, timeout: Duration) -> Result<(), String> {
    let g = Git::new(git, timeout);
    // Refused up front in a worktree without a branch of its own, merge or not.
    own_ref(worktree)?;
    if !merge_in_progress(g, worktree)? {
        return Ok(());
    }
    // Final fix batch F1b: the worker's `HEAD`, imported and recorded on the branch.
    let head = super::import::sync_in(g, worktree)?;
    drop_merge(g, worktree, &head)
}

/// Fix round 3: the merge in progress in `worktree` undone without writing a ref: the
/// index and files read back from `HEAD`, then the merge state dropped. Fix round 4,
/// S4: `HEAD`'s commit, as `git merge --abort` would, not the branch's tip, which a
/// detached `HEAD` does not hold. `head` is that commit, imported (F1b).
fn drop_merge(g: Git<'_>, worktree: &Path, head: &str) -> Result<(), String> {
    g.write(
        worktree,
        &[os("read-tree"), os("-u"), os("--reset"), os(head)],
    )?;
    clear(worktree)
}

pub(crate) fn merge_in_progress(g: Git<'_>, worktree: &Path) -> Result<bool, String> {
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

/// Fix round 3: the branch ref the engine may move in `worktree` (its pin's), or an
/// error: the engine writes no ref it cannot name.
pub(crate) fn own_ref(worktree: &Path) -> Result<String, String> {
    crate::worktree::pinned::own_ref(worktree).ok_or_else(|| {
        format!(
            "{} is not an engine worktree on a branch of its own; refusing to write in it",
            worktree.display()
        )
    })
}
