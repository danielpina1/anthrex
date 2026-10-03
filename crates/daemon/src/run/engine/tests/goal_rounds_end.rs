//! Milestone 9.3 task 4b: a round's own gate (decision 12), its reject and cancel
//! (decisions 12, 16), its end and `round` history line (decision 17), its summary, and
//! the limits each round counts from its start (decision 14, D5).

use std::collections::BTreeSet;

use proto::{
    BlockReason, HISTORY_VERSION, RoundLine, RoundOrigin, RoundOutcome, RunState, TaskState,
};
use serde_json::json;

use super::fixture::*;
use super::goal_rounds_stages::{add_in, creating, plan_round};
use super::goal_rounds_start::{complete, iterate, reply, round_lines, started};
use super::kinds_cancel::{cancel, settle_all};
use super::kinds_integration::{C1, C2, merge_real};
use super::merge::pending_one;
use super::orch::{add, answer, edit_plan, error, launched};
use super::orch_restore::restart;
use super::run_scouts::{scout, scout_ended};
use super::scenarios::halt_and_rebaseline;
use super::wake_notes::clear;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult, OrchEvent, ScoutEnd};
use crate::run::model::{StageLayout, StageRecord};

/// [`complete`] with round 1 approved in stages (set in place, as
/// `fix_layout_is_not_run_again_for_a_round` does): stage 1 holds `t1` at `C1`.
pub(super) fn complete_staged() -> Fixture {
    let mut fx = complete();
    let run = fx.run_mut();
    run.stage_layout = StageLayout::Multi;
    let branch = format!("anthrex/{RUN_ID}/stage-1");
    let tasks_in = BTreeSet::from(["t1".to_string()]);
    run.stages = vec![StageRecord::new(1, branch, C1, tasks_in, 1_500)];
    fx
}

/// The orchestrator plans round 2's `t2` in stage 2 and submits it.
pub(super) fn submit_round(fx: &mut Fixture) -> Vec<Effect> {
    let edits = json!([add_in("t2", "mail", 2, &[])]);
    let effects = edit_plan(fx, json!({"edits": edits, "submit": true}));
    assert!(answer(&effects).0, "{effects:#?}");
    effects
}

/// Every stage branch the running round needs, created.
pub(super) fn create_stages(fx: &mut Fixture) {
    for _ in 0..4 {
        let ops = creating(fx);
        let Some((op, ..)) = ops.first() else {
            return;
        };
        fx.done(*op, OpResult::StageCreated);
    }
}

/// [`complete_staged`] iterated by the user, round 2's `t2` approved and its stage
/// created.
fn round_two_running() -> Fixture {
    let mut fx = complete_staged();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(&mut fx);
    fx
}

/// Every diff measure, history line and worktree removal in flight answered.
pub(super) fn settle_ops(fx: &mut Fixture) {
    for _ in 0..8 {
        let ops: Vec<_> = fx
            .run()
            .pending_ops
            .values()
            .filter_map(|p| match p.kind {
                OpKind::MeasureDiff { .. } => {
                    Some((p.op, OpResult::DiffMeasured(Default::default())))
                }
                OpKind::AppendHistory { .. } => Some((p.op, OpResult::HistoryAppended)),
                OpKind::RemoveWorktree { .. } => {
                    Some((p.op, OpResult::Removed { salvage_ref: None }))
                }
                _ => None,
            })
            .collect();
        if ops.is_empty() {
            return;
        }
        for (op, result) in ops {
            fx.done(op, result);
        }
    }
}

/// The ref guard after every task finished: the run completes.
pub(super) fn verify(fx: &mut Fixture) -> Vec<Effect> {
    settle_ops(fx);
    fx.tick();
    let (op, _) = pending_one(fx, "VerifyRefs", None);
    let effects = fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
    effects
}

