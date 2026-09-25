//! Every M8a.8 run-git function passes `-C <dir> --no-optional-locks`, reaches git with
//! none of AGENTS.md rule 11's five variables, and marks every write with
//! `-c core.hooksPath=/dev/null -c commit.gpgSign=false` (decision 18) — alone in its
//! own test binary, because it sets variables in this process's environment, and in
//! edition 2024 that races any process spawn on another libtest thread
//! (`worktree_env.rs`'s module doc is the precedent). Keep this file to this one test.

mod support;

use daemon::run::git::{
    count_commits, create_run_branch, diff_so_far, lock_worktree, preflight, prepare_review,
    prepare_worktree, project_settings, protected_files, verify_done,
};
use daemon::run::globs::{OwnsMatcher, ProtectedMatcher};
use daemon::run::plan::BUILTIN_PROTECTED;
use std::ffi::OsString;
use support::recording_git;
use support::run_git::{T, commit_file, head, real_git, repo, wt_dir};

const SCRUBBED: [&str; 5] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_PREFIX",
];
const NO_HOOKS: [&str; 2] = ["-c", "core.hooksPath=/dev/null"];
const WRITE_FLAGS: [&str; 6] = [
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "commit.gpgSign=false",
    "-c",
    "core.logAllRefUpdates=false",
];

/// Subcommands that change the repository or a worktree.
fn is_write(args: &[&str]) -> bool {
    match args.first().copied() {
        Some("worktree") => args.get(1).is_some_and(|s| *s != "list"),
        Some("checkout" | "commit-tree" | "update-ref" | "merge" | "add" | "reset" | "clean") => {
            true
        }
        _ => false,
    }
}

