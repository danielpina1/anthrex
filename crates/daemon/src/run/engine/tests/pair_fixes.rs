//! Milestone 9.5 task M9.5.16, fix round 1 (rulings T16-2 to T16-6): the writer's route
//! steps on the workers' skipping ladder, the implementer waits for a running run, a
//! merge retires a test writer, override waits for the implementer, and no merge goes
//! into a writer's checkout.

use proto::{AgentRole, BlockReason, Effort, PairPhase, PlanEdit, Runtime, TaskState};
use serde_json::json;

use super::super::control::{override_task, resume, retry};
use super::super::dispatch::{edit, replies};
use super::super::done::one_reply;
use super::super::gates::{check_result, only_op};
use super::super::holds::{add_dep, answer, delivers};
use super::super::merge::{commit, pending_one};
use super::super::turns::killed_exit;
use super::*;
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::orch::RefreshState;

/// A route on the Claude sonnet at `high` effort, so rung 2 cannot raise the effort.
const SONNET_HIGH: &str =
    "[task.route]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"\neffort = \"high\"";

/// The test writer's two failed red checks: rung 2, its window killed and exited.
fn rung2(fx: &mut Fixture, writer: u32) {
    for _ in 0..2 {
        let (op, _) = claim_red(fx, writer, HEAD);
        fx.done(op, red_check(false));
    }
    killed_exit(fx, writer);
}

/// The next writer's launch after a `DiffSoFar`.
fn fresh_launch(fx: &mut Fixture) -> Launch {
    let (op, _) = pending_one(fx, "DiffSoFar", Some("t1"));
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let (_, kind) = only_op(&effects, "CreateWindow");
    launch_of(kind)
}

/// T16-2: with Codex not installed, rung 2 never steps the writer onto Codex (plain
/// `roster::escalate` from Claude sonnet at `high` gave the Codex peer), and the pick
/// is recorded as an escalation.
#[test]
fn the_writers_rung_2_skips_a_runtime_that_is_not_installed() {
    let tasks = [task("t1", "S", "a", &format!("{PAIRED}\n{SONNET_HIGH}"))];
    let (mut fx, launch, writer) =
        running(PROFILE, &tasks, config::Orchestrator::default(), |run| {
            run.orch.installed.insert("codex".into(), false);
        });
    assert_eq!(launch.spec.runtime, Runtime::Claude);
    assert_eq!(launch.spec.effort, Effort::High);
    rung2(&mut fx, writer);
    let launch = fresh_launch(&mut fx);
    assert_eq!(role_of(&launch), AgentRole::TestWriter);
    assert_eq!(
        launch.spec.runtime,
        Runtime::Claude,
        "Codex is not installed"
    );
    assert_eq!(
        launch.spec.model, "claude-opus-5-5",
        "one step up on Claude"
    );
    let t1 = fx.task("t1");
    let writers: Vec<_> = (t1.routing_decisions.iter())
        .filter(|d| d.role == AgentRole::TestWriter)
        .collect();
    assert_eq!(writers.len(), 2, "{writers:#?}");
    // Minor m1: the first pick's source is the task's own route (no peer installed).
    assert_eq!(
        (writers[0].trigger.as_str(), writers[0].source.as_str()),
        ("test_writer", "explicit_task")
    );
    assert_eq!(
        (writers[1].role, writers[1].session),
        (AgentRole::TestWriter, 2)
    );
    assert_eq!(
        writers[1].trigger, "escalation",
        "decision 9a: rung 2's pick"
    );
}

/// T16-2 (ruling T10a-5): a writer whose session failed for an environment reason is
/// substituted by the peer at its strength on `run retry`, never re-run on its route.
#[test]
fn a_writer_that_failed_in_this_task_is_substituted_on_retry() {
    let (mut fx, launch, writer) = paired();
    assert_eq!(launch.spec.runtime, Runtime::Codex);
    let args = json!({"kind": "environment", "reason": "codex cannot start"});
    assert!(one_reply(&writer_tool(&mut fx, writer, "task_blocked", args)).is_ok());
    fx.task_mut("t1").rounds[0].environment_failed = true;
    killed_exit(&mut fx, writer);
    let effects = retry(&mut fx, "t1");
    assert!(one_reply(&effects).is_ok(), "{effects:#?}");
    let launch = fresh_launch(&mut fx);
    assert_eq!(role_of(&launch), AgentRole::TestWriter);
    assert_eq!(
        (launch.spec.runtime, launch.spec.model.as_str()),
        (Runtime::Claude, "claude-sonnet-5"),
        "the peer at the same strength"
    );
    assert_eq!(
        fx.task("t1").route.runtime,
        Runtime::Claude,
        "the implementer's"
    );
}

