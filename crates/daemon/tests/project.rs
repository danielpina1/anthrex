use daemon::project::{DETECT_TIMEOUT, detect_root, detect_root_with, resolve_root};
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
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

#[test]
fn plain_directory_is_its_own_root() {
    let dir = tempdir().unwrap();

    assert_eq!(detect_root(dir.path()), dir.path().canonicalize().unwrap());
}

#[test]
fn repository_root_is_the_checkout() {
    let repo = init_repo();

    assert_eq!(
        detect_root(repo.path()),
        repo.path().canonicalize().unwrap()
    );
}

#[test]
fn subdirectory_maps_to_the_repository_root() {
    let repo = init_repo();
    let subdirectory = repo.path().join("a/b");
    fs::create_dir_all(&subdirectory).unwrap();

    assert_eq!(
        detect_root(&subdirectory),
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

    assert_eq!(detect_root(&worktree), expected);
    assert_eq!(detect_root(&subdirectory), expected);
    assert_ne!(detect_root(&worktree), worktree.canonicalize().unwrap());
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

    assert_eq!(detect_root(&submodule), submodule.canonicalize().unwrap());
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

    assert_eq!(detect_root(&bare), bare.canonicalize().unwrap());
}

#[test]
fn missing_directory_is_returned_unchanged() {
    let missing = Path::new("/definitely/missing/dir");

    assert_eq!(detect_root(missing), missing);
}

#[test]
fn missing_git_falls_back() {
    let repo = init_repo();
    let subdirectory = repo.path().join("a");
    fs::create_dir(&subdirectory).unwrap();

    assert_eq!(
        detect_root_with(
            OsStr::new("/nonexistent/git"),
            &subdirectory,
            DETECT_TIMEOUT
        ),
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
        detect_root_with(script.as_os_str(), repo.path(), Duration::from_millis(300)),
        repo.path().canonicalize().unwrap()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn resolve_root_finds_the_repository() {
    let repo = init_repo();
    let subdirectory = repo.path().join("a");
    fs::create_dir(&subdirectory).unwrap();

    assert_eq!(
        resolve_root(subdirectory).await,
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

    let resolved = daemon::project::resolve_root_with(
        script.into_os_string(),
        repo.path().to_path_buf(),
        Duration::from_millis(500),
    )
    .await;
    let resolved_at = Instant::now();
    let sleeper_at = sleeper.await.unwrap();

    assert_eq!(resolved, repo.path().canonicalize().unwrap());
    assert!(sleeper_at < resolved_at);
}
