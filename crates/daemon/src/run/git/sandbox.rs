//! What a worker's sandbox may write of the repository's git directories (decisions 25
//! and 54, narrowed by final fix batch F1, findings C-C1 and D-5, and by its fix round
//! 1, findings N2 and N3, and replaced by F1b): nothing of the common directory, and in
//! its worktree's own git directory only what a commit on a detached `HEAD` needs: the
//! exact files on macOS, the directory whole with its configuration denied on Linux
//! ([`GrantShape`]); its objects go to a private directory outside the repository.
//! Blocking; call only
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

/// Which sandbox a worker's grant is written for. Both shapes are testable on any
/// host; [`GrantShape::host`] picks the one the host's Claude Code sandbox needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantShape {
    /// macOS Seatbelt: a path rule lets a missing file be created at a granted path, so
    /// the grant names exactly [`WORKTREE_GIT_FILES`] (with their `.lock`s) and
    /// [`WORKTREE_GIT_DIRS`], and denies nothing. Also every Codex worker's (its
    /// writable roots take no denial).
    Files,
    /// Linux bubblewrap (Claude Code's sandbox-runtime): each granted path is
    /// bind-mounted, which works only for a path that exists, and a lock file never
    /// exists beforehand (git creates it with `O_EXCL`), so an exact-file grant can
    /// never commit there. The grant is the checkout's own git directory whole, with
    /// [`WORKTREE_GIT_DENIED_FILES`] and [`WORKTREE_GIT_DENIED_DIRS`] denied, each
    /// made to exist first ([`placeholders`]).
    WholeDir,
}

impl GrantShape {
    /// [`GrantShape::WholeDir`] on Linux, [`GrantShape::Files`] elsewhere.
    pub fn host() -> GrantShape {
        if cfg!(target_os = "linux") {
            GrantShape::WholeDir
        } else {
            GrantShape::Files
        }
    }
}

/// A worker session's git grant: what its sandbox may write, and, inside that, what it
/// may not (Claude Code's `sandbox.filesystem.denyWrite`; empty for
/// [`GrantShape::Files`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkerGrant {
    pub writable: Vec<PathBuf>,
    pub deny: Vec<PathBuf>,
}

/// The files of the git directory a [`GrantShape::WholeDir`] grant denies: those that
/// decide which repository and which config git uses in the checkout (`commondir`,
/// `gitdir`, `config`, `config.worktree`, which `pinned::check` verifies), `locked`, and
/// `packed-refs` (no ref is the worker's). `commondir` is denied only where it exists
/// (a linked worktree's): any file there, an empty one too, redirects or breaks every
/// git command in the checkout, so no placeholder is made for it, and Claude Code's
/// sandbox would mount `/dev/null` at a missing one. A task checkout's own repository
/// has none; one a worker makes there is refused by `pinned::check`, and the daemon's
/// git reads no config through it (`GIT_COMMON_DIR`, [`pinned::common_dir_env`]).
pub const WORKTREE_GIT_DENIED_FILES: [&str; 6] = [
    "commondir",
    "gitdir",
    "config",
    "config.worktree",
    "locked",
    "packed-refs",
];

/// The directories of the git directory a [`GrantShape::WholeDir`] grant denies:
/// `logs` (the worker's git writes no reflog, fix round 5's
/// `role_launch::WORKER_GIT_CONFIG`), `refs` (no ref is the worker's: it commits on a
/// detached `HEAD`), `info` (exclude, attributes, sparse-checkout, grafts) and `hooks`.
pub const WORKTREE_GIT_DENIED_DIRS: [&str; 4] = ["logs", "refs", "info", "hooks"];

/// A `packed-refs` that holds no ref: git's own header line. `git fsck` warns about an
/// empty file (`emptyPackedRefsFile`), not about this.
const PACKED_REFS_PLACEHOLDER: &str = "# pack-refs with: peeled fully-peeled sorted \n";

/// The writable paths of a worker session in `worktree` for a macOS sandbox:
/// [`worker_git_grant`] with [`GrantShape::Files`].
pub fn worker_git_dirs(
    git_common_dir: &Path,
    worktree: &Path,
    roots: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    Ok(worker_git_grant(git_common_dir, worktree, roots, GrantShape::Files)?.writable)
}

