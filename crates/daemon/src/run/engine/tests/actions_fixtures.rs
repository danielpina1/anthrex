//! Milestone 9.0.6 task 7: one engine fixture per run state the action menu meets,
//! each built through the engine (`start`, `ready`, `approve`, tool calls, op results,
//! requests) wherever a unit fixture can reach the state, and set in place only where
//! it cannot without an agent or a daemon restart (each such line says so).

use proto::{
    AgentRole, BlockInfo, BlockReason, FinishAction, MessageKind, PlanEdit, RunState, TaskState,
};
use serde_json::json;

use super::actions_twins::hold_stage;
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::gate_holds::awaiting;
use super::kinds::research;
use super::merge::{
    commit, doc_task, merge as merge_task, pending, start, start_on, to_queue, window_of,
};
use super::orch::{add, edit_plan, launched};
use super::promote::{fast, hold_verdict, promoted};
use super::worker_messages::{ack, msg};
use crate::run::engine::actions::ActionNode;
use crate::run::engine::{EventKind, OpResult};
use crate::run::model::{Run, StageLayout};

/// A fixture's builder.
pub(super) type Build = fn() -> Fixture;

/// Every fixture by name.
pub(super) const FIXTURES: &[(&str, Build)] = &[
    ("gate", gate),
    ("gate_dormant", gate_dormant),
    ("planning", planning),
    ("planning_empty", planning_empty),
    ("planning_paused", planning_paused),
    ("running", running),
    ("paused", paused),
    ("halted", halted),
    ("halted_retryable", halted_retryable),
    ("held_tier3", held_tier3),
    ("with_awaiting_hold", with_awaiting_hold),
    ("with_approved_hold", with_approved_hold),
    ("fast_path", fast_path),
    ("fast_path_promoted", fast_path_promoted),
    ("multi_stage", multi_stage),
    ("finishing", finishing),
    ("complete", complete),
    ("complete_with_moved_base", complete_with_moved_base),
    ("complete_orchestrated", complete_orchestrated),
    ("cancelled_halted", cancelled_halted),
    ("accepted", accepted),
    ("discarded", discarded),
    ("failed", failed),
];

/// The fixture called `name`, built.
pub(super) fn named(name: &str) -> Fixture {
    let (_, build) = FIXTURES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no fixture {name}"));
    build()
}

/// A node of a run, owning its task id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OwnedNode {
    Run,
    Stage(u16),
    Task(String),
}

impl OwnedNode {
    pub(super) fn as_node(&self) -> ActionNode<'_> {
        match self {
            OwnedNode::Run => ActionNode::Run,
            OwnedNode::Stage(n) => ActionNode::Stage(*n),
            OwnedNode::Task(id) => ActionNode::Task(id),
        }
    }
}

/// The run, every stage of a `Multi` run, and every task.
pub(super) fn nodes(run: &Run) -> Vec<OwnedNode> {
    let mut out = vec![OwnedNode::Run];
    if run.stage_layout == StageLayout::Multi {
        let last = run.tasks.iter().map(|t| t.stage()).max().unwrap_or(1);
        out.extend((1..=last).map(OwnedNode::Stage));
    }
    out.extend(
        run.tasks
            .iter()
            .map(|t| OwnedNode::Task(t.id().to_string())),
    );
    out
}

