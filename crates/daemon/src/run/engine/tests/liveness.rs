//! The liveness check every engine test sequence ends with (ruling T12-I4, extended by
//! M8a.13 to the gate states, by M8a.14 to the run's own states, and by M8a.15 to
//! paused and restored runs): nothing is ever left waiting for nothing. Moved out of
//! `turns_fixes.rs` for size.

use proto::{AgentRole, RunState, TaskState};

use super::fixture::{Fixture, RUN_ID};
use crate::run::engine::{Event, EventKind, OpKind, step};
use crate::run::model::{FailedTurn, Run, StallState};

/// Ruling T15-R2: no round is excused more time than it has lasted, so no spend is
/// negative and no excused time is banked as credit.
pub(super) fn assert_clock_sound(run: &Run, now: u64) {
    for task in &run.tasks {
        for (r, round) in task.rounds.iter().enumerate() {
            let elapsed = round
                .ended_at
                .unwrap_or(now)
                .saturating_sub(round.started_at);
            assert!(
                round.excused_secs <= elapsed,
                "{} round {r}: excused {} of {elapsed} elapsed",
                task.id(),
                round.excused_secs
            );
        }
    }
}

/// The whole check on the fixture's run. A paused run (by the `pause` edit or by a
/// restore) waits for the user, by design; it is alive when resuming it leaves nothing
/// stuck (M8a.15): the check runs on the state a `run resume` one second later
/// produces.
pub(super) fn assert_alive(fx: &Fixture) {
    assert_clock_sound(fx.run(), fx.now);
    if fx.run().state != RunState::Paused {
        run_alive(fx.run());
    } else {
        let resume = Event {
            now: fx.now + 1,
            kind: EventKind::Resume {
                reply: 0,
                run_id: RUN_ID.into(),
                rebaseline: None,
            },
        };
        let (state, _) = step(fx.state.clone(), resume);
        let run = &state.runs[RUN_ID];
        assert_eq!(run.state, RunState::Running, "a paused run resumes");
        run_alive(run);
    }
}

/// M8a.13's gate part alone.
pub(super) fn assert_gates_alive(fx: &Fixture) {
    gates_alive(fx.run());
}

fn run_alive(run: &Run) {
    assert_working_alive(run);
    gates_alive(run);
    assert_run_alive(run);
}

/// Ruling T12-I4's invariant, for every working task of `run`.
fn assert_working_alive(run: &Run) {
    for t in run.tasks.iter().filter(|t| t.state == TaskState::Working) {
        let round = t.rounds.iter().rev().find(|r| r.role == AgentRole::Worker);
        let live =
            |r: &&crate::run::model::AgentRound| r.window_id.is_some() && !r.ended && !r.retiring;
        let open = round.is_some_and(|r| live(&r) && r.turn_open);
        let deliverable = t.fresh_session.is_some()
            || round.is_some_and(|r| {
                (live(&r) && !matches!(r.stall, StallState::Interrupted { .. }))
                    || (r.ended && !r.retiring && r.session_id.is_some())
            });
        let queued = run
            .outbox
            .iter()
            .any(|m| m.task_id == t.spec.id && (m.delivered_at.is_some() || deliverable));
        let timer = round.is_some_and(|r| {
            matches!(r.failed_turn, FailedTurn::WaitingContinue { .. })
                || r.delivery_retry_at.is_some()
                || r.count_retry_at.is_some()
        });
        let op = run
            .pending_ops
            .values()
            .any(|p| p.task_id.as_deref() == Some(t.id()));
        let killing = t.rounds.iter().any(|r| r.retiring && !r.ended);
        assert!(
            open || queued || timer || op || killing,
            "{} is working with nothing pending: {:#?}",
            t.spec.id,
            t
        );
    }
}