/// The grant of a worker session in `worktree`: `roots` (what
/// [`crate::run::role_launch::worker_git_roots`] names: the task's private object
/// directory, created here and given canonical) plus, in the worktree's own git
/// directory (a task checkout's own repository's, or a linked worktree's
/// `<common>/worktrees/<name>`), per `shape`: exactly [`WORKTREE_GIT_FILES`] (and their
/// `.lock`s) and [`WORKTREE_GIT_DIRS`]; or the directory whole, with
/// [`WORKTREE_GIT_DENIED_FILES`] and [`WORKTREE_GIT_DENIED_DIRS`] denied. Final fix
/// batch F1b: nothing of the common directory itself; a root inside it is refused.
///
/// The git directory is found from the repository's side ([`pinned::find_git_dir`]:
/// the `<common>/worktrees/*/gitdir` file naming `<worktree>/.git`), never from the
/// worktree's `.git` file, which the worker can rewrite.
pub fn worker_git_grant(
    git_common_dir: &Path,
    worktree: &Path,
    roots: &[PathBuf],
    shape: GrantShape,
) -> Result<WorkerGrant, String> {
    // The git directory the daemon pinned when it made the worktree, else the one the
    // repository names (uniquely) for it (fix round 2, R2).
    let pin = pinned::pinned(worktree);
    let admin = match &pin {
        Some(pin) if pin.broken.is_none() => pin.git_dir.clone(),
        Some(pin) => return Err(pin.broken.clone().unwrap_or_default()),
        None => pinned::find_git_dir(git_common_dir, worktree)?,
    };
    let mut writable = Vec::with_capacity(roots.len() + 2 * WORKTREE_GIT_FILES.len() + 4);
    for root in roots {
        writable.push(private_dir(git_common_dir, root)?);
    }
    let mut files = Vec::with_capacity(2 * WORKTREE_GIT_FILES.len() + 3);
    for file in WORKTREE_GIT_FILES {
        files.push(admin.join(file));
        files.push(admin.join(format!("{file}.lock")));
    }
    files.extend(WORKTREE_GIT_DIRS.iter().map(|dir| admin.join(dir)));
    let denied = WORKTREE_GIT_DENIED_FILES
        .iter()
        .chain(&WORKTREE_GIT_DENIED_DIRS)
        .map(|name| admin.join(name));
    // Final fix batch F1c (I1's sweep): every entry is named lexically under the
    // engine's own git directory, never resolved. A link a worker left at one of them
    // is removed before the next session is granted it, so a sandbox that resolves its
    // grants is never handed the link's target. The Linux shape sweeps its denied
    // entries too: the sandbox must deny the path itself, never what a link names.
    let swept: Vec<PathBuf> = match shape {
        GrantShape::Files => files.clone(),
        GrantShape::WholeDir => files.iter().cloned().chain(denied).collect(),
    };
    for path in &swept {
        if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            std::fs::remove_file(path)
                .map_err(|err| format!("cannot remove the link {}: {err}", path.display()))?;
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
    match shape {
        GrantShape::Files => {
            writable.extend(files);
            Ok(WorkerGrant {
                writable,
                deny: Vec::new(),
            })
        }
        GrantShape::WholeDir => {
            let deny = placeholders(&admin)?;
            writable.push(admin);
            Ok(WorkerGrant { writable, deny })
        }
    }
}

/// The denied entries of the git directory `admin`, each made to exist where it was
/// missing: bubblewrap can deny only a path that exists, and for a missing one Claude
/// Code's sandbox-runtime binds `/dev/null` there for the session, leaving a file at
/// the path meanwhile. An empty file (git reads an empty `gitdir`, `config` or
/// `config.worktree` as nothing; an empty `locked` locks a linked worktree, which the
/// engine unlocks before it removes one, and means nothing in a repository's own git
/// directory), `packed-refs` with only its header, an empty directory. `commondir` is
/// never made, and is denied only where it exists. Each is made without following a
/// link (the sweep removed any) and checked with `lstat`: a link found afterwards fails
/// closed.
fn placeholders(admin: &Path) -> Result<Vec<PathBuf>, String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut deny = Vec::new();
    for name in WORKTREE_GIT_DENIED_FILES {
        let path = admin.join(name);
        if name != "commondir" {
            let made = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path);
            match made {
                Ok(mut file) if name == "packed-refs" => file
                    .write_all(PACKED_REFS_PLACEHOLDER.as_bytes())
                    .map_err(|err| format!("cannot write {}: {err}", path.display()))?,
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(format!("cannot create {}: {err}", path.display())),
            }
        }
        if exists_unlinked(&path)? {
            deny.push(path);
        }
    }
    for name in WORKTREE_GIT_DENIED_DIRS {
        let path = admin.join(name);
        match std::fs::create_dir(&path) {
            Err(err) if err.kind() != std::io::ErrorKind::AlreadyExists => {
                return Err(format!("cannot create {}: {err}", path.display()));
            }
            _ => {}
        }
        if exists_unlinked(&path)? {
            deny.push(path);
        }
    }
    Ok(deny)
}

