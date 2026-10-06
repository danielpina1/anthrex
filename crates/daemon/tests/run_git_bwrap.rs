//! A worker's git under a real bubblewrap sandbox built the way Claude Code's Linux
//! sandbox (sandbox-runtime) builds one: the whole filesystem read-only (`--ro-bind /
//! /`), each writable path that exists bound read-write (one that does not exist is
//! skipped, as sandbox-runtime skips it), and each denied path that exists bound
//! read-only over it.
//!
//! With the exact-file grant macOS uses, a commit fails: `index.lock` does not exist
//! when the sandbox starts, so it is never bound, and the git directory around it is
//! read-only (`Read-only file system`, the bug a worker reported on Ubuntu 26.04 with
//! Claude Code 2.1.291). With the Linux grant (the git directory whole, its
//! configuration denied) the worker's ordinary git work succeeds, every denied entry
//! stays unwritable, nothing of the common directory is written, and the engine then
//! imports the worker's commit.
//!
//! Linux only, and only where `bwrap` can make a sandbox; skipped with the reason
//! printed otherwise.

mod support;

use daemon::run::git::{GrantShape, WorkerGrant, prepare_task_worktree, sync, worker_git_grant};
use daemon::run::role_launch::{task_repo_dir, with_worker_git_config, worker_git_roots};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use support::run_git::{T, commit_file, out, real_git, repo, wt_dir};

const BWRAP: &str = "bwrap";

