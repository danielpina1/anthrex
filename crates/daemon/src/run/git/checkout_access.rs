//! Giving the owner access back on a checkout before it is removed (M8b.10 review I2;
//! milestone 9.11 decision 17): a walk by directory descriptor. A child of
//! [`super`] (the checkout), moved here to keep that file under AGENTS.md rule 8's
//! ~600 lines.

use std::path::Path;

/// Fix round 1 (I5, review ruling 4b): every directory of the tree is set to exactly
/// this, never the found mode with the owner's bits added: the tree is about to be
/// removed, and its group and other bits were chosen by whoever locked it.
const OWNER_ONLY: u32 = 0o700;

/// The deepest directory the walk enters below its root.
const MAX_DEPTH: usize = 4096;

/// M8b.10 review (I2), milestone 9.11 decision 17 (D9): gives the owner read, write and
/// search access back on `path` and on every real directory beneath it, so
/// `remove_dir_all` cannot be stopped by a directory a worker or a confined command
/// locked (`chmod 555`). The walk is by descriptor: `path` is opened `O_DIRECTORY |
/// O_NOFOLLOW`, each child with `openat(dirfd, name, O_DIRECTORY | O_NOFOLLOW)`, and
/// each mode is changed with `fchmod` on the opened descriptor (a directory the owner
/// cannot open is changed first with `fchmodat(dirfd, name, AT_SYMLINK_NOFOLLOW)`,
/// relative to its opened parent). Nothing is reached through a path re-resolved after
/// a step, so a directory swapped for a link mid-walk leads nowhere outside `path`; a
/// link or anything that is not a directory is left alone (the removal unlinks it). A
/// missing `path` is not an error.
pub(crate) fn restore_owner_access(path: &Path) -> Result<(), String> {
    restore_owner_access_with(path, |_| {})
}

/// [`restore_owner_access`], calling `between` with each directory's path (for display
/// only: it is never resolved again) once that directory is opened and its mode fixed,
/// before its children are listed. A test seam: a test swaps an entry there.
#[doc(hidden)]
pub fn restore_owner_access_with(
    path: &Path,
    mut between: impl FnMut(&Path),
) -> Result<(), String> {
    let failed = |what: &Path, err: std::io::Error| {
        format!("cannot restore access to {}: {err}", what.display())
    };
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(failed(path, err)),
    };
    if !meta.file_type().is_dir() {
        return Ok(());
    }
    let c_path = c_name(path.as_os_str()).map_err(|err| failed(path, err))?;
    let dir = match open_dir(libc::AT_FDCWD, &c_path) {
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            chmod_at(libc::AT_FDCWD, &c_path, OWNER_ONLY).map_err(|err| failed(path, err))?;
            open_dir(libc::AT_FDCWD, &c_path)
        }
        other => other,
    };
    match dir {
        Ok(dir) => walk(&dir, path, &mut between, 0),
        // Swapped for a link or a file since the `lstat`: nothing of ours to open.
        Err(err) if not_a_dir(&err) => Ok(()),
        Err(err) => Err(failed(path, err)),
    }
}

/// One opened directory: its mode fixed with `fchmod`, then each child directory
/// opened relative to it and walked.
fn walk(
    dir: &std::os::fd::OwnedFd,
    shown: &Path,
    between: &mut dyn FnMut(&Path),
    depth: usize,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let failed = |what: &Path, err: std::io::Error| {
        format!("cannot restore access to {}: {err}", what.display())
    };
    // Review m8: one descriptor and one stack frame per level; a nest deeper than this
    // fails the walk (the checkout is kept) rather than exhausting either.
    if depth > MAX_DEPTH {
        return Err(format!(
            "cannot restore access to {}: deeper than {MAX_DEPTH} directories",
            shown.display()
        ));
    }
    let mode = fstat_mode(dir.as_raw_fd()).map_err(|err| failed(shown, err))?;
    // Fix round 2 (n4): changed only when the owner lacks a permission, so a directory
    // the owner can already use (one owned by root, or on a read-only mount) is left
    // alone; a changed one is set to exactly owner-only.
    if mode & OWNER_ONLY != OWNER_ONLY {
        // SAFETY: `dir` is an open descriptor for the duration of the call.
        let changed = unsafe { libc::fchmod(dir.as_raw_fd(), OWNER_ONLY as libc::mode_t) };
        if changed != 0 {
            return Err(failed(shown, std::io::Error::last_os_error()));
        }
    }
    between(shown);
    for name in names(dir).map_err(|err| failed(shown, err))? {
        let child = shown.join(std::ffi::OsStr::from_bytes(name.to_bytes()));
        let opened = match open_dir(dir.as_raw_fd(), &name) {
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                // Locked: changed relative to its opened parent, never followed. Fix
                // round 2 (n5): `name` can still be swapped between the `fstatat` and
                // the change for an entry that is not a link (a hard link to a file the
                // user owns); it then becomes exactly owner-only, which is harmless.
                match fstatat_dir_mode(dir.as_raw_fd(), &name) {
                    Some(_) => {
                        chmod_at(dir.as_raw_fd(), &name, OWNER_ONLY)
                            .map_err(|err| failed(&child, err))?;
                        open_dir(dir.as_raw_fd(), &name)
                    }
                    None => continue,
                }
            }
            other => other,
        };
        match opened {
            Ok(opened) => walk(&opened, &child, between, depth + 1)?,
            Err(err) if not_a_dir(&err) => {}
            Err(err) => return Err(failed(&child, err)),
        }
    }
    Ok(())
}

