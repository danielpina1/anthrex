//! The review of the Linux worker-git fix, I1: no file the daemon writes into a task
//! checkout's git directory is ever created there. A Linux worker's grant is that
//! directory whole, so a temporary file made in it could be opened by a worker still
//! running, which then holds a descriptor to the inode the engine renames over `config`
//! (or `index`, or `MERGE_HEAD`) and writes through it after the rename, past the
//! read-only bind that pins only the old inode. Each such file is made in the
//! checkout's engine directory, which no grant names, and renamed into place.
//!
//! Two tests: a portable one (a directory planted at each old temporary name, which
//! made the engine's write fail and is now never touched), and on Linux an inotify
//! watch on the git directory while the engine re-prepares the checkout, hands a
//! conflict back (merge state and index) and re-detaches it: nothing is created there,
//! only renamed in.

mod support;

use daemon::run::git::{hand_back, prepare_worktree};
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, wt_dir};

struct Setup {
    repo: TempRepo,
    _wt: tempfile::TempDir,
    task: PathBuf,
    admin: PathBuf,
    base: String,
    run_head: String,
}

/// A task checkout with a commit that conflicts with the run branch's.
fn setup(run: &str) -> Setup {
    let repo = repo();
    commit_file(&repo.root, "shared.txt", "base\n", "shared");
    let base = head(&repo.root);
    let (wt, wt_path) = wt_dir();
    let task = wt_path.join(format!("runs/{run}/t1"));
    prepare(&repo.root, run, &base, &task).unwrap();
    commit_file(&task, "shared.txt", "task\n", "task work");
    let back = out(&repo.root, &["symbolic-ref", "--short", "HEAD"]);
    out(
        &repo.root,
        &[
            "checkout",
            "-q",
            "-b",
            &format!("anthrex/{run}/integration"),
        ],
    );
    let run_head = commit_file(&repo.root, "shared.txt", "run\n", "run work");
    out(&repo.root, &["checkout", "-q", &back]);
    let admin = PathBuf::from(out(&task, &["rev-parse", "--absolute-git-dir"]))
        .canonicalize()
        .unwrap();
    Setup {
        repo,
        _wt: wt,
        task,
        admin,
        base,
        run_head,
    }
}

fn prepare(root: &Path, run: &str, base: &str, task: &Path) -> Result<String, String> {
    prepare_worktree(
        real_git(),
        root,
        &format!("anthrex/{run}/t1"),
        base,
        task,
        T,
    )
}

/// The engine's writes into the git directory: the checkout re-prepared (`config`,
/// `info/exclude`), a conflicted hand-back (`MERGE_MSG`, `MERGE_MODE`, `MERGE_HEAD`,
/// and the engine's index installed as `index`).
fn engine_writes(s: &Setup, run: &str) {
    prepare(&s.repo.root, run, &s.base, &s.task).unwrap();
    let back = hand_back(real_git(), &s.task, &s.run_head, T).unwrap();
    assert_eq!(back.files, vec!["shared.txt".to_string()]);
    assert!(
        try_git(&s.task, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success()
    );
}

/// The old temporary names, each planted as a non-empty directory: before the fix the
/// engine's `put` failed to clear it; now it never looks there.
#[test]
fn a_planted_temporary_name_in_the_git_dir_is_never_touched() {
    let s = setup("st01");
    let names = [
        "anthrex-config.tmp",
        "anthrex-MERGE_MSG.tmp",
        "anthrex-MERGE_MODE.tmp",
        "anthrex-MERGE_HEAD.tmp",
        "anthrex-HEAD.tmp",
        "info/anthrex-exclude.tmp",
    ];
    for name in names {
        let dir = s.admin.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("planted"), "x\n").unwrap();
    }
    engine_writes(&s, "st01");
    for name in names {
        assert_eq!(
            std::fs::read_to_string(s.admin.join(name).join("planted")).unwrap(),
            "x\n",
            "{name}"
        );
    }
}

/// Linux: inotify sees nothing created in the git directory (or its `info/`) while the
/// engine writes there; each file is renamed in.
#[test]
fn the_engine_creates_no_file_in_the_git_dir() {
    if !cfg!(target_os = "linux") {
        eprintln!("skipped: not Linux (inotify)");
        return;
    }
    let s = setup("st02");
    let watch = inotify::Watch::new(&[&s.admin, &s.admin.join("info")]);
    engine_writes(&s, "st02");
    let events = watch.events();
    let created: Vec<&String> = events
        .iter()
        .filter(|(kind, _)| *kind == "create")
        .map(|(_, name)| name)
        .collect();
    assert!(
        created.is_empty(),
        "the engine created files in the git dir: {created:?}"
    );
    let moved: Vec<&str> = events
        .iter()
        .filter(|(kind, _)| *kind == "moved_to")
        .map(|(_, name)| name.as_str())
        .collect();
    for name in ["config", "exclude", "MERGE_HEAD", "MERGE_MSG", "index"] {
        assert!(
            moved.contains(&name),
            "{name} was not renamed in: {events:?}"
        );
    }
}

#[cfg(target_os = "linux")]
mod inotify {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    use std::path::Path;

    pub struct Watch {
        fd: libc::c_int,
    }

    impl Watch {
        pub fn new(dirs: &[&Path]) -> Watch {
            // SAFETY: plain syscalls; results checked.
            let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
            assert!(
                fd >= 0,
                "inotify_init1: {}",
                std::io::Error::last_os_error()
            );
            for dir in dirs {
                let c = CString::new(dir.as_os_str().as_bytes()).unwrap();
                // SAFETY: a valid descriptor and NUL-terminated path.
                let wd = unsafe {
                    libc::inotify_add_watch(fd, c.as_ptr(), libc::IN_CREATE | libc::IN_MOVED_TO)
                };
                assert!(
                    wd >= 0,
                    "inotify_add_watch: {}",
                    std::io::Error::last_os_error()
                );
            }
            Watch { fd }
        }

        /// Every event so far: ("create" | "moved_to", name).
        pub fn events(&self) -> Vec<(&'static str, String)> {
            let mut events = Vec::new();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                // SAFETY: reading into an owned buffer of its length.
                let n = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
                if n <= 0 {
                    break;
                }
                let mut at = 0usize;
                let header = std::mem::size_of::<libc::inotify_event>();
                while at + header <= n as usize {
                    // SAFETY: the kernel wrote a whole event at `at`.
                    let event: libc::inotify_event =
                        unsafe { std::ptr::read_unaligned(buf[at..].as_ptr().cast()) };
                    let name_bytes = &buf[at + header..at + header + event.len as usize];
                    let end = name_bytes.iter().position(|b| *b == 0).unwrap_or(0);
                    let name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
                    let kind = if event.mask & libc::IN_CREATE != 0 {
                        "create"
                    } else {
                        "moved_to"
                    };
                    events.push((kind, name));
                    at += header + event.len as usize;
                }
            }
            events
        }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            // SAFETY: closing the descriptor this watch owns.
            unsafe { libc::close(self.fd) };
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod inotify {
    use std::path::Path;

    pub struct Watch;

    impl Watch {
        pub fn new(_: &[&Path]) -> Watch {
            unreachable!("Linux only")
        }

        pub fn events(&self) -> Vec<(&'static str, String)> {
            unreachable!("Linux only")
        }
    }
}
