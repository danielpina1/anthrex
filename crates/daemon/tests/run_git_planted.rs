//! Final fix batch F1 (findings C-C1 and D-5): a sandboxed worker used to be able to
//! write the repository's shared `.git/config` and `.git/hooks/`, and the daemon then ran
//! what it planted, unsandboxed: a `core.fsmonitor` command on every engine or probe
//! `git status`, and a hook inside `run accept`'s merge in the user's own checkout.
//! Every daemon git call now passes `-c core.fsmonitor=false`, and every engine git call
//! (reads and accept included) `-c core.hooksPath=/dev/null`.

mod support;

use daemon::git::probe::{PROBE_TIMEOUT, probe};
use daemon::run::git::{
    AcceptOutcome, accept, count_commits, diff_so_far, prepare_worktree, salvage, verify_done,
};
use daemon::run::globs::{OwnsMatcher, ProtectedMatcher};
use daemon::run::plan::BUILTIN_PROTECTED;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use support::run_git::{T, commit_file, head, out, real_git, repo, write, wt_dir};

/// An executable at `dir/<name>` that appends its name to `marker` and exits 0.
fn planted(dir: &Path, name: &str, marker: &Path) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!("#!/bin/sh\necho {name} >> '{}'\nexit 0\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn ran(marker: &Path) -> String {
    std::fs::read_to_string(marker).unwrap_or_default()
}

#[test]
fn engine_and_probe_reads_never_run_a_planted_fsmonitor() {
    let repo = repo();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/fs01/t1");
    prepare_worktree(real_git(), &repo.root, "anthrex/fs01/t1", &base, &task, T).unwrap();
    commit_file(&task, "a.txt", "a\n", "work");
    write(&task, "b.txt", "untracked\n");

    // What a worker with a writable common dir could run: `git config core.fsmonitor …`
    // lands in the shared `.git/config`, read by every worktree of the repository.
    let tools = tempfile::tempdir().unwrap();
    let marker = tools.path().join("ran");
    let monitor = planted(tools.path(), "fsmonitor", &marker);
    out(
        &repo.root,
        &["config", "core.fsmonitor", monitor.to_str().unwrap()],
    );
    // Proof that the plant is live: an ordinary `git status` runs it.
    out(&task, &["status", "--porcelain"]);
    assert!(ran(&marker).contains("fsmonitor"), "the plant is not live");
    std::fs::remove_file(&marker).unwrap();

    let owns = vec!["a.txt".to_string()];
    let generated = OwnsMatcher::new(&[]).unwrap();
    let protected = ProtectedMatcher::new(
        &BUILTIN_PROTECTED
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    verify_done(
        real_git(),
        &task,
        &base,
        &base,
        &owns,
        &generated,
        &protected,
        None,
        T,
    )
    .unwrap();
    count_commits(real_git(), &task, &base, &base, T).unwrap();
    diff_so_far(real_git(), &task, &base, &base, T).unwrap();
    assert!(probe(real_git(), &task, PROBE_TIMEOUT).is_some());
    assert!(probe(real_git(), &repo.root, PROBE_TIMEOUT).is_some());
    let reference = "refs/anthrex/salvage/fs01/t1/1";
    let saved = salvage(real_git(), &task, reference, "anthrex salvage fs01/t1", T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));

    assert_eq!(
        ran(&marker),
        "",
        "a planted fsmonitor ran inside the daemon"
    );
}

#[test]
fn accept_never_runs_a_planted_hook_in_the_users_checkout() {
    let repo = repo();
    let base = head(&repo.root);
    out(
        &repo.root,
        &["checkout", "-q", "-b", "anthrex/hk01/integration"],
    );
    let run_head = commit_file(&repo.root, "a.txt", "run\n", "run work");
    out(&repo.root, &["checkout", "-q", "main"]);

    // What a worker with a writable common dir could plant: hooks git runs inside a
    // merge, in the repository's default hooks directory.
    let tools = tempfile::tempdir().unwrap();
    let marker = tools.path().join("ran");
    let hooks = repo.root.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for name in [
        "pre-merge-commit",
        "prepare-commit-msg",
        "commit-msg",
        "post-merge",
    ] {
        planted(&hooks, name, &marker);
    }

    let outcome = accept(
        real_git(),
        &repo.root,
        "main",
        &base,
        "anthrex/hk01/integration",
        "anthrex: accept run hk01: x",
        T,
    )
    .unwrap();
    let AcceptOutcome::Merged { commit } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(
        out(&repo.root, &["rev-list", "--parents", "-n", "1", &commit]),
        format!("{commit} {base} {run_head}")
    );
    assert_eq!(ran(&marker), "", "a planted hook ran inside run accept");
}
