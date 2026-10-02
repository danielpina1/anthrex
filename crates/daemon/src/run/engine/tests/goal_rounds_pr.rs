//! Milestone 9.3 task 5: rounds in `pr` mode (KG §2.3, §2.5; decisions 9, 13, 16, 17).
//! When a `pr` run is settled and each refusal of one that is not, a round's stage
//! stacked on the open PR below it, a round above landed PRs starting after a fetch of
//! the base, a round's reject and cancel leaving the earlier PRs delivering and
//! watched, and the end of a `pr` round asking for its summary. Host answers are
//! scripted `OpResult::Host` values (9.2's delivery fixtures): no host runs here.

use proto::{MergeMethod, PrState, RoundOutcome, RunState, TaskOrigin, TaskState};
use serde_json::json;

use super::delivery_land::{pr_record, stage_lines};
use super::delivery_open::{answer as host_answer, green, host_ops, opened, pr_mode};
use super::delivery_sync::{base_fetch, base_sync, fetched};
use super::delivery_watch_adopt::{poll_stage, view_of};
use super::fixes::spec as fixes_spec;
use super::fixture::*;
use super::goal_rounds_end::{create_stages, settle_ops, submit_round};
use super::goal_rounds_stages::{add_in, creating, plan_round};
use super::goal_rounds_start::{complete, iterate, reply, round_lines, started};
use super::kinds_cancel::cancel;
use super::kinds_integration::{C2, merge_real};
use super::merge::{commit, pending_one};
use super::orch::{answer, edit_plan};
use super::propagate::merged_at;
use crate::host::PushOutcome;
use crate::run::delivery::StageDelivery;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::delivery::snapshot::stage_count;
use crate::run::engine::fixes::add_fix;
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{AgentSignal, Effect, EventKind};
use crate::run::model::OpId;

/// Decision 9's refusal of a `pr` run that is running but not settled.
const NOT_SETTLED: &str = "run 3f9a is running; iterate it when it completes";
/// Decision 17's note at the end of round 2 of a `pr` run (brief text).
const ROUND_DONE: &str = "round 2 is done; write its summary with edit_plan summary";

/// [`complete`] delivered by pull request: `running`, stage 1's PR #12 open on the run
/// head and nothing in progress (set in place: delivering needs a host).
pub(super) fn delivering() -> Fixture {
    let mut fx = complete();
    let head = fx.run().run_head.clone();
    let run = fx.run_mut();
    pr_mode(run);
    run.state = RunState::Running;
    let mut pr = pr_record(12, PrState::Open);
    pr.pushed_head = head;
    run.delivery.stages = vec![StageDelivery {
        pr: Some(pr),
        ..StageDelivery::default()
    }];
    fx.tick();
    assert_eq!(fx.run().state, RunState::Running, "{:#?}", fx.run().log);
    fx
}

/// [`complete`] in `pr` mode with stage 1's PR #12 squash-merged: the run `complete`.
pub(super) fn landed() -> Fixture {
    let mut fx = complete();
    let run = fx.run_mut();
    pr_mode(run);
    let mut pr = pr_record(12, PrState::Merged);
    pr.merge_commit = Some(commit(70));
    pr.merge_method = Some(MergeMethod::SquashOrRebase);
    run.delivery.stages = vec![StageDelivery {
        pr: Some(pr),
        landed: Some(PrState::Merged),
        history_written: true,
        ..StageDelivery::default()
    }];
    fx
}

/// `run reject` of the fixture run.
pub(super) fn reject(fx: &mut Fixture) -> Vec<Effect> {
    let reply_id = fx.reply();
    fx.next(EventKind::Reject {
        reply: reply_id,
        run_id: RUN_ID.into(),
    })
}

/// The iterate's refusal (or acceptance) on `fx`, the run's rounds unchanged on refusal.
fn iterated(mut fx: Fixture) -> Result<String, String> {
    let result = reply(&iterate(&mut fx, "more"));
    if result.is_err() {
        assert_eq!(fx.run().rounds.len(), 1);
    }
    result
}