/// T16-2 (ruling T10a-6): at dispatch, the writer does not take the peer runtime while
/// an unfinished task on the task's runtime overlaps its `owns` and may run beside it
/// (ruling FW-1: `t2` does not wait on `t1`).
#[test]
fn the_writers_dispatch_route_keeps_the_overlap_rule() {
    let tasks = [
        task("t1", "S", "a", PAIRED),
        task_toml("t2", "S", "[\"crates/a/src/**\"]", ""),
    ];
    let clear = |run: &mut crate::run::model::Run| run.tasks[1].implicit_deps.clear();
    let (fx, launch, _) = running(PROFILE, &tasks, config::Orchestrator::default(), clear);
    assert_eq!(fx.task("t2").route.runtime, Runtime::Claude);
    assert_eq!(
        launch.spec.runtime,
        Runtime::Claude,
        "Codex would share t2's files"
    );
    let pair = fx.task("t1").pair.clone().unwrap();
    assert_eq!(pair.writer_route, fx.task("t1").route);
}

/// T16-3 (ruling T14-I2): a red check answered while the run is paused records the
/// hand-over but starts no session; the implementer launches on resume.
#[test]
fn the_implementer_waits_for_a_running_run() {
    let (mut fx, _, writer) = paired();
    let (op, _) = claim_red(&mut fx, writer, HEAD);
    let effects = edit(&mut fx, vec![PlanEdit::Pause]);
    assert_eq!(replies(&effects), vec![Ok("applied 1 edit".to_string())]);
    assert_eq!(fx.run().state, proto::RunState::Paused);
    let effects = fx.done(op, red_check(true));
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    let t1 = fx.task("t1");
    assert_eq!(t1.pair.as_ref().unwrap().phase, PairPhase::Implementing);
    assert!(effects.contains(&Effect::RetireWindow { window_id: writer }));
    assert!(
        ops_in(&fx.tick(), "CreateWindow").is_empty(),
        "still paused"
    );
    let effects = resume(&mut fx);
    let (_, kind) = only_op(&effects, "CreateWindow");
    let launch = launch_of(kind);
    assert_eq!(launch.name, format!("{H4}/t1.w2"));
    assert_eq!(role_of(&launch), AgentRole::Worker);
    assert!(
        ops_in(&fx.tick(), "CreateWindow").is_empty(),
        "launched once"
    );
}

/// T16-4: a merge retires every writing session of the task, a test writer's too.
#[test]
fn a_merge_retires_a_live_test_writer() {
    let (mut fx, _, window, _) = implementing();
    let writer = fx.task("t1").rounds[0].window_id.unwrap();
    // A writer the hand-over did not retire (it is the merge's to stop then): the
    // Codex writer, mid-turn.
    let round = &mut fx.task_mut("t1").rounds[0];
    (round.retiring, round.ended, round.ended_at, round.turn_open) = (false, false, None, true);
    let effects = fx.tool(window, "task_done", json!({"summary": "made it pass"}));
    let (op, _) = only_op(&effects, "VerifyDone");
    let result = fx.clean_check("t1");
    let effects = fx.done(op, result);
    let (op, _) = only_op(&effects, "Proof");
    let passed = OpResult::Proof {
        red_failed: true,
        head_passed: true,
        matched: true,
        red_tail: String::new(),
        head_tail: format!("test {TEST} ... ok"),
    };
    let effects = fx.done(op, passed);
    let (op, _) = only_op(&effects, "Check");
    fx.done(op, check_result(true));
    if fx.task("t1").state == TaskState::Review {
        assert!(one_reply(&override_task(&mut fx, "t1", "ship it")).is_ok());
    }
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    let (op, _) = pending_one(&fx, "MergeCandidate", Some("t1"));
    let effects = fx.done(
        op,
        OpResult::Merged {
            commit: commit(1),
            tier: None,
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Merged);
    assert!(
        effects.contains(&Effect::RetireWindow { window_id: writer }),
        "{effects:#?}"
    );
    assert!(effects.contains(&Effect::RetireWindow { window_id: window }));
}

/// T16-5: a failing test alone never merges: override waits for the implementer.
#[test]
fn override_is_refused_while_the_test_is_being_written() {
    let text = "task t1 is still writing its test; override it once its implementer has started";
    let (mut fx, _, writer) = paired();
    assert_eq!(
        one_reply(&override_task(&mut fx, "t1", "x")),
        Err(text.to_string())
    );
    let args = json!({"kind": "question", "reason": "which table?"});
    assert!(one_reply(&writer_tool(&mut fx, writer, "task_blocked", args)).is_ok());
    assert_eq!(
        fx.task("t1").block.as_ref().unwrap().reason,
        BlockReason::Question
    );
    let effects = override_task(&mut fx, "t1", "x");
    assert_eq!(one_reply(&effects), Err(text.to_string()));
    assert!(ops_in(&effects, "CountCommits").is_empty());
    // Once the implementer works, override is the usual one.
    let (mut fx, _, window, _) = implementing();
    let args = json!({"kind": "question", "reason": "which table?"});
    assert!(one_reply(&fx.tool(window, "task_blocked", args)).is_ok());
    let effects = override_task(&mut fx, "t1", "x");
    assert!(
        replies(&effects).is_empty(),
        "the commits are counted: {effects:#?}"
    );
    only_op(&effects, "CountCommits");
}

/// T16-6: a refresh due while the test is being written waits for the implementer, and
/// the writer's mail is not held meanwhile.
#[test]
fn a_refresh_waits_for_the_implementer() {
    let (mut fx, _, writer) = paired();
    fx.task_mut("t1").orch.refresh = Some(RefreshState::Due);
    fx.turn_completed(writer);
    assert!(ops_in(&fx.tick(), "HandBack").is_empty());
    // The writer still hears the red check's failure.
    let (op, _) = claim_red(&mut fx, writer, HEAD);
    let effects = fx.done(op, red_check(false));
    assert_eq!(delivers(&effects).len(), 1, "{effects:#?}");
    assert!(ops_in(&effects, "HandBack").is_empty());
    let (op, _) = claim_red(&mut fx, writer, HEAD);
    let effects = fx.done(op, red_check(true));
    assert!(
        ops_in(&effects, "HandBack").is_empty(),
        "the implementer's turn is open"
    );
    let window = fx.complete_windows()[0].1;
    // The implementer's turn boundary: the fallback's count first, then the refresh.
    let (op, _) = only_op(&fx.turn_completed(window), "CountCommits");
    let commits = OpResult::Commits {
        count: 0,
        head: HEAD.into(),
    };
    let effects = fx.done(op, commits);
    let (_, kind) = only_op(&effects, "HandBack");
    assert!(matches!(
        kind,
        OpKind::HandBack {
            list_merged: true,
            ..
        }
    ));
}

/// T16-6: a held test writer whose dependencies merged resumes without the run head
/// merged into its checkout; the merge follows as a refresh once its implementer works.
#[test]
fn a_held_writer_resumes_without_a_hand_back() {
    let tasks = [
        task("t1", "S", "a", PAIRED),
        task_toml("t2", "S", "[\"crates/a/src/**\"]", ""),
    ];
    let (mut fx, _, writer) = running(PROFILE, &tasks, config::Orchestrator::default(), |_| {});
    let args = json!({"kind": "question", "reason": "which table?"});
    assert!(one_reply(&writer_tool(&mut fx, writer, "task_blocked", args)).is_ok());
    fx.turn_completed(writer);
    edit(&mut fx, vec![add_dep("t1", "t2"), answer("users")]);
    assert!(fx.task("t1").awaiting_deps);
    fx.launch_all();
    let effects = fx.merge("t2", &"c2".repeat(20));
    assert!(ops_in(&effects, "HandBack").is_empty(), "{effects:#?}");
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Working);
    assert!(!t1.awaiting_deps);
    assert_eq!(t1.orch.refresh, Some(RefreshState::Due));
    assert_eq!(delivers(&effects).len(), 1, "the answer reaches the writer");
    assert_eq!(
        fx.task("t1").pair.as_ref().unwrap().phase,
        PairPhase::Writing
    );
}

