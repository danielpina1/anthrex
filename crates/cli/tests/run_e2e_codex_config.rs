//! M8a final fix batch F2 (review C, I1), through a real daemon with `fake-agent` as
//! both runtimes. Codex reads a checkout's `.codex/` again on every turn (each is a new
//! `codex exec` process), and nothing on its command line excludes it (M8a.1 item 7a),
//! so decision 53's check of the base tree alone does not hold across a run. Before
//! every Codex process starts, its checkout's `.codex` must be exactly the base's;
//! otherwise nothing starts and the task is `blocked(environment)`:
//! - (a) a Codex worker that writes `.codex/config.toml` never gets a next turn;
//! - (b) a file a task legitimately owned (decision 56) keeps every later Codex session
//!   off the checkouts that carry it, here the task's own Codex reviewer.
//!
//! Trusted base config (`--trust-project`) still loads: `run_e2e_settings.rs`'s
//! `e2e_codex_project_config_follows_cli_caps` runs a Codex worker to completion on it.

mod support;

use proto::{BlockReason, TaskState};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

/// The run's state once `task` is blocked, or a panic as soon as `name` has started a
/// second process (the guard's absence) or `RUN_WAIT` passes.
fn blocked_before_another_launch(h: &RunHarness, id: &str, task: &str, name: &str, most: usize) {
    let run = until("the task to block", RUN_WAIT, || {
        let launches = h.io_lines(name, "args").len();
        assert!(
            launches <= most,
            "{name} was launched {launches} times: a Codex session started on config it must not load"
        );
        h.run(id).filter(|r| t(r, task).state == TaskState::Blocked)
    });
    let block = t(&run, task).block.clone().expect("a block");
    assert_eq!(block.reason, BlockReason::Environment, "{block:?}");
    assert!(block.text.contains(".codex/config.toml"), "{block:?}");
    assert!(block.text.contains("decision 53"), "{block:?}");
    assert_eq!(h.io_lines(name, "args").len(), most);
}

#[test]
fn e2e_a_codex_worker_never_gets_a_turn_on_config_it_wrote() {
    let h = RunHarness::new("");
    h.script(
        "worker-t1-1",
        &[sh(
            "mkdir -p .codex && printf '[mcp_servers.x]\\ncommand = \"true\"\\n' > .codex/config.toml",
        )],
    );
    let id = h.start(&plan("", &[task("t1", &["a.txt"], CODEX)]), true);
    // The turn ends with no commit and no `task_done`: the engine's nudge is the next
    // turn, which must not start.
    blocked_before_another_launch(&h, &id, "t1", "worker-t1-1", 1);
}

#[test]
fn e2e_owned_codex_config_keeps_later_codex_sessions_off_its_checkout() {
    let h = RunHarness::new("");
    h.script(
        "worker-t1-1",
        &[
            sh("mkdir -p .codex"),
            commit(".codex/config.toml", "model = \"from-the-task\"\n"),
            done("added the config"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    // A Claude worker's reviewer runs Codex (the other runtime), in a review checkout
    // that carries the task's `.codex/config.toml`.
    let id = h.start(&plan("", &[task("t1", &[".codex/config.toml"], "")]), true);
    blocked_before_another_launch(&h, &id, "t1", "reviewer-t1-1", 0);
}