/// Round 2's `t2` in stage 2, planned, approved, its stages created and `t2` merged
/// at `C2`, then stage 2's PR opened as #13 (tier 3 green, pushed). Returns the
/// `OpenPr` and the effects of the step that recorded it.
fn deliver_round_two(fx: &mut Fixture) -> (HostOp, Vec<Effect>) {
    plan_round(fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(fx);
    deliver_after_plan(fx)
}

/// [`deliver_round_two`] for a round already planned and its stages created.
fn deliver_after_plan(fx: &mut Fixture) -> (HostOp, Vec<Effect>) {
    let (op, open) = pushed(fx);
    let effects = host_answer(fx, op, opened(13, false));
    (open, effects)
}

/// `t2` merged at `C2`, tier 3 green on stage 2 and its head pushed: the pending
/// `OpenPr` of stage 2.
fn pushed(fx: &mut Fixture) -> (OpId, HostOp) {
    merge_real(fx, "t2", C2);
    settle_ops(fx);
    fx.tick();
    if !host_ops(fx)
        .iter()
        .any(|(_, op)| matches!(op, HostOp::Push { .. }))
    {
        green(fx, 2);
    }
    let (op, push) = pushes(fx);
    let head = fx.run().stage_head(2).unwrap().to_string();
    assert_eq!(
        push,
        HostOp::Push {
            stage: 2,
            sha: head
        }
    );
    host_answer(fx, op, HostResult::Pushed(PushOutcome::Pushed));
    opens(fx)
}

/// [`delivering`] iterated, round 2 planned with its stages created and its PR about
/// to open.
pub(super) fn opening() -> (Fixture, OpId) {
    let mut fx = delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(&mut fx);
    let (op, _) = pushed(&mut fx);
    (fx, op)
}

/// The one pending push.
fn pushes(fx: &Fixture) -> (OpId, HostOp) {
    let ops: Vec<_> = (host_ops(fx).into_iter())
        .filter(|(_, op)| matches!(op, HostOp::Push { .. }))
        .collect();
    assert_eq!(ops.len(), 1, "one push: {:#?}", host_ops(fx));
    ops[0].clone()
}

/// The one pending `OpenPr`.
fn opens(fx: &Fixture) -> (OpId, HostOp) {
    let ops: Vec<_> = (host_ops(fx).into_iter())
        .filter(|(_, op)| matches!(op, HostOp::OpenPr { .. }))
        .collect();
    assert_eq!(ops.len(), 1, "one open: {:#?}", host_ops(fx));
    ops[0].clone()
}

/// Every window the engine killed exits, and every op that follows is answered.
pub(super) fn sessions_end(fx: &mut Fixture) {
    let killed: Vec<u32> = (fx.log.iter())
        .filter_map(|e| match e {
            Effect::KillWindow { window_id } => Some(*window_id),
            _ => None,
        })
        .collect();
    for window in killed {
        let exited = AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: true,
            pid: 0,
        };
        fx.signal(window, exited);
    }
    settle_ops(fx);
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

/// Round 2's `round` lines among the whole log, by outcome.
fn round_two_lines(fx: &Fixture) -> Vec<RoundOutcome> {
    round_lines(&fx.log)
        .into_iter()
        .filter(|l| l.round == 2)
        .map(|l| l.outcome)
        .collect()
}

/// Decision 9, step 7 (KG §2.3): a delivering `pr` run with nothing in progress starts a
/// round; one with a stage without its PR, a running task, a pending fix task, a held
/// host op or a due base sync is refused with the not-settled text.
#[test]
fn a_delivering_run_with_nothing_in_progress_can_be_iterated() {
    assert_eq!(iterated(delivering()), started(2));
    // A stage without its PR.
    refused(|fx| fx.run_mut().delivery.stages[0].pr = None);
    // A pending fix task (an engine-made bisect fix on stage 1), then a running one.
    refused(|fx| {
        fix(fx);
    });
    refused(|fx| {
        let id = fix(fx);
        fx.task_mut(&id).state = TaskState::Working;
    });
    // A held host op (a push the remote refused).
    refused(|fx| fx.run_mut().delivery.stages[0].held = Some("protected".into()));
    // A due base sync.
    refused(|fx| {
        let due = &mut fx.run_mut().delivery.base_sync_due;
        due.insert(1, commit(71));
    });
}

/// [`delivering`] changed by `change`: refused with decision 9's not-settled text.
fn refused(change: impl FnOnce(&mut Fixture)) {
    let mut fx = delivering();
    change(&mut fx);
    assert_eq!(iterated(fx), Err(NOT_SETTLED.to_string()));
}

/// An engine-made bisect fix task on stage 1, queued; its id.
fn fix(fx: &mut Fixture) -> String {
    let spec = fixes_spec(TaskOrigin::Bisect, "crates/fix/**", Default::default());
    let now = fx.now;
    add_fix(fx.run_mut(), spec, now, &mut Vec::new()).expect("added")
}

/// Decision 9 (D7): a complete `pr` run (every PR landed) starts a round; a cancelled
/// one, complete with its PRs left open, is refused with the brief's text.
#[test]
fn a_complete_pr_run_can_be_iterated_and_a_cancelled_one_cannot() {
    assert_eq!(iterated(landed()), started(2));
    let mut fx = complete();
    pr_mode(fx.run_mut());
    fx.run_mut().cancelled = true;
    assert_eq!(
        iterated(fx),
        Err("run 3f9a was cancelled; start a new goal for more work".into())
    );
}

/// KG §2.5: while stage 1's PR is open, round 2's stage 2 is created from stage 1's
/// head and its PR is based on stage 1's branch.
#[test]
fn a_round_stage_is_stacked_on_the_open_pr_below() {
    let mut fx = delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    // The widened run's stage 1 first, at the run head its PR holds.
    let ops = creating(&fx);
    assert_eq!(ops.len(), 1, "{ops:?}");
    fx.done(ops[0].0, crate::run::engine::OpResult::StageCreated);
    let s1 = fx.run().stage_head(1).unwrap().to_string();
    let ops = creating(&fx);
    assert_eq!(
        (ops[0].1.clone(), ops[0].2.clone()),
        (format!("anthrex/{RUN_ID}/stage-2"), s1)
    );
    let mut fx2 = delivering();
    assert_eq!(reply(&iterate(&mut fx2, "more")), started(2));
    let (open, _) = deliver_round_two(&mut fx2);
    let HostOp::OpenPr { stage, base, .. } = open else {
        unreachable!()
    };
    assert_eq!((stage, base), (2, format!("anthrex/{RUN_ID}/stage-1")));
}

/// KG §2.5: when every earlier PR has landed, the base fetch is due first and round 2's
/// stage 2 waits for it; the fetched base is in stage 2 before any of the round's work
/// merges there, and its PR is based on the base branch.
#[test]
fn when_every_pr_landed_the_round_starts_from_the_fetched_base() {
    let mut fx = landed();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    assert!(
        fx.run().delivery.base_fetch_due,
        "the fetch is made due first"
    );
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    // Stage 1 (the widened run's) is created; stage 2 waits for the base fetch.
    let ops = creating(&fx);
    fx.done(ops[0].0, crate::run::engine::OpResult::StageCreated);
    assert!(creating(&fx).is_empty(), "{:#?}", fx.run().pending_ops);
    let (_, fetch) = base_fetch(&fx);
    assert!(matches!(fetch, HostOp::Fetch { stage: None, .. }));
    let base = commit(71);
    fetched(&mut fx, &base, None);
    assert!(!fx.run().delivery.base_fetch_due);
    let ops = creating(&fx);
    assert_eq!(ops.len(), 1, "stage 2 is created once the base is fetched");
    fx.done(ops[0].0, crate::run::engine::OpResult::StageCreated);
    // The fetched base goes into stage 2 first (9.2's base sync).
    let (op, spec) = base_sync(&fx);
    assert_eq!((spec.to, spec.from_head.clone()), (2, base.clone()));
    fx.done(op, merged_at(&commit(72)));
    assert_eq!(
        fx.run().delivery.base_synced.as_deref(),
        Some(base.as_str())
    );
    let (open, _) = deliver_after_plan(&mut fx);
    let HostOp::OpenPr { stage, base, .. } = open else {
        unreachable!()
    };
    assert_eq!((stage, base.as_str()), (2, "main"));
}

/// Decision 12 in `pr` mode, and the task 4b review's widened run: a rejected round
/// returns the run to delivering with no stage record of its own; stage 1's PR is
/// still watched, and the run can be iterated again.
#[test]
fn rejecting_a_pr_round_returns_to_delivering() {
    let mut fx = delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    submit_round(&mut fx);
    assert_eq!(
        reply(&reject(&mut fx)),
        Ok("run 3f9a round 2 rejected; the earlier rounds are unchanged".into())
    );
    let run = fx.run();
    assert_eq!(run.state, RunState::Running);
    assert!(run.stages.is_empty(), "{:?}", run.stages);
    assert!(run.delivery.watching);
    assert_eq!(round_two_lines(&fx), vec![RoundOutcome::Rejected]);
    // The widened run's stage 1 is created at the head its PR holds; stage 2, all of
    // whose tasks were cancelled, is skipped.
    create_stages(&mut fx);
    fx.tick();
    let run = fx.run();
    assert_eq!(
        run.stage_head(1),
        Some(run.delivery.pr(1).unwrap().pushed_head.as_str())
    );
    assert!(run.delivery.delivering(stage_count(run)), "{:#?}", run.log);
    // Stage 1's PR is still polled, and its view is taken in.
    let now = fx.now;
    fx.run_mut().delivery.stages[0]
        .pr
        .as_mut()
        .unwrap()
        .next_poll_at = now;
    let head = fx.run().delivery.pr(1).unwrap().pushed_head.clone();
    poll_stage(&mut fx, 1, view_of(12, &head));
    let run = fx.run();
    assert_eq!(run.delivery.pr(1).unwrap().last_view_at, Some(fx.now));
    assert_eq!(run.state, RunState::Running, "{:#?}", run.log);
    assert_eq!(reply(&iterate(&mut fx, "again")), started(3));
    assert_eq!(fx.run().rounds[2].first_stage, 3);
}

/// Decision 12 for a `pr` run iterated after every PR landed: a reject returns it to
/// `complete`, not to running (it has no PR to deliver), with no second completion.
#[test]
fn rejecting_a_landed_pr_runs_round_returns_to_complete() {
    let mut fx = landed();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    submit_round(&mut fx);
    assert!(reply(&reject(&mut fx)).is_ok());
    let logged = fx.log.len();
    fx.tick();
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(wake_texts(&fx.log[logged..]).is_empty());
    assert!(ops_in(&fx.log[logged..], "VerifyRefs").is_empty());
}

/// Decision 17 in `pr` mode: once the round's tasks are finished and its stage has its
/// PR, the round ends (one `round` line) in the same step that asks for its summary,
/// and the run keeps delivering; a summary written then is the round's.
#[test]
fn a_pr_round_end_asks_for_its_summary() {
    let mut fx = delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    let (_, effects) = deliver_round_two(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Running);
    assert!(run.delivery.watching);
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Completed));
    assert_eq!(
        run.rounds[1].ended_at,
        Some(fx.now),
        "ended in the asking step"
    );
    assert!(!run.finish_edit);
    assert_eq!(round_two_lines(&fx), vec![RoundOutcome::Completed]);
    // The summary wake goes out in the step that ended the round, never before.
    let asked: Vec<usize> = (0..fx.log.len())
        .filter(|&i| {
            wake_texts(&fx.log[i..=i])
                .iter()
                .any(|t| t.contains(ROUND_DONE))
        })
        .collect();
    assert_eq!(asked.len(), 1, "{:#?}", wake_texts(&fx.log));
    assert!(
        wake_texts(&effects).iter().any(|t| t.contains(ROUND_DONE)),
        "{effects:#?}"
    );
    let effects = edit_plan(&mut fx, json!({"edits": [], "summary": "round two"}));
    assert!(answer(&effects).0, "{effects:#?}");
    let run = fx.run();
    assert_eq!(run.rounds[1].summary.as_deref(), Some("round two"));
    assert_eq!(run.rounds[0].summary, None);
    // A later pass ends nothing twice.
    fx.tick();
    assert_eq!(round_two_lines(&fx), vec![RoundOutcome::Completed]);
}

