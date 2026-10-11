//! Milestone 9.11 decision 32 (D3): an engine command (a check, a proof, `setup`, a tier
//! step) loses the user's cloud and registry tokens it would inherit from the daemon,
//! except the names `ANTHREX_CHECK_ENV_PASS` lists. Alone in its own test binary,
//! because it sets variables in this process's environment, which in edition 2024 races
//! any process spawn on another libtest thread (`run_exec_env.rs` is the precedent).
//! Keep this file to this one test.
//!
//! `run_shell` is the public entry point to `exec::run_matching` (which is crate-private);
//! it calls `run_matching` with no pattern and no confinement.

use daemon::run::exec::run_shell;
use std::time::Duration;

#[test]
fn a_check_loses_known_secrets_but_keeps_the_ones_the_user_passes() {
    // SAFETY: this binary has exactly one test, so no other thread reads or spawns while
    // the environment changes.
    unsafe {
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "a1");
        std::env::set_var("NPM_TOKEN", "n1");
        std::env::set_var("GITHUB_TOKEN", "g1");
        std::env::set_var("ANTHREX_CHECK_ENV_PASS", "NPM_TOKEN");
    }
    let dir = tempfile::tempdir().unwrap();
    // Filtered so that a long environment cannot push a line out of the 200-line tail.
    let outcome = run_shell(
        dir.path(),
        "env | grep -E '^(AWS_|NPM_TOKEN|GITHUB_TOKEN)'; true",
        &[],
        &[],
        Duration::from_secs(60),
    );
    assert!(outcome.ok, "{outcome:?}");
    let lines: Vec<&str> = outcome.tail.lines().collect();
    assert!(
        !lines
            .iter()
            .any(|line| line.starts_with("AWS_SECRET_ACCESS_KEY=")),
        "a cloud token reached a check: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("GITHUB_TOKEN=")),
        "a registry token the user did not pass reached a check: {lines:?}"
    );
    assert!(
        lines.contains(&"NPM_TOKEN=n1"),
        "the passed token did not reach the check: {lines:?}"
    );
}
