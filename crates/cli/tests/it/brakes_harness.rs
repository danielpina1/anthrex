//! Milestone 9.16 ruling C19: the harnesses' daemons run with the quota brake off unless
//! a test opts in. Each test asks the daemon which config file it reads (its settings
//! reply) and loads that file with the daemon's own loader.

use std::path::Path;

use crate::support::TestDaemon;
use crate::support::brakes::BRAKES_ON;
use crate::support::lessons::daemon_config_path;
use crate::support::run_harness::{REQUEST_WAIT, RunHarness};

/// `quota_brake` of the config the daemon at `socket` reads; its problems are none.
fn quota_brake_of(socket: &Path) -> bool {
    let path = daemon_config_path(socket, REQUEST_WAIT);
    let (config, problems) = config::load(&path);
    assert!(problems.is_empty(), "{path:?}: {problems:?}");
    config.orchestrator.brakes.quota_brake
}

#[test]
fn run_harness_daemons_run_with_the_quota_brake_off() {
    let h = RunHarness::new("");
    assert!(!quota_brake_of(&h.socket()));
    let h = RunHarness::with_config("", "[testing]\ntest_slots = 1\n", &[]);
    assert!(!quota_brake_of(&h.socket()));
    let h = RunHarness::adapt("claude", "", &[], &[]);
    assert!(!quota_brake_of(&h.socket()));
    let daemon = TestDaemon::start(&[]);
    assert!(!quota_brake_of(&daemon.socket()));
}

#[test]
fn a_tests_own_brakes_table_replaces_the_default() {
    let h = RunHarness::with_config("", BRAKES_ON, &[]);
    assert!(quota_brake_of(&h.socket()));
    let h = RunHarness::adapt_with_config("claude", "", &[], &[], BRAKES_ON);
    assert!(quota_brake_of(&h.socket()));
    // A brake key in the `[orchestrator]` lines themselves.
    let h = RunHarness::new("brakes.quota_pause_7d_percent = 90\n");
    assert!(quota_brake_of(&h.socket()));
}