/// Whether `path` exists; an error when it is a symbolic link.
fn exists_unlinked(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "{} is a symbolic link; it was tampered with",
            path.display()
        )),
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(format!("cannot read {}: {err}", path.display())),
    }
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
    // Final fix batch F1d: a task's temporary directory lives under the daemon's short
    // root, which is made private and checked with `lstat`, never resolved.
    let parent = if parent == super::tmp::tmp_root() {
        super::tmp::ensure_root()?
    } else {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
        parent
            .canonicalize()
            .map_err(|err| format!("cannot resolve {}: {err}", parent.display()))?
    };
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

/// F1c round 3 (N2): writes `content` as `base/<dirs…>/<name>` without following a link
/// at any of `dirs` (which a worker or a confined command may control; `base` is the
/// engine's own). Each directory is opened `O_DIRECTORY | O_NOFOLLOW` relative to the
/// last, the file is written to an exclusive temporary name in the final one and
/// renamed over `name` there, so a link swapped in at any step makes the write fail
/// rather than land elsewhere.
pub(crate) fn put_beneath(
    base: &Path,
    dirs: &[&str],
    name: &str,
    content: &[u8],
) -> Result<(), String> {
    use std::ffi::CString;
    use std::io::Write as _;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt as _;

    let shown = dirs
        .iter()
        .fold(base.to_path_buf(), |p, d| p.join(d))
        .join(name);
    let failed = |what: &str| {
        format!(
            "cannot write {} ({what}: {}); it was tampered with or is unwritable",
            shown.display(),
            std::io::Error::last_os_error()
        )
    };
    let c = |s: &[u8]| CString::new(s).map_err(|_| format!("{} holds a NUL", shown.display()));
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let base_c = c(base.as_os_str().as_bytes())?;
    // SAFETY: a valid NUL-terminated path; the result is checked.
    let fd = unsafe { libc::open(base_c.as_ptr(), flags) };
    if fd < 0 {
        return Err(failed("open"));
    }
    // SAFETY: `fd` is a descriptor this function just opened and owns.
    let mut dir = unsafe { OwnedFd::from_raw_fd(fd) };
    for d in dirs {
        let d_c = c(d.as_bytes())?;
        // SAFETY: as above, relative to an owned directory descriptor.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), d_c.as_ptr(), flags) };
        if fd < 0 {
            return Err(failed(d));
        }
        // SAFETY: as above.
        dir = unsafe { OwnedFd::from_raw_fd(fd) };
    }
    let temp = format!("anthrex-{name}.tmp");
    let temp_c = c(temp.as_bytes())?;
    let name_c = c(name.as_bytes())?;
    // SAFETY: unlinking a name inside an owned directory descriptor.
    unsafe { libc::unlinkat(dir.as_raw_fd(), temp_c.as_ptr(), 0) };
    let fflags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: as above; mode 0644.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            temp_c.as_ptr(),
            fflags,
            0o644 as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(failed("create"));
    }
    // SAFETY: an owned, freshly created file descriptor.
    let mut file = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
    file.write_all(content)
        .map_err(|err| format!("cannot write {}: {err}", shown.display()))?;
    drop(file);
    // SAFETY: both names are inside the same owned directory descriptor.
    let renamed = unsafe {
        libc::renameat(
            dir.as_raw_fd(),
            temp_c.as_ptr(),
            dir.as_raw_fd(),
            name_c.as_ptr(),
        )
    };
    if renamed != 0 {
        return Err(failed("rename"));
    }
    Ok(())
}
