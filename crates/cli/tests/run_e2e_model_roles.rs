//! Milestone 9.8 end to end (MR §7, §8): the role table through a real daemon with
//! `fake-agent` as the runtimes it finds. No test reaches a real agent: every runtime
//! command is `fake-agent` or a path that does not exist.

mod support;

use proto::{BlockReason, TaskState};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

/// MR §7: a role whose runtime's CLI is missing fails its launch naming the role and
/// the way out. The reviewer row is the Codex default and Codex's command does not
/// exist: the worker (the small row, Claude) finishes green, and the review's start
/// blocks the task with `reviewer: codex not found; choose another model in C-b S`.
#[test]
fn a_missing_cli_names_the_role() {
    let codex = ("ANTHREX_CODEX_BIN", "/nonexistent/anthrex-test/codex");
    let reviewer = "[models.reviewer]\nmodel = \"codex:default\"\n";
    let h = RunHarness::with_env_and_config("", &[codex], reviewer);
    h.script(
        "worker-t1-1",
        &[commit("a.rs", "fn a() {}\n"), done("added a")],
    );
    let id = h.start(&plan("", &[task("t1", &["a.rs"], "")]), true);
    let blocked = |r: &proto::RunInfo| t(r, "t1").state == TaskState::Blocked;
    let run = h.wait_run(&id, blocked, RUN_WAIT);
    let t1 = t(&run, "t1");
    let block = t1.block.as_ref().expect("a block");
    assert_eq!(block.reason, BlockReason::Environment, "{block:?}");
    let want = "reviewer: codex not found; choose another model in C-b S";
    assert!(block.text.contains(want), "{block:?}");
    assert!(t1.rounds.iter().any(|r| r.role == proto::AgentRole::Worker));
}