/// Every wake text among `effects`.
fn wake_texts(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::WakeOrchestrator { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

const SUMMARY_WAKE: &str = "write your summary with edit_plan summary";

/// Decision 12: a round the orchestrator started waits at the gate even with `--yes`.
#[test]
fn an_orchestrator_round_always_stops_at_the_gate() {
    let mut fx = complete();
    fx.run_mut().orch.yes = true;
    let approved_at = fx.run().approved_at;
    assert!(answer(&edit_plan(&mut fx, json!({"iterate": "add docs"}))).0);
    submit_round(&mut fx);
    let run = fx.run();
    assert_eq!(run.rounds[1].origin, RoundOrigin::Orchestrator);
    assert_eq!(run.state, RunState::AwaitingApproval);
    assert_eq!(run.approved_at, approved_at);
}

/// Decision 12: a user's round skips the gate only when the run was started with
/// approve at once; round 1's approval stays the run's.
#[test]
fn a_user_round_skips_the_gate_only_with_approve_at_once() {
    for yes in [true, false] {
        let mut fx = complete();
        fx.run_mut().orch.yes = yes;
        let (approved_at, approved_by) = (fx.run().approved_at, fx.run().approved_by.clone());
        assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
        submit_round(&mut fx);
        let run = fx.run();
        let want = if yes {
            RunState::Running
        } else {
            RunState::AwaitingApproval
        };
        assert_eq!(run.state, want, "yes = {yes}");
        assert_eq!(run.approved_at, approved_at, "yes = {yes}");
        assert_eq!(run.approved_by, approved_by, "yes = {yes}");
    }
}

/// Decision 12: approving round 2 runs its tasks; the layout is not fixed again and
/// round 1's approval time stays.
#[test]
fn approving_a_round_runs_its_tasks_without_fixing_the_layout_again() {
    let mut fx = complete_staged();
    let (approved_at, stage_one) = (fx.run().approved_at, fx.run().stages[0].clone());
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    let logged = fx.run().log.len();
    submit_round(&mut fx);
    let effects = fx.approve();
    assert_eq!(
        super::dispatch::replies(&effects),
        vec![Ok(format!("run {RUN_ID} approved"))]
    );
    create_stages(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.approved_at, approved_at);
    assert_eq!(run.stage_layout, StageLayout::Multi);
    assert_eq!(run.stages.first(), Some(&stage_one), "stage 1 kept");
    let fixed = run.log[logged..]
        .iter()
        .any(|l| l.text.starts_with("approved with"));
    assert!(!fixed, "{:#?}", run.log);
    let prepared: Vec<String> = fx
        .ops("PrepareWorktree")
        .iter()
        .map(|(_, k)| op_task(k))
        .collect();
    assert!(prepared.iter().any(|t| t == "t2"), "{prepared:?}");
    assert_ne!(fx.task("t2").state, TaskState::Pending);
}

/// Decision 12: rejecting round 2 cancels its tasks, ends it `rejected` and returns the
/// run to `complete`, with round 1 and stage 1 as they were and no discard.
#[test]
fn rejecting_a_round_returns_to_complete_with_round_one_intact() {
    let mut fx = complete_staged();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    clear(&mut fx);
    submit_round(&mut fx);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    let (t1, stages) = (fx.task("t1").clone(), fx.run().stages.clone());
    let started_at = fx.run().rounds[1].started_at;
    let reply_id = fx.reply();
    let effects = fx.next(EventKind::Reject {
        reply: reply_id,
        run_id: RUN_ID.into(),
    });
    assert_eq!(
        reply(&effects),
        Ok("run 3f9a round 2 rejected; the earlier rounds are unchanged".into())
    );
    let now = fx.now;
    let run = fx.run();
    assert_eq!(run.state, RunState::Complete);
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Rejected));
    assert_eq!(run.rounds[1].ended_at, Some(now));
    assert_eq!(run.rounds[0].outcome, Some(RoundOutcome::Completed));
    assert_eq!(run.orch.request_wake, None);
    assert_eq!(fx.task("t1"), &t1);
    assert_eq!(run.stages, stages);
    assert!(ops_in(&effects, "Discard").is_empty(), "{effects:#?}");
    let summary = |e: &[Effect]| wake_texts(e).iter().any(|t| t.contains(SUMMARY_WAKE));
    assert!(!summary(&effects), "{effects:#?}");
    let effects = fx.tick();
    assert!(!summary(&effects) && ops_in(&effects, "Discard").is_empty());
    let line = RoundLine {
        v: HISTORY_VERSION,
        record_id: format!("{RUN_ID}/round/2"),
        at: now,
        run_id: RUN_ID.into(),
        round: 2,
        origin: RoundOrigin::User,
        outcome: RoundOutcome::Rejected,
        tasks: 1,
        merged: 0,
        calls: 0,
        minutes: (now - started_at) / 60,
    };
    let lines: Vec<_> = round_lines(&fx.log)
        .into_iter()
        .filter(|l| l.round == 2)
        .collect();
    assert_eq!(lines, vec![line]);
}