/// A link (`ELOOP` under `O_NOFOLLOW`), a file (`ENOTDIR`) or an entry already gone.
fn not_a_dir(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::NotFound
        || err
            .raw_os_error()
            .is_some_and(|code| code == libc::ELOOP || code == libc::ENOTDIR)
}

fn c_name(name: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
}

/// `openat(at, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)`.
fn open_dir(at: libc::c_int, name: &std::ffi::CStr) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: `name` is a valid NUL-terminated string that outlives the call.
    let fd = unsafe { libc::openat(at, name.as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` was just opened and is owned by nothing else.
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) })
}

fn fstat_mode(fd: libc::c_int) -> std::io::Result<u32> {
    // SAFETY: an all-zero `stat` is a valid value to be overwritten.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `fd` is open and `stat` is a valid out-pointer.
    if unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // `mode_t` is `u16` on macOS and `u32` on Linux.
    #[allow(clippy::unnecessary_cast)]
    Ok(stat.st_mode as u32)
}

/// The mode of `name` under the opened directory `at`, when it is a real directory
/// (`fstatat` with `AT_SYMLINK_NOFOLLOW`).
fn fstatat_dir_mode(at: libc::c_int, name: &std::ffi::CStr) -> Option<u32> {
    // SAFETY: an all-zero `stat` is a valid value to be overwritten.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `name` is NUL-terminated and `stat` is a valid out-pointer.
    let done = unsafe { libc::fstatat(at, name.as_ptr(), &mut stat, libc::AT_SYMLINK_NOFOLLOW) };
    // `mode_t` is `u16` on macOS and `u32` on Linux.
    #[allow(clippy::unnecessary_cast)]
    let (mode, kind, dir) = (
        stat.st_mode as u32,
        libc::S_IFMT as u32,
        libc::S_IFDIR as u32,
    );
    (done == 0 && mode & kind == dir).then_some(mode)
}

/// The entry names of the opened directory `dir` (not `.` or `..`), read through a
/// duplicate of its descriptor.
fn names(dir: &std::os::fd::OwnedFd) -> std::io::Result<Vec<std::ffi::CString>> {
    use std::os::fd::AsRawFd;
    // SAFETY: `dir` is open; the duplicate is owned by the stream below.
    let dup = unsafe { libc::dup(dir.as_raw_fd()) };
    if dup < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `dup` is an open directory descriptor; on success the stream owns it.
    let stream = unsafe { libc::fdopendir(dup) };
    if stream.is_null() {
        let err = std::io::Error::last_os_error();
        // SAFETY: `dup` is still ours when `fdopendir` failed.
        unsafe { libc::close(dup) };
        return Err(err);
    }
    let mut names = Vec::new();
    loop {
        // SAFETY: `stream` is a valid open directory stream.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        // SAFETY: `d_name` is NUL-terminated within the entry, valid until the next
        // `readdir` on this stream; it is copied at once.
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes() != b"." && name.to_bytes() != b".." {
            names.push(name.to_owned());
        }
    }
    // SAFETY: `stream` is valid and closed once (it closes `dup`).
    unsafe { libc::closedir(stream) };
    Ok(names)
}

/// `fchmodat(at, name, mode, AT_SYMLINK_NOFOLLOW)`: the entry itself, never what it
/// names if it became a link.
fn chmod_at(at: libc::c_int, name: &std::ffi::CStr, mode: u32) -> std::io::Result<()> {
    // SAFETY: `name` is a valid NUL-terminated string that outlives the call.
    let result = match forced_unsupported() {
        true => Err(std::io::Error::from_raw_os_error(libc::EOPNOTSUPP)),
        false => match unsafe {
            libc::fchmodat(
                at,
                name.as_ptr(),
                mode as libc::mode_t,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } {
            0 => Ok(()),
            _ => Err(std::io::Error::last_os_error()),
        },
    };
    let err = match result {
        Ok(()) => return Ok(()),
        Err(err) => err,
    };
    // Fix round 1 (I5): a `fchmodat` that cannot honour `AT_SYMLINK_NOFOLLOW` (glibc
    // before 2.32, or a newer one without `/proc` on a kernel before 6.6; a newer glibc
    // answers `EOPNOTSUPP` only when `name` is a link at that moment) is refused, never
    // retried without the flag: that re-resolves `name`, and a link swapped in between
    // would have its target changed (decision 17). The directory is left as it is, so
    // its removal fails and the checkout is kept: nothing is lost.
    // (`ENOTSUP` and `EOPNOTSUPP` are one value on Linux, two on macOS.)
    let unsupported = err
        .raw_os_error()
        .is_some_and(|code| code == libc::ENOTSUP || code == libc::EOPNOTSUPP);
    if unsupported {
        before_refusal(name);
        return Err(std::io::Error::other(
            "this system cannot change a directory's mode without following a link there; \
             it was left as it is",
        ));
    }
    Err(err)
}

/// A test seam: whether `fchmodat` is to answer as an older Linux does without
/// `AT_SYMLINK_NOFOLLOW` support (`EOPNOTSUPP`).
fn forced_unsupported() -> bool {
    #[cfg(test)]
    return tests::FORCE_UNSUPPORTED.with(std::cell::Cell::get);
    #[cfg(not(test))]
    false
}

/// A test seam: the test's hook, run where the change is refused (a test swaps the
/// entry for a link there, to show nothing follows it).
fn before_refusal(name: &std::ffi::CStr) {
    #[cfg(test)]
    tests::BEFORE_FALLBACK.with(|hook| {
        if let Some(hook) = &*hook.borrow() {
            hook(name);
        }
    });
    let _ = name;
}

#[cfg(test)]
#[path = "checkout_access_tests.rs"]
mod tests;
