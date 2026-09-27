//! Helpers for the M8a.8 run-git tests (`tests/run_git*.rs`): a `TempRepo` with a
//! repository-local identity, and short ways to run git in it and read the answer.

use super::{TempRepo, git_output};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The timeout every run-git call in these tests gets: generous, since these tests
/// assert what the calls did, never how long they took.
pub const T: Duration = Duration::from_secs(30);

pub fn real_git() -> &'static OsStr {
    OsStr::new("git")
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// Runs git in `dir`, asserting success; stdout, trimmed.
pub fn out(dir: &Path, args: &[&str]) -> String {
    let output = try_git(dir, args);
    assert!(
        output.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

pub fn try_git(dir: &Path, args: &[&str]) -> std::process::Output {
    let owned = os(args);
    let refs: Vec<&OsStr> = owned.iter().map(OsString::as_os_str).collect();
    git_output(dir, &refs)
}

/// A `TempRepo` whose repository-local config names a committer, so preflight's
/// identity check passes whatever the machine's global config holds.
pub fn repo() -> TempRepo {
    identified(TempRepo::new())
}

pub fn identified(repo: TempRepo) -> TempRepo {
    out(&repo.root, &["config", "user.name", "Run Tester"]);
    out(&repo.root, &["config", "user.email", "run@tester.test"]);
    repo
}

/// Writes `path` (creating its directories) with `content`, commits it with `message`
/// and returns the new `HEAD`.
pub fn commit_file(dir: &Path, path: &str, content: &str, message: &str) -> String {
    write(dir, path, content);
    out(dir, &["add", "--", path]);
    out(dir, &["commit", "-q", "-m", message]);
    head(dir)
}

pub fn write(dir: &Path, path: &str, content: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, content).unwrap();
}

pub fn head(dir: &Path) -> String {
    out(dir, &["rev-parse", "HEAD"])
}

/// A fresh, canonical directory for engine worktrees (`<wt>` in decision 16).
pub fn wt_dir() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    (dir, path)
}

/// `path`'s block of `git worktree list --porcelain` in `root`, if git knows it.
pub fn worktree_block(root: &Path, path: &Path) -> Option<String> {
    let list = out(root, &["worktree", "list", "--porcelain"]);
    let wanted = format!("worktree {}", path.display());
    list.split("\n\n")
        .find(|block| block.lines().next() == Some(wanted.as_str()))
        .map(str::to_string)
}

/// A `git` stand-in in `dir` that runs the shell `before` (with `$REAL` the real git's
/// absolute path, and the stand-in's own arguments in `$@`, so `$2` is the `-C`
/// directory every run-git call passes first), then execs the real git. It lets a test
/// make something happen at an exact point inside an operation — another writer's ref
/// appearing just before an `update-ref`, a commit landing on the base just before a
/// merge — with no timing involved.
pub fn wrapper_git(dir: &Path, before: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let which = std::process::Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(which.stdout).unwrap().trim().to_string();
    assert!(!real.is_empty(), "no git on PATH");
    let script = dir.join("wrapper-git");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ -n \"$ANTHREX_TEST_SETTLE\" ] && exit 0\nREAL='{real}'\n{before}\nexec \"$REAL\" \"$@\"\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    settle(&script);
    script
}

/// Waits until `script` can be executed. On Linux a fork in another test thread can
/// briefly inherit the fd that wrote it, and exec then fails with ETXTBSY; the probe
/// run (`ANTHREX_TEST_SETTLE` set) exits before the script does anything.
fn settle(script: &Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match std::process::Command::new(script)
            .env("ANTHREX_TEST_SETTLE", "1")
            .status()
        {
            Ok(status) => {
                assert!(status.success(), "the settle probe failed: {status}");
                return;
            }
            Err(e) if e.raw_os_error() == Some(26) && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("{} does not run: {e}", script.display()),
        }
    }
}

/// Final fix batch F1b: a task worktree's `HEAD` is detached (a plain commit id), never
/// on the task's branch, which only the engine moves.
pub fn assert_detached(dir: &Path) {
    assert!(
        !try_git(dir, &["symbolic-ref", "-q", "HEAD"])
            .status
            .success(),
        "{}'s HEAD is on a branch; a task worktree is detached",
        dir.display()
    );
}
