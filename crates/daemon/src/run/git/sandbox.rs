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
pub fn private_dir(git_common_dir: &Path, root: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root)
        .map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    let meta = std::fs::symlink_metadata(root)
        .map_err(|err| format!("cannot read {}: {err}", root.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!("{} is not a plain directory", root.display()));
    }
    let canonical = root
        .canonicalize()
        .map_err(|err| format!("cannot resolve {}: {err}", root.display()))?;
    let common = git_common_dir
        .canonicalize()
        .unwrap_or_else(|_| git_common_dir.to_path_buf());
    if canonical.starts_with(&common) || common.starts_with(&canonical) {
        return Err(format!(
            "{} overlaps the git common directory {}; a worker may write nothing of it",
            canonical.display(),
            common.display()
        ));
    }
    Ok(canonical)
}