/// The task 4b review: once a `pr` round has ended (here by delivering its PR), `run
/// cancel` is the whole run's, and still cancels it.
#[test]
fn a_cancel_after_a_pr_round_ended_cancels_the_run() {
    let mut fx = delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    deliver_round_two(&mut fx);
    assert!(fx.run().rounds[1].ended_at.is_some());
    assert_eq!(
        reply(&cancel(&mut fx)),
        Ok(format!(
            "run {RUN_ID} cancelled; it completes once its sessions have ended"
        ))
    );
    let run = fx.run();
    assert!(run.cancelled);
    assert!(!run.delivery.watching);
    // The run completes, its two PRs left open; round 2 keeps its outcome and line.
    sessions_end(&mut fx);
    fx.tick();
    let (op, _) = fx.op("VerifyRefs");
    fx.done(op, crate::run::engine::OpResult::RefsOk);
    let run = fx.run();
    assert_eq!(run.state, RunState::Complete, "{:#?}", run.log);
    assert_eq!(
        run.outcome.as_deref(),
        Some("cancelled; 2 pull requests left open: #12, #13")
    );
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Completed));
    assert_eq!(round_two_lines(&fx), vec![RoundOutcome::Completed]);
}

/// Decision 16 in `pr` mode: a round's cancel keeps the earlier PRs watched and writes
/// no `stage` line; the round ends `cancelled` once its stage is skipped, and the run
/// keeps delivering.
#[test]
fn a_round_cancel_keeps_watching_earlier_prs() {
    let mut fx = delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(&mut fx);
    let logged = fx.log.len();
    assert_eq!(
        reply(&cancel(&mut fx)),
        Ok("run 3f9a round 2 cancelled; it ends once its sessions have ended".into())
    );
    let run = fx.run();
    assert!(run.delivery.watching);
    assert!(!run.cancelled);
    sessions_end(&mut fx);
    fx.tick();
    let run = fx.run();
    assert_eq!(run.state, RunState::Running);
    assert!(run.delivery.watching);
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Cancelled));
    assert_eq!(round_two_lines(&fx), vec![RoundOutcome::Cancelled]);
    assert!(stage_lines(&fx.log[logged..]).is_empty());
    assert!(run.delivery.delivering(stage_count(run)));
}

