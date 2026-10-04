//! W2 re-review N1 (AGENTS.md rule 11): every git the daemon runs itself drops the
//! eleven inherited git variables: the five location variables, the two object-store
//! ones and the four pathspec modes. One site is driven here, the project root
//! detection; the others share its helper (`subprocess::scrub_inherited_git`).
//!
//! Alone in its own test binary, as `worktree_env.rs` and `run_git_pathspec_env.rs`
//! are and for their reason: setting a variable races any thread that spawns a
//! process, and only a binary with no other test has no such thread. Keep this file to
//! this one test.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

const SCRUBBED: [&str; 11] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_PREFIX",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_LITERAL_PATHSPECS",
    "GIT_GLOB_PATHSPECS",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
];

#[test]
fn project_detection_runs_git_without_the_inherited_git_variables() {
    let tmp = tempfile::tempdir().unwrap();
    let seen = tmp.path().join("env.txt");
    // A stand-in `git` (never the real one): it records its environment and fails, so
    // detection falls back to the directory itself.
    let git = tmp.path().join("git");
    let script = format!("#!/bin/sh\nenv > '{}'\nexit 1\n", seen.display());
    std::fs::write(&git, script).unwrap();
    std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    unsafe {
        for key in SCRUBBED {
            std::env::set_var(key, "/tmp/inherited");
        }
    }
    let roots = daemon::project::detect_roots_with(
        OsStr::new(git.as_os_str()),
        tmp.path(),
        Duration::from_secs(10),
    );
    // SAFETY: as above.
    unsafe {
        for key in SCRUBBED {
            std::env::remove_var(key);
        }
    }
    assert_eq!(roots.worktree, None, "the stand-in failed");
    let env = std::fs::read_to_string(&seen).expect("the stand-in ran");
    let leaked: Vec<&str> = (env.lines())
        .filter(|line| SCRUBBED.iter().any(|k| line.starts_with(&format!("{k}="))))
        .collect();
    assert!(leaked.is_empty(), "inherited by git: {leaked:?}");
}
