//! Milestone 9.15 ruling C3: the harnesses' daemons run with lessons off unless a test
//! opts in. Each test asks the daemon which config file it reads (its settings reply)
//! and loads that file with the daemon's own loader.

use std::path::Path;

use serde_json::json;

use crate::support::TestDaemon;
use crate::support::lessons::daemon_config_path;
use crate::support::run_harness::{REQUEST_WAIT, RunHarness};

/// `(enabled, risk_note)` of the config the daemon at `socket` reads; its problems are
/// none.
fn lessons_of(socket: &Path) -> (bool, bool) {
    let path = daemon_config_path(socket, REQUEST_WAIT);
    let (config, problems) = config::load(&path);
    assert!(problems.is_empty(), "{path:?}: {problems:?}");
    (config.lessons.enabled, config.lessons.risk_note)
}

#[test]
fn run_harness_daemons_run_with_lessons_off() {
    let h = RunHarness::new("");
    assert_eq!(lessons_of(&h.socket()), (false, false));
    let h = RunHarness::with_config("", "[testing]\ntest_slots = 1\n", &[]);
    assert_eq!(lessons_of(&h.socket()), (false, false));
    let h = RunHarness::adapt("claude", "", &[], &[]);
    assert_eq!(lessons_of(&h.socket()), (false, false));
}

#[test]
fn a_tests_own_lessons_table_replaces_the_default() {
    let h = RunHarness::with_config("", "[lessons]\nrisk_note = true\n", &[]);
    // One table, the test's: `enabled` is back at its default.
    assert_eq!(lessons_of(&h.socket()), (true, true));
    let h = RunHarness::adapt_with_config("claude", "", &[], &[], "[lessons]\nenabled = true\n");
    assert_eq!(lessons_of(&h.socket()), (true, true));
}

#[test]
fn test_daemons_run_with_lessons_off() {
    let daemon = TestDaemon::start(&[]);
    assert_eq!(lessons_of(&daemon.socket()), (false, false));
}

#[test]
fn decider_calls_leave_out_distill_and_risk_note() {
    let h = RunHarness::adapt("claude", "", &[], &[]);
    let calls: Vec<String> = ["run_name", "distill", "check_summary", "risk_note"]
        .iter()
        .map(|kind| json!({"kind": kind, "argv": [], "prompt": ""}).to_string())
        .collect();
    std::fs::write(h.decider_dir().join("calls.jsonl"), calls.join("\n") + "\n").unwrap();
    let kinds: Vec<serde_json::Value> = h
        .decider_calls()
        .iter()
        .map(|c| c["kind"].clone())
        .collect();
    assert_eq!(kinds, vec![json!("check_summary")]);
}
