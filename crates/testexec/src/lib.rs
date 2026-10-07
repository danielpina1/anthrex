//! Stand-in executables for tests that a concurrent fork can never leave busy.
//!
//! Linux refuses to exec a file that any process holds open for writing (`ETXTBSY`,
//! "executable file busy"). A test binary runs its tests on many threads, and every
//! `Command::spawn` forks: the child holds a copy of every descriptor the process had
//! at that instant until it execs, `O_CLOEXEC` or not. So a test that writes a script
//! with `fs::write` while another test forks can find its own script busy when it, or
//! the daemon code it drives, execs it a moment later.
//!
//! Renaming the file into place does not help: the inherited descriptor refers to the
//! inode, not the name, and the inode is the one that is exec'd. What does help is
//! never holding a write descriptor to it in the test process at all. [`write_executable`]
//! has `/bin/sh` write the bytes in a child of its own, under a staging name, waits for
//! that child to exit, and renames the finished file into place (so a script that is
//! being replaced is never seen half-written). No fork of the test process can inherit
//! a descriptor the test process never had.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `body` to `path` with mode 0755, as [`write_executable_mode`].
pub fn write_executable(path: impl AsRef<Path>, body: impl AsRef<[u8]>) -> PathBuf {
    write_executable_mode(path, body, 0o755)
}

/// Writes `body` to `path` with `mode`, without this process ever opening `path` (or the
/// staging file renamed onto it) for writing, so the file can be exec'd at once. Panics
/// when the write fails: this is test support.
pub fn write_executable_mode(path: impl AsRef<Path>, body: impl AsRef<[u8]>, mode: u32) -> PathBuf {
    write_executable_with(path, body, mode, || {})
}

/// [`write_executable_mode`], calling `while_writing` while the writer child is alive
/// and its input is written, so a test can fork at the moment a write is in flight.
#[doc(hidden)]
pub fn write_executable_with(
    path: impl AsRef<Path>,
    body: impl AsRef<[u8]>,
    mode: u32,
    while_writing: impl FnOnce(),
) -> PathBuf {
    let path = path.as_ref();
    let staged = staging_name(path);
    let mut writer = Command::new("/bin/sh")
        .args([
            "-c",
            "umask 077 && cat > \"$1\" && chmod \"$2\" \"$1\"",
            "sh",
        ])
        .arg(&staged)
        .arg(format!("{mode:o}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start /bin/sh to write {}: {e}", path.display()));
    let mut input = writer.stdin.take().expect("piped stdin");
    input
        .write_all(body.as_ref())
        .unwrap_or_else(|e| panic!("cannot pass {} to its writer: {e}", path.display()));
    drop(input);
    while_writing();
    let status = writer.wait().expect("wait for the writer");
    assert!(
        status.success(),
        "writing {} failed: {status}",
        staged.display()
    );
    std::fs::rename(&staged, path).unwrap_or_else(|e| {
        panic!(
            "cannot rename {} to {}: {e}",
            staged.display(),
            path.display()
        )
    });
    path.to_path_buf()
}

/// A name next to `path`, unique in this process, for the file before it is renamed.
fn staging_name(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .unwrap_or_else(|| panic!("{} has no file name", path.display()))
        .to_string_lossy();
    path.with_file_name(format!(
        ".{name}.{}.{}.staged",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// A child of this process held between `fork` and `exec` until it is released: the
/// exact state in which a concurrent `spawn` on another test thread leaves every
/// descriptor this process has open, made to last as long as a test needs.
pub struct StalledFork {
    release: libc::c_int,
    spawner: Option<std::thread::JoinHandle<()>>,
}

impl StalledFork {
    /// Forks now and returns once the child is running, before it execs `true`.
    pub fn start() -> Self {
        use std::os::unix::process::CommandExt;
        let (ready_read, ready_write) = cloexec_pipe();
        let (release_read, release_write) = cloexec_pipe();
        let mut command = Command::new("true");
        // SAFETY: the closure only calls `write` and `read`, both async-signal-safe.
        unsafe {
            command.pre_exec(move || {
                let byte = 1u8;
                libc::write(ready_write, (&raw const byte).cast(), 1);
                let mut got = 0u8;
                libc::read(release_read, (&raw mut got).cast(), 1);
                Ok(())
            });
        }
        // `spawn` returns only once the child has exec'd, so it runs on its own thread.
        let spawner = std::thread::spawn(move || {
            let status = command.spawn().and_then(|mut child| child.wait());
            assert!(
                matches!(&status, Ok(s) if s.success()),
                "the stalled fork: {status:?}"
            );
        });
        let mut got = 0u8;
        // SAFETY: `ready_read` is a pipe this function opened; one byte is read into `got`.
        let n = unsafe { libc::read(ready_read, (&raw mut got).cast(), 1) };
        assert_eq!(n, 1, "the stalled fork never reported in");
        // SAFETY: closing descriptors this function opened and no longer uses here.
        unsafe {
            libc::close(ready_read);
            libc::close(ready_write);
            libc::close(release_read);
        }
        Self {
            release: release_write,
            spawner: Some(spawner),
        }
    }

    /// Lets the child exec, which closes everything it inherited, and waits for it.
    pub fn release(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        if let Some(spawner) = self.spawner.take() {
            let byte = 1u8;
            // SAFETY: `release` is the write end of a pipe this value owns.
            unsafe {
                libc::write(self.release, (&raw const byte).cast(), 1);
                libc::close(self.release);
            }
            spawner.join().expect("the stalled fork's spawner");
        }
    }
}

impl Drop for StalledFork {
    fn drop(&mut self) {
        self.finish();
    }
}

/// A pipe whose ends close on exec, so no other test's child keeps them past its exec.
fn cloexec_pipe() -> (libc::c_int, libc::c_int) {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` has room for the two descriptors `pipe` writes.
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe");
    for fd in fds {
        // SAFETY: `fd` was just opened by `pipe`.
        assert_eq!(
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
            0
        );
    }
    (fds[0], fds[1])
}