/// Minor m3: each red check leaves a red-only proof record, which the history tally
/// and the snapshot's last proof leave out.
#[test]
fn a_red_check_leaves_a_red_only_proof_record() {
    let (mut fx, _, writer) = paired();
    let (op, _) = claim_red(&mut fx, writer, HEAD);
    fx.done(op, red_check(false));
    let (op, _) = claim_red(&mut fx, writer, HEAD);
    fx.done(op, red_check(true));
    let proofs = &fx.task("t1").proofs;
    let checks: Vec<(bool, bool, &str)> = (proofs.iter())
        .map(|p| (p.red_only, p.red_failed, p.test.as_str()))
        .collect();
    assert_eq!(checks, vec![(true, false, TEST), (true, true, TEST)]);
    assert_eq!(proofs[1].red, HEAD);
    let tally = crate::run::history::gates(fx.task("t1"));
    assert_eq!((tally.proofs, tally.proofs_failed), (0, 0));
}

/// Ruling T16-7 (N3): a writer that falls back to the task's route, which a model list
/// chose, records the list's source and snapshot.
#[test]
fn a_writer_on_a_list_chosen_route_records_the_list() {
    let tasks = [task("t1", "S", "a", &format!("{PAIRED}\n{SONNET_HIGH}"))];
    let (fx, launch, _) = running(PROFILE, &tasks, config::Orchestrator::default(), |run| {
        run.orch.installed.insert("codex".into(), false);
        let route = run.tasks[0].route.clone();
        run.tasks[0].list_pick = Some(crate::run::model::ListPick {
            candidates: vec![proto::RoutingCandidate {
                route,
                skipped_reason: None,
            }],
            chosen: Some(0),
            pick: Default::default(),
            slot: None,
        });
    });
    assert_eq!(launch.spec.runtime, Runtime::Claude, "the task's own route");
    let t1 = fx.task("t1");
    let writer = (t1.routing_decisions.iter())
        .find(|d| d.role == AgentRole::TestWriter)
        .unwrap();
    assert_eq!(
        (writer.trigger.as_str(), writer.source.as_str()),
        ("test_writer", "configured_list")
    );
    assert_eq!(writer.pick_policy.as_deref(), Some("first"));
}
