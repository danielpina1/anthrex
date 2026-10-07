//! Milestone 9.8 task M9.8.3 (decision 18): a daemon started on a config with an old
//! model key logs that key's migration note once, beside the config problems. Real
//! binary, real daemon on `/tmp` paths, `fake-agent` for every agent.

mod support;

use std::time::Instant;

use support::run_daemon::DAEMON_START_WAIT;
use support::run_harness::RunHarness;

const NOTE: &str = "config: [orchestrator.routes.review] is replaced by [models.reviewer]; it is migrated until you save in C-b S";

#[test]
fn daemon_start_logs_each_migrated_key() {
    let h = RunHarness::with_config(
        "",
        "[orchestrator.routes.review]\ncandidates = [{ runtime = \"claude\", model = \"claude-opus-5-5\" }]\n",
        &[],
    );
    let deadline = Instant::now() + DAEMON_START_WAIT;
    while !h.log_tail().contains(NOTE) {
        assert!(
            Instant::now() < deadline,
            "the daemon never logged the migration note:\n{}",
            h.log_tail()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let tail = h.log_tail();
    assert_eq!(
        tail.matches("[orchestrator.routes.review] is replaced")
            .count(),
        1,
        "{tail}"
    );
}