/// Decision 17's `pr` end waits for every task: an engine-made fix task queued on
/// stage 1 keeps round 2 open after its PR opened, until it finishes.
#[test]
fn a_pr_round_ends_only_once_every_task_finished() {
    let (mut fx, op) = opening();
    let id = fix(&mut fx);
    host_answer(&mut fx, op, opened(13, false));
    assert_eq!(fx.run().rounds[1].ended_at, None);
    fx.force(&id, TaskState::Cancelled);
    assert_eq!(fx.run().rounds[1].ended_at, Some(fx.now));
    assert_eq!(round_two_lines(&fx), vec![RoundOutcome::Completed]);
}

/// Decision 17's `pr` end waits for a propagate in flight (stage 1's head moved).
#[test]
fn a_pr_round_ends_only_once_no_propagate_is_in_flight() {
    let (mut fx, op) = opening();
    set_stage_head(fx.run_mut(), 1, &commit(80));
    fx.tick();
    let (prop, _) = pending_one(&fx, "Propagate", None);
    assert!(fx.run().propagate_due.is_empty());
    host_answer(&mut fx, op, opened(13, false));
    assert_eq!(fx.run().rounds[1].ended_at, None);
    fx.done(prop, merged_at(&commit(81)));
    assert_eq!(fx.run().rounds[1].ended_at, Some(fx.now));
}
