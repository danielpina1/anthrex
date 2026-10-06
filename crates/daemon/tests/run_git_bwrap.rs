//! A worker's git under a real bubblewrap sandbox built the way Claude Code's Linux
//! sandbox (sandbox-runtime) builds one: the whole filesystem read-only (`--ro-bind /
//! /`), each writable path that exists bound read-write (one that does not exist is
//! skipped, as sandbox-runtime skips it), and each denied path that exists bound
//! read-only over it.
//!
//! With the exact-file grant macOS uses, a commit fails: `index.lock` does not exist
//! when the sandbox starts, so it is never bound, and the git directory around it is
//! read-only (`Read-only file system`, the bug a worker reported on Ubuntu 26.04). The
//! sandbox-runtime behaviour modelled here is Claude Code 2.1.292's, the version
//! installed on the Ubuntu test host, whose bundled code was read (never run): its
//! "Skipping non-existent write path" and its argument order. With the Linux grant (the
//! git directory whole, its
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
        .args(["--ro-bind", "/", "/", "--dev", "/dev", "--unshare-pid"])
        .args(["--unshare-user", "--proc", "/proc", "--", "true"])
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
    root: PathBuf,
    run: String,
    base: String,
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
        root: repo.root.clone(),
        run: run.to_string(),
        base,
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
        self.command(grant, script).output().unwrap()
    }

    /// The bubblewrap command [`Setup::sandboxed`] runs, to spawn.
    fn command(&self, grant: &WorkerGrant, script: &str) -> Command {
        // The argument order of Claude Code 2.1.292's sandbox-runtime (its bundled
        // `pw` and its filesystem builder): `--new-session --die-with-parent`, then
        // `--ro-bind / /`, every existing `allowWrite` path `--bind` (a missing one is
        // skipped), then every existing `denyWrite` path inside them `--ro-bind`, so a
        // denial is mounted over the grant that holds it; then `--dev /dev`,
        // `--unshare-pid`, `--unshare-user` and `--proc /proc`.
        let mut command = Command::new(BWRAP);
        command.args(["--new-session", "--die-with-parent", "--ro-bind", "/", "/"]);
        let writable = std::iter::once(&self.task).chain(&grant.writable);
        for path in writable.filter(|path| path.exists()) {
            command.arg("--bind").arg(path).arg(path);
        }
        for path in grant.deny.iter().filter(|path| path.exists()) {
            command.arg("--ro-bind").arg(path).arg(path);
        }
        command
            .args(["--dev", "/dev", "--unshare-pid", "--unshare-user"])
            .args(["--proc", "/proc"])
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
            .envs(with_worker_git_config(Vec::new()));
        command
    }

    /// The engine re-preparing the checkout, as it does before each session.
    fn reprepare(&self) {
        prepare_task_worktree(
            real_git(),
            &self.root,
            &format!("anthrex/{}/t1", self.run),
            &self.base,
            &self.task,
            &task_repo_dir(self.data.path(), "t1"),
            T,
        )
        .unwrap();
    }
}

/// Re-review N1: the engine's writes to a denied entry while a sandbox is alive. A
/// rename over `config` on the host detaches the sandbox's read-only bind on it (the
/// kernel detaches mounts on a dentry renamed over), after which `config` resolves
/// through the writable bind of the git directory. So the engine never renames over a
/// denied entry: `config` unchanged is left alone, and one that drifted is rewritten in
/// place. A sandboxed process started before the re-prepare still cannot append to
/// `config` or replace it with a link afterwards.
#[test]
fn a_reprepare_keeps_config_denied_in_a_live_sandbox() {
    require_bwrap!();
    let s = setup("bw04");
    let grant = s.grant(GrantShape::WholeDir);
    assert!(grant.deny.contains(&s.admin.join("config")));
    let flags = tempfile::tempdir().unwrap();
    let ready = s.task.join(".sandbox-ready");
    let go = flags.path().join("go");
    let config = s.admin.join("config");
    let script = format!(
        "touch '{ready}'\n\
         i=0; while [ ! -e '{go}' ]; do i=$((i+1)); [ $i -gt 600 ] && exit 9; sleep 0.05; done\n\
         if printf '[core]\\n\\tfsmonitor = /bin/true\\n' >> '{config}' 2>/dev/null; then echo APPENDED; fi\n\
         if ln -sf /dev/null '{config}' 2>/dev/null; then echo LINKED; fi\n\
         if mv -f '{config}' '{config}.moved' 2>/dev/null; then echo MOVED; fi\n\
         echo DONE",
        ready = ready.display(),
        go = go.display(),
        config = config.display(),
    );
    let child = s
        .command(&grant, &script)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !ready.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the sandbox never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // The config drifted (so the engine must write it), then the engine re-prepares.
    let before = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, format!("{before}# drift\n")).unwrap();
    s.reprepare();
    assert_eq!(std::fs::read_to_string(&config).unwrap(), before);
    std::fs::write(&go, "").unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("DONE"), "{}", shown(&output));
    assert!(!stdout.contains("APPENDED"), "{}", shown(&output));
    assert!(!stdout.contains("LINKED"), "{}", shown(&output));
    assert!(!stdout.contains("MOVED"), "{}", shown(&output));
    let meta = std::fs::symlink_metadata(&config).unwrap();
    assert!(meta.file_type().is_file());
    assert_eq!(std::fs::read_to_string(&config).unwrap(), before);
    // And an unchanged config is not written at all: same inode after a re-prepare.
    use std::os::unix::fs::MetadataExt as _;
    s.reprepare();
    assert_eq!(std::fs::metadata(&config).unwrap().ino(), meta.ino());
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
        ("modules", "sub/config"),
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
        "modules",
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
