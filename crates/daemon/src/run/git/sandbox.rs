//! What a worker's sandbox may write of the repository's git directories (decisions 25
//! and 54, narrowed by final fix batch F1, findings C-C1 and D-5, and by its fix round
//! 1, findings N2 and N3, and replaced by F1b): nothing of the common directory, and in
//! its worktree's own git directory only the files a commit on a detached `HEAD` needs;
//! its objects go to a private directory outside the repository. Blocking; call only
//! from `spawn_blocking`, behind the run's `GitQueue::write` (it creates the private
//! directory).

use std::path::{Path, PathBuf};

use crate::worktree::pinned;

/// The files of a linked worktree's git directory that a commit, an amend, a reset, a
/// merge's conclusion, a revert, a cherry-pick or a rebase write, each with its `.lock`
/// (found by running each under a real seatbelt profile). Not `commondir`, `gitdir`,
/// `config.worktree` or `locked`: those decide which repository and which config the
/// daemon's own git calls in the worktree use.
pub const WORKTREE_GIT_FILES: [&str; 13] = [
    "HEAD",
    "index",
    "ORIG_HEAD",
    "COMMIT_EDITMSG",
    "MERGE_HEAD",
    "MERGE_MSG",
    "MERGE_MODE",
    "MERGE_RR",
    "AUTO_MERGE",
    "REBASE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "FETCH_HEAD",
];

/// The directories of a linked worktree's git directory a worker may write whole. Not
/// `logs/` (final fix batch F1, fix round 5): the worker's git writes no reflog
/// (`core.logAllRefUpdates=false`, [`crate::run::role_launch::WORKER_GIT_CONFIG`]).
pub const WORKTREE_GIT_DIRS: [&str; 3] = ["rebase-merge", "rebase-apply", "sequencer"];

/// The writable paths of a worker session in `worktree`: `roots` (what
/// [`crate::run::role_launch::worker_git_roots`] names: the task's private object
/// directory, created here and given canonical) plus, in the worktree's own git
/// directory `<common>/worktrees/<name>`, exactly [`WORKTREE_GIT_FILES`] (and their
/// `.lock`s) and [`WORKTREE_GIT_DIRS`]. Final fix batch F1b: nothing of the common
/// directory itself; a root inside it is refused.
///
/// The git directory is found from the repository's side ([`pinned::find_git_dir`]:
/// the `<common>/worktrees/*/gitdir` file naming `<worktree>/.git`), never from the
/// worktree's `.git` file, which the worker can rewrite.
pub fn worker_git_dirs(
    git_common_dir: &Path,
    worktree: &Path,
    roots: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    // The git directory the daemon pinned when it made the worktree, else the one the
    // repository names (uniquely) for it (fix round 2, R2).
    let pin = pinned::pinned(worktree);
    let admin = match &pin {
        Some(pin) if pin.broken.is_none() => pin.git_dir.clone(),
        Some(pin) => return Err(pin.broken.clone().unwrap_or_default()),
        None => pinned::find_git_dir(git_common_dir, worktree)?,
    };
    let mut dirs = Vec::with_capacity(roots.len() + 2 * WORKTREE_GIT_FILES.len() + 4);
    for root in roots {
        dirs.push(private_dir(git_common_dir, root)?);
    }
    for file in WORKTREE_GIT_FILES {
        dirs.push(admin.join(file));
        dirs.push(admin.join(format!("{file}.lock")));
    }
    dirs.extend(WORKTREE_GIT_DIRS.iter().map(|dir| admin.join(dir)));
    // Final fix batch F1c (I1's sweep): every entry is named lexically under the
    // engine's own git directory, never resolved. A link a worker left at one of them
    // is removed before the next session is granted it, so a sandbox that resolves its
    // grants is never handed the link's target.
    for granted in dirs.iter().skip(roots.len()) {
        if std::fs::symlink_metadata(granted).is_ok_and(|meta| meta.file_type().is_symlink()) {
            std::fs::remove_file(granted)
                .map_err(|err| format!("cannot remove the link {}: {err}", granted.display()))?;
        }
    }
    // Fix round 5: no reflog is left for the worker's git (which could not write it) or
    // for the engine's (which must never append through one): the worktree's `HEAD`'s,
    // and the task branch's.
    let mut reflogs = vec![admin.join("logs/HEAD")];
    if let Some(own) = pin.and_then(|pin| pin.own) {
        reflogs.push(git_common_dir.join("logs").join(own));
    }
    for log in reflogs {
        match std::fs::remove_file(&log) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("cannot remove the reflog {}: {err}", log.display()));
            }
            _ => {}
        }
    }
    Ok(dirs)
}

/// Final fix batch F1b: the private object directory `root`, created when missing, a
/// real directory outside the git common directory, spelled canonically (a sandbox
/// matches resolved paths).
///
/// Final fix batch F1c (re-review 4, I1): no sandbox grant is ever computed from a path
/// a worker could have swapped. `root` itself is inside the worker's grant, so a
/// leftover worker process could replace it with a link at any moment; resolving it
/// (`canonicalize`) after checking it could grant the next session whatever the link
/// named. So only the parent, which is the engine's own, is resolved; the leaf's name
/// is appended to it, the directory is made without following a link (`mkdir` fails
/// on one), and the leaf is then checked with `lstat`: anything but a real directory
/// fails closed. The returned path is never the result of resolving the leaf.
pub fn private_dir(git_common_dir: &Path, root: &Path) -> Result<PathBuf, String> {
    let dir = engine_child(root)?;
    let common = git_common_dir
        .canonicalize()
        .unwrap_or_else(|_| git_common_dir.to_path_buf());
    if dir.starts_with(&common) || common.starts_with(&dir) {
        return Err(format!(
            "{} overlaps the git common directory {}; a worker may write nothing of it",
            dir.display(),
            common.display()
        ));
    }
    Ok(dir)
}

/// `path` as a real directory whose parent is the engine's: the parent created and
/// resolved, the leaf's name appended, the leaf made with `mkdir` (which never follows
/// a link) and checked with `lstat`. A link or a file at the leaf is refused.
pub(crate) fn engine_child(path: &Path) -> Result<PathBuf, String> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(format!("{} has no parent directory", path.display()));
    };
    if name == ".." || name == "." {
        return Err(format!("{} does not name a directory", path.display()));
    }
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
    let parent = parent
        .canonicalize()
        .map_err(|err| format!("cannot resolve {}: {err}", parent.display()))?;
    let dir = parent.join(name);
    match std::fs::create_dir(&dir) {
        Err(err) if err.kind() != std::io::ErrorKind::AlreadyExists => {
            return Err(format!("cannot create {}: {err}", dir.display()));
        }
        _ => {}
    }
    let meta = std::fs::symlink_metadata(&dir)
        .map_err(|err| format!("cannot read {}: {err}", dir.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "{} is not a plain directory; it was tampered with",
            dir.display()
        ));
    }
    Ok(dir)
}
