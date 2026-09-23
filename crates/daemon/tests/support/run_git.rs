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