#[test]
fn every_run_git_call_passes_no_optional_locks_and_no_git_env() {
    // Everything the calls below need is built before the environment changes, with
    // the real git, so no setup command sees the planted variables.
    let repo = repo();
    let base = head(&repo.root);
    let newer = commit_file(&repo.root, "src/lib.rs", "pub fn f() {}\n", "newer");
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/env1/t1");
    prepare_worktree(real_git(), &repo.root, "anthrex/env1/t1", &base, &task, T).unwrap();
    let red = commit_file(&task, "src/t.rs", "t\n", "red");
    let scripts = tempfile::tempdir().unwrap();
    let git = recording_git(scripts.path());
    let git = git.as_os_str();
    let protected = ProtectedMatcher::new(
        &BUILTIN_PROTECTED
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let generated = OwnsMatcher::new(&["Cargo.lock".to_string()]).unwrap();

    let previous: Vec<(&str, Option<OsString>)> = ["GIT_DIR", "GIT_INDEX_FILE"]
        .iter()
        .map(|k| (*k, std::env::var_os(k)))
        .collect();
    // SAFETY: this is the only test in this binary, so no other thread spawns a process
    // or reads the environment while these writes happen (module doc).
    unsafe {
        std::env::set_var("GIT_DIR", "/nonexistent/leak-marker");
        std::env::set_var("GIT_INDEX_FILE", "/nonexistent/leak-index");
    }

    let results: Vec<(&str, Result<(), String>)> = vec![
        ("preflight", preflight(git, &repo.root, T).map(drop)),
        (
            "project_settings",
            project_settings(
                git,
                &repo.root,
                &newer,
                true,
                Some(&[".codex/config.toml"]),
                T,
            )
            .map(drop),
        ),
        (
            "protected_files",
            protected_files(git, &repo.root, &newer, &protected, T).map(drop),
        ),
        (
            "create_run_branch",
            create_run_branch(
                git,
                &repo.root,
                "anthrex/env1/integration",
                &base,
                &wt.join("runs/env1/integration"),
                T,
            )
            .map(drop),
        ),
        (
            "prepare_worktree (new)",
            prepare_worktree(
                git,
                &repo.root,
                "anthrex/env1/t2",
                &base,
                &wt.join("runs/env1/t2"),
                T,
            )
            .map(drop),
        ),
        (
            "prepare_worktree (re-point)",
            prepare_worktree(
                git,
                &repo.root,
                "anthrex/env1/t2",
                &newer,
                &wt.join("runs/env1/t2"),
                T,
            )
            .map(drop),
        ),
        (
            "lock_worktree",
            lock_worktree(git, &repo.root, &task, "anthrex run env1", T).map(drop),
        ),
        (
            "verify_done",
            verify_done(
                git,
                &task,
                &base,
                &base,
                &["src/**".to_string()],
                &generated,
                &protected,
                Some(&red),
                T,
            )
            .map(drop),
        ),
        (
            "count_commits",
            count_commits(git, &task, &base, &base, T).map(drop),
        ),
        (
            "diff_so_far",
            diff_so_far(git, &task, &base, &base, T).map(drop),
        ),
        (
            "prepare_review",
            prepare_review(
                git,
                &repo.root,
                "anthrex/env1/t1",
                &base,
                &wt.join("runs/env1/t1.review"),
                T,
            )
            .map(drop),
        ),
        (
            "prepare_review (replace)",
            prepare_review(
                git,
                &repo.root,
                "anthrex/env1/t1",
                &base,
                &wt.join("runs/env1/t1.review"),
                T,
            )
            .map(drop),
        ),
    ];

    // SAFETY: as above.
    unsafe {
        for (key, value) in &previous {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    for (name, result) in &results {
        assert!(result.is_ok(), "{name}: {result:?}");
    }
    let log = std::fs::read_to_string(scripts.path().join("git.log")).unwrap();
    let mut calls = 0;
    let mut pinned = 0;
    let mut writes = 0;
    let mut no_replace = 0;
    for line in log.lines() {
        if let Some(env) = line.strip_prefix("env\t") {
            // Ruling T14-R3: no call reads `refs/replace` objects.
            no_replace += usize::from(env == "GIT_NO_REPLACE_OBJECTS=1");
            for key in SCRUBBED {
                assert!(
                    !env.starts_with(&format!("{key}=")),
                    "{key} reached git: {env}"
                );
            }
            continue;
        }
        let argv: Vec<&str> = line
            .strip_prefix("argv")
            .unwrap()
            .split('\t')
            .skip(1)
            .collect();
        calls += 1;
        assert_eq!(argv[0], "-C", "{argv:?}");
        assert!(std::path::Path::new(argv[1]).is_absolute(), "{argv:?}");
        assert_eq!(argv[2], "--no-optional-locks", "{argv:?}");
        // Final fix batch F1 (C-C1, D-5): no call runs a configured fsmonitor, and a
        // call that is not a write still runs no hook.
        assert_eq!(&argv[3..5], ["-c", "core.fsmonitor=false"], "{argv:?}");
        let mut rest = &argv[5..];
        // Fix round 1 (N2): a call in an engine worktree is pinned to its git dir, and
        // then names it and the work tree itself.
        if rest.first().is_some_and(|a| a.starts_with("--git-dir=")) {
            pinned += 1;
            assert!(rest[1].starts_with("--work-tree="), "{argv:?}");
            assert_eq!(&rest[1]["--work-tree=".len()..], argv[1], "{argv:?}");
            rest = &rest[2..];
        } else {
            assert!(
                !argv[1].contains("/runs/"),
                "an engine worktree call that is not pinned: {argv:?}"
            );
        }
        let (flags, command) = if rest.starts_with(&WRITE_FLAGS) {
            (true, &rest[WRITE_FLAGS.len()..])
        } else if rest.starts_with(&NO_HOOKS) {
            (false, &rest[2..])
        } else {
            // Only preflight's root detection (`project::detect_roots_with`, shared with
            // the rest of the daemon) runs outside `run::git`'s `Git`; a `rev-parse`
            // runs no hook.
            assert_eq!(
                rest.last(),
                Some(&"--show-toplevel"),
                "a read that may run hooks: {argv:?}"
            );
            (false, rest)
        };
        if is_write(command) {
            writes += 1;
            assert!(
                flags,
                "a write without the hook and signing overrides: {argv:?}"
            );
        }
    }
    assert!(calls >= 20, "only {calls} git calls were recorded:\n{log}");
    assert!(
        pinned >= 5,
        "only {pinned} pinned calls were recorded:\n{log}"
    );
    assert!(writes >= 5, "only {writes} writes were recorded:\n{log}");
    assert_eq!(no_replace, calls, "replace objects allowed:\n{log}");
}