/// Three tasks, `t2` after `t1`, at the plan gate, the run branch made.
pub(super) fn gate() -> Fixture {
    let plan = plan_with(
        PROFILE,
        &[
            task("t1", "S", "a", ""),
            task("t2", "S", "b", "deps = [\"t1\"]"),
            task("t3", "S", "c", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(false);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    fx
}

/// A planned run whose orchestrator submitted `t1`, at its gate, its orchestrator
/// dormant after a daemon restart (`orch_window::restored`, as the restore leaves it):
/// a resume relaunches it (milestone 9 decision 11).
fn gate_dormant() -> Fixture {
    let mut fx = launched(false);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    crate::run::engine::orch_window::restored(fx.run_mut());
    assert!(crate::run::engine::orch_window::relaunchable(fx.run()));
    fx
}

/// A planned run whose orchestrator added `t1` and has not submitted.
fn planning() -> Fixture {
    let mut fx = launched(false);
    edit_plan(&mut fx, json!({"edits": [add("t1", "auth")]}));
    assert_eq!(fx.run().state, RunState::Planning);
    assert_eq!(fx.run().tasks.len(), 1);
    fx
}

/// A planned run whose orchestrator has added nothing yet: its submit is refused.
fn planning_empty() -> Fixture {
    launched(false)
}

/// [`planning`], paused there by a daemon restart, which a unit fixture cannot do: set
/// in place as `restore` leaves it.
fn planning_paused() -> Fixture {
    let mut fx = planning();
    fx.run_mut().state = RunState::Paused;
    fx.run_mut().paused_from = Some(RunState::Planning);
    fx
}

/// The window `task` was launched in.
fn window(windows: &[(String, u32)], task: &str) -> u32 {
    window_of(windows, task)
}

/// `task` blocks itself with a question from `window`.
fn ask(fx: &mut Fixture, window: u32, task: &str) {
    let args = json!({"kind": "question", "reason": "which table?"});
    let effects = fx.tool_as(AgentRole::Worker, window, task, "task_blocked", args);
    assert!(replies(&effects).iter().all(Result::is_ok), "{effects:#?}");
}

/// A running run with a task in every state a task node meets: `pending` (waits for
/// `working`), `working`, `question` (blocked(question)), `human` (blocked(human)),
/// `depcancel` (blocked(dep_cancelled), its dependency `doomed` cancelled), `review`,
/// `mergeq` (merge_queue), `merged`, `paused` (paused(message)), `big` (an L task,
/// blocked) and `research`.
pub(super) fn running() -> Fixture {
    let plan = plan_with(
        &profile_with("max_writers = 8"),
        &[
            task("pending", "S", "a", "deps = [\"working\"]"),
            task("working", "S", "b", ""),
            task("question", "S", "c", ""),
            task("human", "S", "d", ""),
            task("doomed", "S", "e", "deps = [\"working\"]"),
            task("depcancel", "S", "f", "deps = [\"doomed\"]"),
            task("review", "S", "g", ""),
            task("mergeq", "S", "h", ""),
            task("merged", "S", "i", ""),
            task("paused", "S", "j", ""),
            task("big", "S", "k", ""),
            research("research", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let windows = fx.launch_all();
    ask(&mut fx, window(&windows, "question"), "question");
    ask(&mut fx, window(&windows, "big"), "big");
    // A plan's L task never runs (rule 7.2.4); a size check raises a running one to L.
    // The deciders are off here, so the size is set in place as the decider leaves it.
    fx.task_mut("big").size = proto::Size::L;
    // A bounce or budget cap blocks a task for a human; reaching one needs a worker's
    // whole gate history, so the block is set in place as `ladder` leaves it.
    let human = fx.task_mut("human");
    human.state = TaskState::Blocked;
    human.block = Some(BlockInfo {
        reason: BlockReason::Human,
        text: "bounced three times at the check".into(),
    });
    let cancel = PlanEdit::CancelTask {
        task_id: "doomed".into(),
    };
    assert!(replies(&edit(&mut fx, vec![cancel]))[0].is_ok());
    let paused = window(&windows, "paused");
    edit(
        &mut fx,
        vec![msg(&["paused"], MessageKind::StopAndWait, "hold on")],
    );
    let effects = fx.turn_completed(paused);
    ack(&mut fx, &effects);
    fx.turn_completed(paused);
    fx.force("review", TaskState::Review);
    fx.force("mergeq", TaskState::MergeQueue);
    fx.merge("merged", &commit(1));
    fx
}

/// The two-task run `t1`, `t2` (after `t1`), running.
fn two_running() -> Fixture {
    super::actions_rules::running()
}

/// [`two_running`], paused by the user's `pause` edit.
fn paused() -> Fixture {
    let mut fx = two_running();
    assert!(replies(&edit(&mut fx, vec![PlanEdit::Pause]))[0].is_ok());
    assert_eq!(fx.run().state, RunState::Paused);
    fx
}

/// `t1` in the merge queue, `t2` working, when the merge finds the run ref moved
/// (decision 21): halted, not retryable.
pub(super) fn halted() -> Fixture {
    let profile = profile_with("max_writers = 2");
    let (mut fx, windows) = start_on(&profile, &[doc_task("t1", ""), doc_task("t2", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (op, _) = pending(&fx, "MergeCandidate", Some("t1"))[0].clone();
    let reason = format!("refs/heads/anthrex/{RUN_ID}/integration moved from 1eeeeee to 9999999");
    fx.done(op, OpResult::RefMoved { reason });
    assert_eq!(fx.run().state, RunState::Halted);
    assert!(!fx.run().halt_retryable);
    fx
}

/// The one task of a run merged; the ref guard could not read the refs twice: halted,
/// and a plain resume retries it (review m1).
fn halted_retryable() -> Fixture {
    let mut fx = all_merged();
    for _ in 0..2 {
        fx.tick();
        let (op, _) = pending(&fx, "VerifyRefs", None)[0].clone();
        let message = "git timed out".to_string();
        fx.done(op, OpResult::Failed { message });
    }
    assert_eq!(fx.run().state, RunState::Halted);
    assert!(fx.run().halt_retryable);
    fx
}

/// [`two_running`] with its stage's tier 3 held by three executor failures (ruling
/// C-18), which needs the executor: set in place (`actions_twins::hold_stage`).
fn held_tier3() -> Fixture {
    let mut fx = two_running();
    hold_stage(fx.run_mut());
    fx
}

/// A running run whose orchestrator's new epic `mail` holds `t2`, awaiting approval.
fn with_awaiting_hold() -> Fixture {
    let mut fx = super::gate_holds::held(false);
    awaiting(&mut fx);
    fx
}

/// [`with_awaiting_hold`], its hold approved by the user.
fn with_approved_hold() -> Fixture {
    let mut fx = with_awaiting_hold();
    assert!(replies(&hold_verdict(&mut fx, "epic:mail", true))[0].is_ok());
    fx
}

/// A running fast-path run of `t1` (promotable).
fn fast_path() -> Fixture {
    fast()
}

/// [`fast_path`], promoted, its orchestrator launched and not submitted.
fn fast_path_promoted() -> Fixture {
    promoted()
}

/// A running run of `t1` (stage 1) and `t2` (stage 2), stage 1's branch created.
fn multi_stage() -> Fixture {
    let (mut fx, _) = start(&[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    assert_eq!(fx.run().stage_layout, StageLayout::Multi);
    let (op, _) = pending(&fx, "CreateStageBranch", None)[0].clone();
    fx.done(op, OpResult::StageCreated);
    fx
}

/// Two check-mode tasks, `t1` and `t2`, each merged.
fn all_merged() -> Fixture {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge_task(&mut fx, "t1", &commit(1));
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge_task(&mut fx, "t2", &commit(2));
    fx
}

/// [`all_merged`], its refs verified and its final check green: complete.
pub(super) fn complete() -> Fixture {
    let mut fx = all_merged();
    for _ in 0..4 {
        if fx.run().state == RunState::Complete {
            break;
        }
        fx.tick();
        if let Some((op, _)) = pending(&fx, "VerifyRefs", None).first().cloned() {
            fx.done(op, OpResult::RefsOk);
        } else if let Some((op, _)) = pending(&fx, "Check", None).first().cloned() {
            fx.done(op, super::gates::check_result(true));
        }
    }
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
    fx
}

/// Milestone 9.3: a planned run whose orchestrator's task merged, complete: the one
/// fixture state a round can start from (decision 9).
fn complete_orchestrated() -> Fixture {
    super::goal_rounds_start::complete()
}

/// The base head after an advance.
pub(super) const MOVED_TO: &str = "abababababababababababababababababababab";

/// [`complete`], its base advanced by two commits.
fn complete_with_moved_base() -> Fixture {
    let mut fx = complete();
    fx.next(EventKind::BaseAdvanced {
        run_id: RUN_ID.into(),
        to: MOVED_TO.into(),
        commits: 2,
    });
    assert!(fx.run().base_moved.is_some());
    fx
}

/// [`complete`] with its `Accept` op in flight.
fn finishing() -> Fixture {
    let mut fx = complete();
    let reply = fx.reply();
    fx.next(EventKind::Finish {
        reply,
        run_id: RUN_ID.into(),
        action: FinishAction::Accept,
    });
    assert!(!pending(&fx, "Accept", None).is_empty());
    fx
}

/// [`halted`], cancelled while halted, every session ended: what `complete.rs`'s
/// discard of a cancelled halted run takes (review m2). Cancelling the run cancels
/// its tasks through the engine; the sessions' ends and the in-flight merge's
/// answer need agents, so the remaining ops are dropped in place.
fn cancelled_halted() -> Fixture {
    let mut fx = halted();
    let reply = fx.reply();
    assert!(
        replies(&fx.next(EventKind::Cancel {
            reply,
            run_id: RUN_ID.into(),
        }))[0]
            .is_ok()
    );
    let run = fx.run_mut();
    for task in &mut run.tasks {
        if !task.state.is_finished() {
            task.state = TaskState::Cancelled;
        }
    }
    run.pending_ops.clear();
    assert_eq!(fx.run().state, RunState::Halted);
    fx
}

/// [`complete`], accepted: its `Accept` op finished.
fn accepted() -> Fixture {
    let mut fx = finishing();
    let (op, _) = pending(&fx, "Accept", None)[0].clone();
    let outcome = "accepted".to_string();
    fx.done(
        op,
        OpResult::Finished {
            outcome,
            kept_branches: Vec::new(),
        },
    );
    assert_eq!(fx.run().state, RunState::Accepted);
    fx
}

/// [`gate`], rejected: its `Discard` op finished.
fn discarded() -> Fixture {
    let mut fx = gate();
    let reply = fx.reply();
    fx.next(EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    let (op, _) = pending(&fx, "Discard", None)[0].clone();
    let outcome = "discarded".to_string();
    fx.done(
        op,
        OpResult::Finished {
            outcome,
            kept_branches: Vec::new(),
        },
    );
    assert_eq!(fx.run().state, RunState::Discarded);
    fx
}

/// A run whose integration worktree could not be made (decision 31): failed.
fn failed() -> Fixture {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.start(true);
    let (op, _) = fx.op("CreateRunBranch");
    let message = "worktree add failed".to_string();
    fx.done(op, OpResult::Failed { message });
    assert_eq!(fx.run().state, RunState::Failed);
    fx
}

#[test]
fn every_fixture_builds() {
    for (name, build) in FIXTURES {
        let fx = build();
        assert!(
            *name == "planning_empty" || !fx.run().tasks.is_empty(),
            "{name}"
        );
    }
    let fx = running();
    let state = |id: &str| fx.task(id).state;
    assert_eq!(state("pending"), TaskState::Pending);
    assert_eq!(state("working"), TaskState::Working);
    assert_eq!(state("doomed"), TaskState::Cancelled);
    assert_eq!(state("review"), TaskState::Review);
    assert_eq!(state("mergeq"), TaskState::MergeQueue);
    assert_eq!(state("merged"), TaskState::Merged);
    let reason = |id: &str| fx.task(id).block.as_ref().map(|b| b.reason);
    assert_eq!(reason("question"), Some(BlockReason::Question));
    assert_eq!(reason("big"), Some(BlockReason::Question));
    assert_eq!(reason("depcancel"), Some(BlockReason::DepCancelled));
    assert_eq!(reason("paused"), Some(BlockReason::MessagePause));
}