/// Fix round 1 (I2): a request wake is its round's. Round 2's request is pasted, the user
/// rejects round 2 and iterates round 3, and only then does round 2's
/// `OrchestratorWoken` arrive: it leaves round 3's request waiting, which only round
/// 3's own clears.
#[test]
fn a_late_woken_of_a_rejected_round_leaves_the_next_rounds_request() {
    let mut fx = complete_staged();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    clear(&mut fx);
    submit_round(&mut fx);
    let (revision, seq) = (fx.run().orch.digest_rev, super::super::notes_seq(fx.run()));
    let reply_id = fx.reply();
    fx.next(EventKind::Reject {
        reply: reply_id,
        run_id: RUN_ID.into(),
    });
    assert_eq!(
        fx.run().orch.request_wake,
        None,
        "round 2's own reject clears it"
    );
    assert_eq!(reply(&iterate(&mut fx, "again")), started(3));
    let woken = |fx: &mut Fixture, request| {
        fx.next(EventKind::Orch(OrchEvent::OrchestratorWoken {
            run_id: RUN_ID.into(),
            digest_revision: revision,
            notes_seq: seq,
            request: Some(request),
            first_turn: false,
        }))
    };
    woken(&mut fx, 2);
    assert!(
        fx.run().orch.request_wake.is_some(),
        "round 2's woken cleared round 3's"
    );
    woken(&mut fx, 3);
    assert_eq!(fx.run().orch.request_wake, None);
}

/// Decision 12 (pinning): round 1's reject still discards the run.
#[test]
fn rejecting_round_one_still_discards_the_run() {
    let mut fx = launched(false);
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    assert!(answer(&effects).0);
    let reply_id = fx.reply();
    let effects = fx.next(EventKind::Reject {
        reply: reply_id,
        run_id: RUN_ID.into(),
    });
    assert_eq!(
        reply(&effects),
        Ok(format!("run {RUN_ID} rejected; discarding it"))
    );
    assert_eq!(ops_in(&effects, "Discard").len(), 1, "{effects:#?}");
}

/// Decision 16: `run cancel` during round 2 cancels that round only; the round ends
/// `cancelled` once its sessions have ended, and the run can be iterated again.
#[test]
fn cancelling_a_round_ends_it_cancelled() {
    let mut fx = round_two_running();
    let windows = fx.launch_all();
    assert!(windows.iter().any(|(t, _)| t == "t2"), "{windows:?}");
    let t1 = fx.task("t1").clone();
    let effects = cancel(&mut fx);
    assert_eq!(
        reply(&effects),
        Ok("run 3f9a round 2 cancelled; it ends once its sessions have ended".into())
    );
    let run = fx.run();
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
    assert!(run.finish_edit, "nothing new starts while the round ends");
    assert!(!run.cancelled);
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Cancelled));
    assert_eq!(run.rounds[1].ended_at, None);
    settle_all(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Complete);
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Cancelled));
    assert!(run.rounds[1].ended_at.is_some());
    assert!(!run.finish_edit && !run.cancelled);
    assert_eq!(fx.task("t1"), &t1);
    let outcomes: Vec<_> = round_lines(&fx.log)
        .iter()
        .map(|l| (l.round, l.outcome))
        .collect();
    assert_eq!(
        outcomes,
        vec![(1, RoundOutcome::Completed), (2, RoundOutcome::Cancelled)]
    );
    assert_eq!(reply(&iterate(&mut fx, "again")), started(3));
}

