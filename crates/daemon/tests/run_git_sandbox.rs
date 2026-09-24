//! Final fix batch F1 (findings C-C1, D-5, and D-2's route): what a sandboxed worker may
//! write in the repository's git common directory, proven under a real macOS seatbelt
//! profile (`/usr/bin/sandbox-exec`) that allows writes only to the worktree and the
//! roots the engine grants, as Claude Code's and Codex's sandboxes do. A commit in the
//! linked task worktree works; writing the shared config, a hook, the base branch,
//! another run's branch, a sibling task's branch, the run branch, or the files of its own
//! git dir that choose its repository and config (`commondir`, `gitdir`,
//! `config.worktree`; fix round 1, N2 and N3) does not. The worker's ordinary git work
//! (fix round 2, R3) succeeds. The same harness with the whole common dir writable (the
//! grant before this fix) lets every write through, which shows the profile is live. Skipped where `sandbox-exec` does not exist.

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
    run: String,
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
    // A sibling task's branch and the run branch, which the worker must not move.
    out(&repo.root, &["branch", &format!("anthrex/{run}/t2"), &base]);
    out(
        &repo.root,
        &["branch", &format!("anthrex/{run}/integration"), &base],
    );

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
        run: run.to_string(),
        repo,
        _wt: wt,
        common,
        task,
        base,
    }
}

/// A worker's ordinary git work, every step of which must succeed under the grant
/// (fix round 2, R3): commits, an amend, a revert, a `reset --hard`, a rebase and an
/// interactive `rebase --exec`, and a conflicted merge concluded with `merge
/// --continue`. `git stash` is not among them: it writes `refs/stash`, which is not
/// granted.
fn work_script(branch: &str) -> String {
    [
        "set -e",
        "printf 'work\\n' > a.txt && git add a.txt && git commit -q -m work",
        "printf 'more\\n' > b.txt && git add b.txt && git commit -q -m more",
        "git commit -q --amend -m 'more, amended'",
        "git revert --no-edit HEAD >/dev/null",
        "git reset -q --hard HEAD~1",
        "git rebase -q HEAD~1 >/dev/null 2>&1",
        "GIT_SEQUENCE_EDITOR=true git rebase -q -i --exec true HEAD~1 >/dev/null 2>&1",
        "git checkout -q --detach HEAD~1",
        "printf 'theirs\\n' > a.txt && git add a.txt && git commit -q -m theirs",
        "theirs=$(git rev-parse HEAD)",
        &format!("git checkout -q {branch}"),
        "printf 'ours\\n' > a.txt && git add a.txt && git commit -q -m ours",
        "if git merge -q --no-edit \"$theirs\" >/dev/null 2>&1; then exit 3; fi",
        "printf 'resolved\\n' > a.txt && git add a.txt",
        "GIT_EDITOR=true git merge --continue >/dev/null",
        "test \"$(git rev-parse HEAD^2)\" = \"$theirs\"",
    ]
    .join("\n")
}

/// What a worker tries: a commit (and a revert, which writes `MERGE_MSG` and more) on
/// its own branch, then writes it must not make.
fn attempts(s: &Setup, profile: &str) -> [bool; 10] {
    let hook = s.common.join("hooks/post-merge");
    let admin = PathBuf::from(out(&s.task, &["rev-parse", "--absolute-git-dir"]));
    let run = s.run.as_str();
    let branch = format!("anthrex/{run}/t1");
    let write = |path: PathBuf| {
        sandboxed(
            profile,
            &s.task,
            &format!("printf 'x\\n' > '{}'", path.display()),
        )
    };
    [
        sandboxed(profile, &s.task, &work_script(&branch)),
        sandboxed(profile, &s.task, "git config core.fsmonitor 'touch /tmp/x'"),
        write(hook),
        sandboxed(profile, &s.task, "git update-ref refs/heads/main HEAD"),
        sandboxed(
            profile,
            &s.task,
            "git update-ref refs/heads/anthrex/other/integration HEAD",
        ),
        sandboxed(
            profile,
            &s.task,
            &format!("git update-ref refs/heads/anthrex/{run}/t2 HEAD"),
        ),
        sandboxed(
            profile,
            &s.task,
            &format!("git update-ref refs/heads/anthrex/{run}/integration HEAD"),
        ),
        write(admin.join("commondir")),
        write(admin.join("gitdir")),
        write(admin.join("config.worktree")),
    ]
}

#[test]
fn a_sandboxed_worker_commits_but_cannot_write_config_hooks_or_other_branches() {
    if !Path::new(SANDBOX_EXEC).exists() {
        eprintln!("skipped: no {SANDBOX_EXEC}");
        return;
    }
    let s = setup("sb01");
    let roots = worker_git_roots(&s.common, "sb01", "t1");
    let granted = worker_git_dirs(&s.common, &s.task, &roots).unwrap();
    assert!(!granted.contains(&s.common), "{granted:?}");
    let mut writable = vec![s.task.clone()];
    writable.extend(granted);

    let [commit, rest @ ..] = attempts(&s, &profile(&writable));
    assert!(
        commit,
        "the worker's ordinary git work failed in its worktree"
    );
    assert_ne!(
        head(&s.task),
        s.base,
        "the commit landed on the task branch"
    );
    let names = [
        "the shared git config",
        "a hook",
        "the base branch",
        "another run's branch",
        "a sibling task's branch",
        "the run branch",
        "its git dir's commondir",
        "its git dir's gitdir",
        "a config.worktree",
    ];
    for (wrote, what) in rest.iter().zip(names) {
        assert!(!wrote, "the worker wrote {what}");
    }
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
    assert_eq!(attempts(&s, &profile(&writable)), [true; 10]);
}

/// The grant is found from the repository's side: a `.git` file the worker pointed at
/// another worktree's git dir changes nothing.
#[test]
fn the_grant_never_follows_the_worktrees_git_file() {
    let s = setup("sb03");
    let roots = worker_git_roots(&s.common, "sb03", "t1");
    let own = PathBuf::from(out(&s.task, &["rev-parse", "--absolute-git-dir"]));
    let other = s.task.with_file_name("users-own");
    out(
        &s.repo.root,
        &["worktree", "add", "-q", "--detach", other.to_str().unwrap()],
    );
    let other_admin = out(&other, &["rev-parse", "--absolute-git-dir"]);
    std::fs::write(s.task.join(".git"), format!("gitdir: {other_admin}\n")).unwrap();
    let granted = worker_git_dirs(&s.common, &s.task, &roots).unwrap();
    assert!(granted.contains(&own.join("index")), "{granted:?}");
    assert!(
        !granted.iter().any(|p| p.starts_with(&other_admin)),
        "{granted:?}"
    );

    // A directory no worktree of the repository names is refused.
    let stray = tempfile::tempdir().unwrap();
    let err = worker_git_dirs(&s.common, stray.path(), &roots).unwrap_err();
    assert!(err.contains("is not a linked worktree of"), "{err}");
}
