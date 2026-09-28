//! The followups file's "A session's signals and tool calls before its `CreateWindow`
//! result are lost", end to end: the daemon holds every `CreateWindow`'s `done` line
//! (the debug builds' `ANTHREX_TEST_DELAY_WINDOW_MS`, set for this harness's daemon
//! only), so each `fake-agent` session runs its whole script, its tool call and turn
//! end included, before the engine knows its window. On a slow-fsync CI runner this
//! happened without the hold: the worker's `task_done` was refused, its turn end
//! dropped, and the run stalled until the test's limit.

mod support;

use proto::TaskState;
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

/// The hold on each `CreateWindow`'s `done` line: far over `fake-agent`'s whole scripted
/// turn (tens of milliseconds), so every event of it comes before the window is known.
/// A lower bound, never a wait (docs/timing-budgets.md).
const WINDOW_HOLD_MS: &str = "1500";

#[test]
fn e2e_a_session_that_finishes_before_its_window_is_known_still_completes() {
    let h = RunHarness::with_env(
        "",
        &[("ANTHREX_TEST_DELAY_WINDOW_MS", WINDOW_HOLD_MS)],
        true,
    );
    green_scripts(&h.repo);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    // One task path (`k = 1`). Before the fix this wait ran out: the worker's claim was
    // refused and its turn end lost.
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let log = std::fs::read_to_string(h.data().join("daemon.log")).unwrap_or_default();
    assert!(
        log.contains("ANTHREX_TEST_DELAY_WINDOW_MS: holding op done lines"),
        "the window hold was not armed:\n{log}"
    );
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
}
