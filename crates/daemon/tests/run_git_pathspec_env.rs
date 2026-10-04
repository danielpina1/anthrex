//! Milestone 9.5 re-review 3's NI-2: an inherited `GIT_LITERAL_PATHSPECS` (or another
//! pathspec-mode variable) must not change what the daemon's git reads, whose
//! `:(literal)` and `:(exclude,literal)` pathspecs would otherwise match nothing.
//!
//! Alone in its own test binary, as `worktree_env.rs` is and for its reason: setting a
//! variable races any thread that spawns a process, and only a binary with no other test
//! has no such thread. Keep this file to this one test.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use daemon::worktree::run_git;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

#[test]
fn pathspec_mode_variables_are_scrubbed() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path();
    git(dir, &["init", "-q", "-b", "main"]);
    for name in ["a.rs", "*.rs"] {
        std::fs::write(dir.join(name), "x\n").unwrap();
    }
    git(dir, &["add", "-A"]);
    let who = ["-c", "user.name=T", "-c", "user.email=t@test"];
    git(dir, &[&who[..], &["commit", "-q", "-m", "base"]].concat());
    for name in ["a.rs", "*.rs"] {
        std::fs::write(dir.join(name), "y\n").unwrap();
    }
    let modes = [
        "GIT_LITERAL_PATHSPECS",
        "GIT_GLOB_PATHSPECS",
        "GIT_NOGLOB_PATHSPECS",
        "GIT_ICASE_PATHSPECS",
    ];
    let previous: Vec<(&str, Option<OsString>)> =
        modes.iter().map(|k| (*k, std::env::var_os(k))).collect();
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    // Git refuses the literal setting with any other, so it is set alone.
    unsafe {
        std::env::set_var("GIT_LITERAL_PATHSPECS", "1");
    }
    let args: Vec<&OsStr> = ["diff", "--name-only", "--", ":(exclude,literal)*.rs"]
        .iter()
        .map(OsStr::new)
        .collect();
    let result = run_git(
        OsStr::new("git"),
        dir,
        &args,
        Instant::now() + Duration::from_secs(10),
    );
    // SAFETY: as above.
    unsafe {
        for (key, value) in &previous {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
    let output = result.expect("git diff");
    assert!(output.success, "{output:?}");
    assert_eq!(
        output.stdout.trim(),
        "a.rs",
        "the magic pathspec still applies"
    );
}