/// Why bubblewrap cannot be used here, or `None` when it can.
fn unavailable() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return Some("not Linux".to_string());
    }
    match Command::new(BWRAP)
        .args(["--ro-bind", "/", "/", "--dev", "/dev", "--", "true"])
        .output()
    {
        Err(err) => Some(format!("{BWRAP} cannot be run: {err}")),
        Ok(output) if !output.status.success() => Some(format!(
            "{BWRAP} cannot make a sandbox here: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Ok(_) => None,
    }
}

macro_rules! require_bwrap {
    () => {
        if let Some(reason) = unavailable() {
            eprintln!("skipped: {reason}");
            return;
        }
    };
}

struct Setup {
    _repo: support::TempRepo,
    _wt: tempfile::TempDir,
    data: tempfile::TempDir,
    common: PathBuf,
    task: PathBuf,
    admin: PathBuf,
}

fn setup(run: &str) -> Setup {
    let repo = repo();
    let base = commit_file(&repo.root, "f.txt", "base\n", "base");
    let (wt, wt_path) = wt_dir();
    let data = tempfile::tempdir().unwrap();
    let task = wt_path.join(format!("runs/{run}/t1"));
    prepare_task_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/t1"),
        &base,
        &task,
        &task_repo_dir(data.path(), "t1"),
        T,
    )
    .unwrap();
    let common = PathBuf::from(out(
        &repo.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .canonicalize()
    .unwrap();
    let admin = PathBuf::from(out(&task, &["rev-parse", "--absolute-git-dir"]))
        .canonicalize()
        .unwrap();
    Setup {
        _repo: repo,
        _wt: wt,
        data,
        common,
        task,
        admin,
    }
}

impl Setup {
    fn grant(&self, shape: GrantShape) -> WorkerGrant {
        let roots = worker_git_roots(self.data.path(), "t1");
        worker_git_grant(&self.common, &self.task, &roots, shape).unwrap()
    }

    /// Runs `script` with `sh -c` in the task checkout, under bubblewrap with the
    /// worktree and `grant` writable and `grant.deny` read-only, with the worker's git
    /// environment.
    fn sandboxed(&self, grant: &WorkerGrant, script: &str) -> Output {
        let mut command = Command::new(BWRAP);
        command.args(["--die-with-parent", "--ro-bind", "/", "/", "--dev", "/dev"]);
        let writable = std::iter::once(&self.task).chain(&grant.writable);
        for path in writable.filter(|path| path.exists()) {
            command.arg("--bind").arg(path).arg(path);
        }
        for path in grant.deny.iter().filter(|path| path.exists()) {
            command.arg("--ro-bind").arg(path).arg(path);
        }
        command
            .arg("--chdir")
            .arg(&self.task)
            .args(["--", "sh", "-c", script])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .envs(with_worker_git_config(Vec::new()))
            .output()
            .unwrap()
    }
}

/// A worker's ordinary git work on its detached `HEAD`: commits, an amend, a revert, a
/// `reset --hard`, a rebase and an interactive `rebase --exec`, and a conflicted merge
/// concluded with `merge --continue` (as `run_git_sandbox`'s macOS work script).
fn work_script() -> String {
    [
        "set -e",
        "printf 'work\\n' > a.txt && git add a.txt && git commit -q -m work",
        "printf 'more\\n' > b.txt && git add b.txt && git commit -q -m more",
        "git commit -q --amend -m 'more, amended'",
        "git revert --no-edit HEAD >/dev/null",
        "git reset -q --hard HEAD~1",
        "git rebase -q HEAD~1 >/dev/null 2>&1",
        "GIT_SEQUENCE_EDITOR=true git rebase -q -i --exec true HEAD~1 >/dev/null 2>&1",
        "ours=$(git rev-parse HEAD)",
        "git checkout -q --detach HEAD~1",
        "printf 'theirs\\n' > a.txt && git add a.txt && git commit -q -m theirs",
        "theirs=$(git rev-parse HEAD)",
        "git checkout -q --detach \"$ours\"",
        "printf 'ours\\n' > a.txt && git add a.txt && git commit -q -m ours",
        "if git merge -q --no-edit \"$theirs\" >/dev/null 2>&1; then exit 3; fi",
        "printf 'resolved\\n' > a.txt && git add a.txt",
        "GIT_EDITOR=true git merge --continue >/dev/null",
        "test \"$(git rev-parse HEAD^2)\" = \"$theirs\"",
    ]
    .join("\n")
}

fn shown(output: &Output) -> String {
    format!(
        "status {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The bug: under the exact-file grant a `git add` cannot create `index.lock`.
#[test]
fn the_exact_file_grant_cannot_commit_under_bubblewrap() {
    require_bwrap!();
    let s = setup("bw01");
    let grant = s.grant(GrantShape::Files);
    let output = s.sandboxed(
        &grant,
        "printf 'work\\n' > a.txt && git add a.txt && git commit -q -m work",
    );
    assert!(!output.status.success(), "{}", shown(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("index.lock") && stderr.contains("Read-only file system"),
        "{}",
        shown(&output)
    );
}

/// The fix: under the Linux grant the worker's git work succeeds, and the engine
/// imports its commit.
#[test]
fn the_linux_grant_commits_under_bubblewrap() {
    require_bwrap!();
    let s = setup("bw02");
    let grant = s.grant(GrantShape::WholeDir);
    let output = s.sandboxed(&grant, &work_script());
    assert!(output.status.success(), "{}", shown(&output));
    let head = out(&s.task, &["rev-parse", "HEAD"]);
    assert_eq!(sync(real_git(), &s.task, T).unwrap(), head);
}

/// Under the Linux grant every denied entry of the git directory, and the common
/// directory, stay unwritable: no write, no new file inside a denied directory, no
/// replacement by a link.
#[test]
fn the_linux_grant_keeps_the_denied_entries_unwritable() {
    require_bwrap!();
    let s = setup("bw03");
    let grant = s.grant(GrantShape::WholeDir);
    let before = snapshot(&s.admin, &s.common);
    let mut attempts = Vec::new();
    for name in [
        "config",
        "config.worktree",
        "locked",
        "gitdir",
        "packed-refs",
    ] {
        let path = s.admin.join(name);
        attempts.push(format!("printf 'x\\n' > '{}'", path.display()));
        attempts.push(format!("rm -f '{}'", path.display()));
        attempts.push(format!(
            "mv '{}' '{}.moved'",
            path.display(),
            path.display()
        ));
    }
    for (dir, file) in [
        ("logs", "HEAD"),
        ("refs", "heads/x"),
        ("info", "exclude"),
        ("info", "attributes"),
        ("hooks", "pre-commit"),
    ] {
        let path = s.admin.join(dir).join(file);
        attempts.push(format!(
            "mkdir -p '{}' && printf 'x\\n' > '{}'",
            path.parent().unwrap().display(),
            path.display()
        ));
        attempts.push(format!("rm -rf '{}'", s.admin.join(dir).display()));
    }
    for name in ["config", "HEAD", "packed-refs", "hooks/post-merge"] {
        let path = s.common.join(name);
        attempts.push(format!("printf 'x\\n' > '{}'", path.display()));
    }
    attempts.push(format!(
        "printf 'x\\n' > '{}'",
        s.common.join("refs/heads/main").display()
    ));
    for attempt in &attempts {
        let output = s.sandboxed(&grant, attempt);
        assert!(!output.status.success(), "allowed: {attempt}");
    }
    assert_eq!(snapshot(&s.admin, &s.common), before);
}

/// Every file under the git directory's denied entries and the common directory (the
/// object store aside), with its content.
fn snapshot(admin: &Path, common: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    let mut files = Vec::new();
    let mut walk = vec![common.to_path_buf()];
    for name in [
        "config",
        "config.worktree",
        "locked",
        "gitdir",
        "packed-refs",
        "logs",
        "refs",
        "info",
        "hooks",
    ] {
        walk.push(admin.join(name));
    }
    while let Some(path) = walk.pop() {
        if path.starts_with(common.join("objects")) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            files.push((path, None));
            continue;
        };
        if meta.is_dir() {
            files.push((path.clone(), None));
            for entry in std::fs::read_dir(&path).unwrap().flatten() {
                walk.push(entry.path());
            }
        } else {
            files.push((path.clone(), std::fs::read(&path).ok()));
        }
    }
    files.sort();
    files
}