/// M8a.14's extension to the run's own states: a running run whose tasks are all
/// merged or cancelled has an op in flight (its clean-up, `VerifyRefs` or the final
/// check) or a killed session still to exit; a running run with a merge queue has a
/// `MergeCandidate` in flight; a halted run says why.
fn assert_run_alive(run: &Run) {
    match run.state {
        proto::RunState::Running => {
            let done = run
                .tasks
                .iter()
                .all(|t| matches!(t.state, TaskState::Merged | TaskState::Cancelled));
            // A killed session's exit brings its task's clean-up, then completion.
            let ending = run
                .tasks
                .iter()
                .any(|t| t.state == TaskState::Cancelled && t.rounds.iter().any(|r| !r.ended));
            assert!(
                !done || !run.pending_ops.is_empty() || ending,
                "every task is finished and nothing is pending: {:#?}",
                run.tasks
                    .iter()
                    .map(|t| (t.id(), t.state))
                    .collect::<Vec<_>>()
            );
            let merging = run
                .pending_ops
                .values()
                .any(|p| matches!(p.kind, OpKind::MergeCandidate { .. }));
            assert!(
                run.merge_queue.is_empty() || merging,
                "a merge queue {:?} with no merge in flight",
                run.merge_queue
            );
        }
        proto::RunState::Halted => assert!(run.halted_reason.is_some(), "halted with no reason"),
        _ => {}
    }
}

/// M8a.13's extension to the gate states: a task in `proof` or `check` has its gate op
/// in flight; one in `review` has its `PrepareReview` or reviewer launch in flight, a
/// live watched reviewer turn, a reviewer message deliverable or in flight, a resume in
/// flight, or waits for a reader slot; one in the merge queue is queued (M8a.14 runs
/// it).
fn gates_alive(run: &Run) {
    let gated = [
        TaskState::Proof,
        TaskState::Check,
        TaskState::Review,
        TaskState::MergeQueue,
    ];
    for t in run.tasks.iter().filter(|t| gated.contains(&t.state)) {
        let op = run
            .pending_ops
            .values()
            .any(|p| p.task_id.as_deref() == Some(t.id()));
        let alive = match t.state {
            TaskState::Proof | TaskState::Check => op,
            // M8a.14: queued (the run-level check wants a merge in flight while the run
            // runs), or its candidate or hand-back in flight.
            // Ruling T14-R2 (N3): a due hand-back waits for a halted or paused run.
            TaskState::MergeQueue => {
                op || run.merge_queue.iter().any(|q| q == t.id())
                    || (t.handback_due
                        && matches!(run.state, proto::RunState::Halted | proto::RunState::Paused))
            }
            _ => {
                let reviewer = t
                    .rounds
                    .iter()
                    .rev()
                    .find(|r| r.role == AgentRole::Reviewer);
                let live = |r: &&crate::run::model::AgentRound| {
                    r.window_id.is_some() && !r.ended && !r.retiring
                };
                let watched = reviewer.is_some_and(|r| live(&r) && r.turn_open);
                let resumable = reviewer.is_some_and(|r| {
                    live(&r) || (r.ended && !r.retiring && r.session_id.is_some())
                });
                let address = format!("{}.review", t.id());
                let mail = run
                    .outbox
                    .iter()
                    .any(|m| m.task_id == address && (m.delivered_at.is_some() || resumable));
                // Ruling T13-I1: a failed turn's wait; T13-I2: a given-up reviewer's
                // exit; m6: every dispatch waits while a hub holds a writer slot.
                let timer = reviewer.is_some_and(|r| {
                    matches!(r.failed_turn, FailedTurn::WaitingContinue { .. })
                        || r.delivery_retry_at.is_some()
                });
                let killing = reviewer.is_some_and(|r| r.retiring && !r.ended);
                let slot_wait = reviewer.is_none_or(|r| r.retiring || r.ended)
                    && (super::super::schedule::readers_busy(run)
                        >= usize::from(run.limits.max_readers)
                        || super::super::schedule::hub_holds_slot(run));
                // Ruling T14-I2: a halted or paused run starts no reviewer; the task
                // waits for the user's resume.
                let stopped =
                    matches!(run.state, proto::RunState::Halted | proto::RunState::Paused);
                op || watched || mail || timer || killing || slot_wait || stopped
            }
        };
        assert!(
            alive,
            "{} is in {:?} with nothing pending: {:#?}",
            t.spec.id, t.state, t
        );
    }
}
