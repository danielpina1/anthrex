use daemon::project::{DETECT_TIMEOUT, detect_roots, detect_roots_with, resolve_roots};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tempfile::{TempDir, tempdir};

fn git(dir: &Path, args: &[&OsStr]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed with {status}");
}

fn init_repo() -> TempDir {
    let dir = tempdir().unwrap();
    git(dir.path(), &[OsStr::new("init")]);
    git(
        dir.path(),
        &[
            OsStr::new("commit"),
            OsStr::new("--allow-empty"),
            OsStr::new("-m"),
            OsStr::new("init"),
        ],
    );
    dir
}

fn hanging_git(dir: &Path) -> PathBuf {
    let script = dir.join("hanging-git");
    fs::write(&script, "#!/bin/sh\nsleep 10\n").unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    script
}

struct OwnedHelper {
    pid_file: PathBuf,
    stopped: bool,
}

impl OwnedHelper {
    fn stop(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        self.stopped = true;
        let Ok(pid) = fs::read_to_string(&self.pid_file) else {
            return false;
        };
        let Ok(pid) = pid.trim().parse::<libc::pid_t>() else {
            return false;
        };
        // SAFETY: the wrapper writes only the pid of the background helper it started
        // for this test. The test owns that process and never targets any other pid.
        if unsafe { libc::kill(pid, libc::SIGKILL) } == -1
            && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        {
            return false;
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        (unsafe { libc::kill(pid, 0) }) == -1
            && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
}

impl Drop for OwnedHelper {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[test]
fn plain_directory_is_its_own_root() {
    let dir = tempdir().unwrap();

    assert_eq!(
        detect_roots(dir.path()).project,
        dir.path().canonicalize().unwrap()
    );
}

#[test]
fn repository_root_is_the_checkout() {
    let repo = init_repo();

    assert_eq!(
        detect_roots(repo.path()).project,
        repo.path().canonicalize().unwrap()
    );
}

#[test]
fn subdirectory_maps_to_the_repository_root() {
    let repo = init_repo();
    let subdirectory = repo.path().join("a/b");
    fs::create_dir_all(&subdirectory).unwrap();

    assert_eq!(
        detect_roots(&subdirectory).project,
        repo.path().canonicalize().unwrap()
    );
}

#[test]
fn linked_worktree_maps_to_the_main_checkout() {
    let repo = init_repo();
    let worktree_parent = tempdir().unwrap();
    let worktree = worktree_parent.path().join("wt");
    git(
        repo.path(),
        &[
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-b"),
            OsStr::new("feat"),
            worktree.as_os_str(),
        ],
    );
    let subdirectory = worktree.join("sub");
    fs::create_dir(&subdirectory).unwrap();
    let expected = repo.path().canonicalize().unwrap();

    assert_eq!(detect_roots(&worktree).project, expected);
    assert_eq!(detect_roots(&subdirectory).project, expected);
    assert_ne!(
        detect_roots(&worktree).project,
        worktree.canonicalize().unwrap()
    );
}

#[test]
fn submodule_maps_to_its_own_checkout() {
    let repo = init_repo();
    let submodule_source = init_repo();
    git(
        repo.path(),
        &[
            OsStr::new("-c"),
            OsStr::new("protocol.file.allow=always"),
            OsStr::new("submodule"),
            OsStr::new("add"),
            submodule_source.path().as_os_str(),
            OsStr::new("mods"),
        ],
    );
    let submodule = repo.path().join("mods");

    assert_eq!(
        detect_roots(&submodule).project,
        submodule.canonicalize().unwrap()
    );
}

#[test]
fn bare_repository_falls_back_to_the_directory() {
    let parent = tempdir().unwrap();
    git(
        parent.path(),
        &[
            OsStr::new("init"),
            OsStr::new("--bare"),
            OsStr::new("x.git"),
        ],
    );
    let bare = parent.path().join("x.git");

    assert_eq!(detect_roots(&bare).project, bare.canonicalize().unwrap());
}

#[test]
fn missing_directory_is_returned_unchanged() {
    let missing = Path::new("/definitely/missing/dir");

    assert_eq!(detect_roots(missing).project, missing);
}

#[test]
fn missing_git_falls_back() {
    let repo = init_repo();
    let subdirectory = repo.path().join("a");
    fs::create_dir(&subdirectory).unwrap();

    assert_eq!(
        detect_roots_with(
            OsStr::new("/nonexistent/git"),
            &subdirectory,
            DETECT_TIMEOUT
        )
        .project,
        subdirectory.canonicalize().unwrap()
    );
}

#[test]
fn hanging_git_times_out() {
    let repo = init_repo();
    let scripts = tempdir().unwrap();
    let script = hanging_git(scripts.path());
    let started = Instant::now();

    assert_eq!(
        detect_roots_with(script.as_os_str(), repo.path(), Duration::from_millis(300)).project,
        repo.path().canonicalize().unwrap()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn inherited_stdout_does_not_outlive_the_detection_deadline() {
    let repo = init_repo();
    let scripts = tempdir().unwrap();
    let script = scripts.path().join("inherited-stdout-git");
    let pid_file = scripts.path().join("helper.pid");
    let root = repo.path().canonicalize().unwrap();
    let cwd = root.clone();
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nsleep 30 &\nprintf '%s\\n' \"$!\" > '{}'\nprintf '%s\\n%s\\n' '{}'/'.git' '{}'\nexit 0\n",
            pid_file.display(),
            root.display(),
            root.display(),
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    let mut helper = OwnedHelper {
        pid_file: pid_file.clone(),
        stopped: false,
    };

    let (tx, rx) = mpsc::channel();
    let detector = std::thread::spawn(move || {
        let started = Instant::now();
        let detected = detect_roots_with(script.as_os_str(), &cwd, Duration::from_secs(1));
        tx.send((detected, started.elapsed())).unwrap();
    });
    let helper_deadline = Instant::now() + Duration::from_secs(2);
    while !pid_file.exists() {
        if let Ok((detected, elapsed)) = rx.try_recv() {
            detector.join().unwrap();
            panic!(
                "project detection returned before the wrapper started its helper: detected {detected:?} after {elapsed:?}"
            );
        }
        assert!(
            Instant::now() < helper_deadline,
            "git wrapper did not start its inherited-stdout helper"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let timely = rx.recv_timeout(Duration::from_secs(2));
    let helper_stopped = helper.stop();
    detector.join().unwrap();
    assert!(
        helper_stopped,
        "owned inherited-stdout helper survived cleanup"
    );
    let (detected, elapsed) = timely.expect(
        "project detection stayed blocked on stdout inherited by an exited git child's helper",
    );

    assert_eq!(detected.project, root);
    assert!(elapsed < Duration::from_secs(2), "elapsed: {elapsed:?}");
}

#[tokio::test]
async fn resolve_root_finds_the_repository() {
    let repo = init_repo();
    let subdirectory = repo.path().join("a");
    fs::create_dir(&subdirectory).unwrap();

    assert_eq!(
        resolve_roots(subdirectory).await.project,
        repo.path().canonicalize().unwrap()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn resolve_root_does_not_block_the_runtime() {
    let repo = init_repo();
    let scripts = tempdir().unwrap();
    let script = hanging_git(scripts.path());
    let sleeper = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        Instant::now()
    });

    let resolved = daemon::project::resolve_roots_with(
        script.into_os_string(),
        repo.path().to_path_buf(),
        Duration::from_millis(500),
    )
    .await;
    let resolved_at = Instant::now();
    let sleeper_at = sleeper.await.unwrap();

    assert_eq!(resolved.project, repo.path().canonicalize().unwrap());
    assert!(sleeper_at < resolved_at);
}

#[test]
fn detect_roots_reports_the_worktree_and_the_project() {
    let repo = init_repo();
    let expected = repo.path().canonicalize().unwrap();

    let roots = detect_roots(repo.path());

    assert_eq!(roots.project, expected);
    assert_eq!(roots.worktree, Some(expected));
}

#[test]
fn detect_roots_in_a_linked_worktree_splits_them() {
    let repo = init_repo();
    let worktree_parent = tempdir().unwrap();
    let worktree = worktree_parent.path().join("wt");
    git(
        repo.path(),
        &[
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-b"),
            OsStr::new("feat"),
            worktree.as_os_str(),
        ],
    );
    let expected_project = repo.path().canonicalize().unwrap();
    let expected_worktree = worktree.canonicalize().unwrap();

    let roots = detect_roots(&worktree);

    assert_eq!(roots.project, expected_project);
    assert_eq!(roots.worktree, Some(expected_worktree));
    assert_ne!(roots.project, roots.worktree.unwrap());
}

#[test]
fn detect_roots_outside_a_repository_has_no_worktree() {
    let dir = tempdir().unwrap();

    let roots = detect_roots(dir.path());

    assert_eq!(roots.project, dir.path().canonicalize().unwrap());
    assert_eq!(roots.worktree, None);
}

#[test]
fn detect_roots_with_a_failing_git_has_no_worktree() {
    let repo = init_repo();
    let subdirectory = repo.path().join("a");
    fs::create_dir(&subdirectory).unwrap();

    let roots = detect_roots_with(
        OsStr::new("/nonexistent/git"),
        &subdirectory,
        DETECT_TIMEOUT,
    );

    assert_eq!(roots.project, subdirectory.canonicalize().unwrap());
    assert_eq!(roots.worktree, None);
}