/// `t2` waits at the head of the queue when its merge finds the run ref moved.
pub(super) fn halted_in_round_two() -> Fixture {
    let mut fx = round_two_running();
    fx.run_mut()
        .pending_ops
        .retain(|_, p| p.task_id.as_deref() != Some("t2"));
    let task = fx.task_mut("t2");
    task.state = TaskState::MergeQueue;
    task.head = Some(HEAD.into());
    task.spent_total.tool_calls = 5;
    fx.run_mut().merge_queue.push("t2".into());
    fx.tick();
    let (op, _) = pending_one(&fx, "MergeCandidate", Some("t2"));
    let reason = format!("refs/heads/anthrex/{RUN_ID}/stage-2 moved from c1c1c1c to 9999999");
    fx.done(op, OpResult::RefMoved { reason });
    fx
}

/// KG §2.4 (pinning: M8a's halt): a task that fails in a round halts the run; the
/// round stays open and round 1 as it was.
#[test]
fn a_failing_task_in_a_round_halts_the_run() {
    let fx = halted_in_round_two();
    let run = fx.run();
    assert_eq!(run.state, RunState::Halted);
    assert_eq!(run.round(), 2);
    assert_eq!(run.rounds[1].outcome, None);
    assert_eq!(run.rounds[1].ended_at, None);
    assert_eq!(run.rounds[0].outcome, Some(RoundOutcome::Completed));
    assert_eq!(fx.task("t2").state, TaskState::MergeQueue);
}

/// KG §2.4: the user resumes; the round's work merges and the run completes, which
/// ends the round `completed` with its summary wake.
#[test]
fn resume_then_completion_ends_the_round() {
    let mut fx = halted_in_round_two();
    halt_and_rebaseline(&mut fx, vec![(1, C1.into()), (2, C1.into())]);
    assert_eq!(fx.run().state, RunState::Running);
    let (op, _) = pending_one(&fx, "MergeCandidate", Some("t2"));
    let merged = OpResult::Merged {
        commit: C2.into(),
        tier: None,
    };
    fx.done(op, merged);
    let effects = verify(&mut fx);
    let run = fx.run();
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Completed));
    assert_eq!(run.rounds[1].ended_at, Some(fx.now));
    let lines = round_lines(&effects);
    assert_eq!(lines.len(), 1, "{effects:#?}");
    assert_eq!((lines[0].round, lines[0].merged), (2, 1));
    let wakes = wake_texts(&effects);
    assert!(
        wakes.iter().any(|t| t.contains(SUMMARY_WAKE)),
        "{effects:#?}"
    );
}

/// Decision 17: the end of a round writes one `round` line, its fields exact; neither a
/// restart nor a later pass writes another.
#[test]
fn the_end_of_a_round_writes_one_round_line() {
    let mut fx = complete();
    fx.task_mut("t1").spent_total.tool_calls = 7;
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(&mut fx);
    // The widened run's stage 1 starts at the run head, which holds round 1's `t1`.
    assert!(fx.run().stages[0].tasks_in.contains("t1"));
    fx.task_mut("t2").spent_total.tool_calls = 5;
    merge_real(&mut fx, "t2", C2);
    // Ten minutes pass before the ref guard.
    fx.send(fx.now + 600, EventKind::Tick);
    let started_at = fx.run().rounds[1].started_at;
    let effects = verify(&mut fx);
    let now = fx.now;
    assert!(now - started_at >= 600);
    let line = RoundLine {
        v: HISTORY_VERSION,
        record_id: format!("{RUN_ID}/round/2"),
        at: now,
        run_id: RUN_ID.into(),
        round: 2,
        origin: RoundOrigin::User,
        outcome: RoundOutcome::Completed,
        tasks: 1,
        merged: 1,
        calls: 5,
        minutes: (now - started_at) / 60,
    };
    assert_eq!(round_lines(&effects), vec![line]);
    assert!(!fx.run().finish_edit);
    settle_ops(&mut fx);
    let count = |fx: &Fixture| round_lines(&fx.log).iter().filter(|l| l.round == 2).count();
    fx.tick();
    restart(&mut fx);
    fx.tick();
    assert_eq!(fx.run().state, RunState::Complete);
    assert_eq!(count(&fx), 1);
}

