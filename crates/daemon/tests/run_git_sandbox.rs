//! Final fix batch F1 (findings C-C1, D-5, and D-2's route): what a sandboxed worker may
//! write in the repository's git common directory, proven under a real macOS seatbelt
//! profile (`/usr/bin/sandbox-exec`) that allows writes only to the worktree and the
//! roots the engine grants, as Claude Code's and Codex's sandboxes do. A commit in the
//! linked task worktree works; writing the shared config, a hook, the base branch or
//! another run's branch does not. The same harness with the whole common dir writable
//! (the grant before this fix) lets every one of those through, which shows the profile
//! is live. Skipped where `sandbox-exec` does not exist.

mod support;

use daemon::run::git::{prepare_worktree, worker_git_dirs};
use daemon::run::role_launch::worker_git_roots;
use std::path::{Path, PathBuf};
use std::process::Command;
use support::run_git::{T, head, out, real_git, repo, try_git, wt_dir};

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// A seatbelt profile: everything allowed except file writes, which are allowed only
/// below `writable` (and to `/dev`, for `/dev/null`).
fn profile(writable: &[PathBuf]) -> String {
    let mut rules = String::from("(version 1)\n(allow default)\n(deny file-write*)\n");
    rules.push_str("(allow file-write* (subpath \"/dev\")");
    for path in writable {
        rules.push_str(&format!(" (subpath {:?})", path.display().to_string()));
    }
    rules.push_str(")\n");
    rules
}

/// Runs `script` with `sh -c` in `dir` under `profile`; whether it exited 0.
fn sandboxed(profile: &str, dir: &Path, script: &str) -> bool {
    let output = Command::new(SANDBOX_EXEC)
        .args(["-p", profile, "sh", "-c", script])
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    output.status.success()
}

struct Setup {
    repo: support::TempRepo,
    _wt: tempfile::TempDir,
    common: PathBuf,
    task: PathBuf,
    base: String,
}

fn setup(run: &str) -> Setup {
    let repo = repo();
    let base = head(&repo.root);
    let (wt, wt_path) = wt_dir();
    let task = wt_path.join(format!("runs/{run}/t1"));
    prepare_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/t1"),
        &base,
        &task,
        T,
    )
    .unwrap();
    // Another run's branch, which this run's worker must not move either.
    out(&repo.root, &["branch", "anthrex/other/integration", &base]);
    // A `pack-refs` removes the loose refs and their now empty directories: the
    // engine must still grant (and so create) the run's branch directories.
    out(&repo.root, &["pack-refs", "--all"]);
    let common = PathBuf::from(out(
        &repo.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .canonicalize()
    .unwrap();
    Setup {
        repo,
        _wt: wt,
        common,
        task,
        base,
    }
}

/// What a worker tries: a commit on its own branch, then four writes it must not make.
fn attempts(s: &Setup, profile: &str) -> [bool; 5] {
    let hook = s.common.join("hooks/post-merge");
    [
        sandboxed(
            profile,
            &s.task,
            "printf 'work\\n' > a.txt && git add a.txt && git commit -q -m work",
        ),
        sandboxed(profile, &s.task, "git config core.fsmonitor 'touch /tmp/x'"),
        sandboxed(
            profile,
            &s.task,
            &format!("printf '#!/bin/sh\\n' > '{}'", hook.display()),
        ),
        sandboxed(profile, &s.task, "git update-ref refs/heads/main HEAD"),
        sandboxed(
            profile,
            &s.task,
            "git update-ref refs/heads/anthrex/other/integration HEAD",
        ),
    ]
}

#[test]
fn a_sandboxed_worker_commits_but_cannot_write_config_hooks_or_other_branches() {
    if !Path::new(SANDBOX_EXEC).exists() {
        eprintln!("skipped: no {SANDBOX_EXEC}");
        return;
    }
    let s = setup("sb01");
    let roots = worker_git_roots(&s.common, "sb01");
    let granted = worker_git_dirs(real_git(), &s.common, &s.task, &roots, T).unwrap();
    assert!(
        granted.iter().all(|dir| dir.is_dir()),
        "every granted root exists: {granted:?}"
    );
    assert!(!granted.contains(&s.common), "{granted:?}");
    let mut writable = vec![s.task.clone()];
    writable.extend(granted);

    let [commit, config, hook, base, other] = attempts(&s, &profile(&writable));
    assert!(commit, "the worker could not commit in its worktree");
    assert_ne!(
        head(&s.task),
        s.base,
        "the commit landed on the task branch"
    );
    assert!(!config, "the worker wrote the shared git config");
    assert!(!hook, "the worker wrote a hook");
    assert!(!base, "the worker moved the base branch");
    assert!(!other, "the worker moved another run's branch");
    assert_eq!(out(&s.repo.root, &["rev-parse", "main"]), s.base);
    assert!(
        !try_git(&s.repo.root, &["config", "--get", "core.fsmonitor"])
            .status
            .success()
    );
}

#[test]
fn the_whole_common_dir_writable_lets_every_write_through() {
    if !Path::new(SANDBOX_EXEC).exists() {
        eprintln!("skipped: no {SANDBOX_EXEC}");
        return;
    }
    let s = setup("sb02");
    let writable = vec![s.task.clone(), s.common.clone()];
    assert_eq!(attempts(&s, &profile(&writable)), [true; 5]);
}

#[test]
fn a_worktree_whose_git_file_points_elsewhere_is_refused() {
    let s = setup("sb03");
    let elsewhere = tempfile::tempdir().unwrap();
    out(elsewhere.path(), &["init", "-q", "."]);
    std::fs::write(
        s.task.join(".git"),
        format!("gitdir: {}\n", elsewhere.path().join(".git").display()),
    )
    .unwrap();
    let roots = worker_git_roots(&s.common, "sb03");
    let err = worker_git_dirs(real_git(), &s.common, &s.task, &roots, T).unwrap_err();
    assert!(err.contains("is not a linked worktree of"), "{err}");

    // Nor may it borrow another worktree's administrative directory (the user's own
    // linked checkout, say), whose `HEAD` and index the grant would make writable.
    let other = s.task.with_file_name("users-own");
    out(
        &s.repo.root,
        &["worktree", "add", "-q", "--detach", other.to_str().unwrap()],
    );
    let other_admin = out(&other, &["rev-parse", "--absolute-git-dir"]);
    std::fs::write(s.task.join(".git"), format!("gitdir: {other_admin}\n")).unwrap();
    let err = worker_git_dirs(real_git(), &s.common, &s.task, &roots, T).unwrap_err();
    assert!(err.contains("belongs to"), "{err}");
}