/// Decision 17: the orchestrator's summary is stored on the current round too.
#[test]
fn the_summary_is_stored_on_the_round() {
    let mut fx = complete();
    let summary = |fx: &mut Fixture, text: &str| {
        let effects = edit_plan(fx, json!({"edits": [], "summary": text}));
        assert!(answer(&effects).0, "{effects:#?}");
    };
    summary(&mut fx, "round one done");
    assert_eq!(
        fx.run().rounds[0].summary.as_deref(),
        Some("round one done")
    );
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(&mut fx);
    merge_real(&mut fx, "t2", C2);
    verify(&mut fx);
    summary(&mut fx, "round two done");
    let run = fx.run();
    assert_eq!(run.rounds[0].summary.as_deref(), Some("round one done"));
    assert_eq!(run.rounds[1].summary.as_deref(), Some("round two done"));
    let o = run.orch.orchestrator.as_ref().unwrap();
    assert_eq!(o.summary.as_deref(), Some("round two done"));
}

/// Decision 14 (D5): `max_tasks`, `max_windows` and `max_scouts` count from the round's
/// start. Round 1 uses each up; round 2 adds two tasks, opens a window and starts a
/// scout, and its third task is refused with `validate_graph`'s text.
#[test]
fn each_round_counts_its_own_limits() {
    let mut fx = launched(false);
    let limits = &mut fx.run_mut().limits;
    limits.max_tasks = 2;
    limits.max_windows = 1;
    limits.orch.max_scouts = 1;
    // Round 1: the orchestrator's window is the one window allowed.
    assert_eq!(fx.run().windows_created, 1);
    let too_many = "3 tasks exceed max_tasks (2)";
    let rejected = |effects: &[Effect]| {
        let (ok, value) = answer(effects);
        assert!(!ok, "{value}");
        value["errors"][0]["message"].as_str().unwrap().to_string()
    };
    let edits = json!({"edits": [add("t1", "auth"), add("t2", "api")]});
    assert!(answer(&edit_plan(&mut fx, edits)).0);
    let third = json!({"edits": [add("t3", "cli")]});
    assert_eq!(rejected(&edit_plan(&mut fx, third)), too_many);
    assert!(answer(&scout(&mut fx, "a")).0);
    assert_eq!(
        error(&scout(&mut fx, "b")),
        format!("run {RUN_ID} already has 1 scouts, the most max_scouts allows")
    );
    let (op, _) = fx.op("StartScout");
    fx.done(op, OpResult::ScoutStarted { window_id: 77 });
    scout_ended(&mut fx, &format!("{H4}-a"), ScoutEnd::Reported);
    assert!(answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true}))).0);
    fx.approve();
    fx.complete_prepares();
    let blocked = fx.run().tasks.iter().any(|t| {
        t.state == TaskState::Blocked
            && t.block.as_ref().is_some_and(|b| {
                b.reason == BlockReason::Environment && b.text == "run window limit (1) reached"
            })
    });
    assert!(blocked, "round 1 used its window up: {:#?}", fx.run().tasks);
    merge_real(&mut fx, "t1", C1);
    merge_real(&mut fx, "t2", C2);
    verify(&mut fx);
    // Round 2 counts from its start. Its tasks name round 1's scout report (rule 7.1).
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    let add_in = |id: &str, module: &str| {
        let mut edit = add_in(id, module, 2, &[]);
        edit["task"]["scout_refs"] = json!([format!("{H4}-a")]);
        edit
    };
    let edits = json!({"edits": [add_in("t3", "cli"), add_in("t4", "docs")]});
    let (ok, value) = answer(&edit_plan(&mut fx, edits));
    assert!(ok, "{value}");
    let third = json!({"edits": [add_in("t5", "web")]});
    assert_eq!(rejected(&edit_plan(&mut fx, third)), too_many);
    assert!(answer(&scout(&mut fx, "c")).0);
    // Fix round 1 (m5): round 2's own second scout is refused.
    assert_eq!(
        error(&scout(&mut fx, "d")),
        format!("run {RUN_ID} already has 1 scouts, the most max_scouts allows")
    );
    assert!(answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true}))).0);
    fx.approve();
    create_stages(&mut fx);
    fx.complete_prepares();
    let windows: Vec<String> = fx
        .run()
        .pending_ops
        .values()
        .filter(|p| matches!(p.kind, OpKind::CreateWindow { .. }))
        .filter_map(|p| p.task_id.clone())
        .collect();
    assert_eq!(windows.len(), 1, "round 2 opens one window: {windows:?}");
    assert!(["t3", "t4"].contains(&windows[0].as_str()), "{windows:?}");
}
