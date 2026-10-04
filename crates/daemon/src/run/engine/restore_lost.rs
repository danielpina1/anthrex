//! Decision 44, engine side: an op the restore dropped as `NotStarted`, and what its task
//! needs instead (split out of `restore.rs` for AGENTS.md rule 8, move-only). Pure
//! (design decision 2).

use proto::{AgentRole, TaskState};

use super::requests::log;
use super::{Effect, OpId, OpKind, emit_op, next_op};
use crate::run::model::{PendingOp, Run};
use crate::run::orch::RefreshState;

/// Decision 44: an op dropped as `NotStarted`, and what its task needs instead.
pub(super) fn lost(run: &mut Run, pending: PendingOp, now: u64, fx: &mut Vec<Effect>) {
    let PendingOp {
        op,
        task_id,
        kind,
        lane,
    } = pending;
    let i = task_id
        .as_deref()
        .and_then(|id| run.tasks.iter().position(|t| t.id() == id));
    match (&kind, i) {
        // Idempotent git work that starts no session: sent again under a new id.
        (
            OpKind::CreateRunBranch { .. }
            | OpKind::PrepareWorktree { .. }
            | OpKind::AbortMerge { .. }
            | OpKind::RemoveWorktree { .. }
            | OpKind::CrownRacer { .. }
            | OpKind::MeasureDiff { .. }
            | OpKind::AppendHistory { .. },
            _,
        ) => {
            let again = next_op(run);
            emit_op(run, again, task_id.as_deref(), kind, fx);
            // Review m1: a race lane's op (its crown, its removal) stays the lane's.
            if let Some(pending) = run.pending_ops.get_mut(&again) {
                pending.lane = lane;
            }
        }
        // A session starts only while the run runs (ruling T14-I2): `relaunch`.
        (OpKind::CreateWindow { .. }, Some(i)) => {
            if let Some(round) = run.tasks[i].rounds.iter_mut().find(|r| r.launch_op == op) {
                round.relaunch = Some(Box::new(kind));
                // Ruling T17b-3 (N1): the old daemon may have started this racer, so
                // its lane is never cleaned as if it had exited.
                round.orphaned = round.role == AgentRole::Racer;
            }
        }
        (OpKind::MergeCandidate { .. }, Some(i)) if run.tasks[i].merge_op == Some(op) => {
            run.tasks[i].merge_op = None;
        }
        // Milestone 9.1 decisions 50 and 51: a propagate is due again; a sync task's
        // hand-back is sent again when the run runs.
        (OpKind::Propagate(spec), _) => super::propagate::lost(run, spec),
        (OpKind::HandBack { task_head, .. }, Some(i))
            if super::propagate::sync_due(&run.tasks[i]) =>
        {
            super::propagate::hand_back_lost(run, i, task_head.clone());
        }
        // Milestone 9 decision 42e: a lost refresh is due again at the next boundary.
        (OpKind::HandBack { .. }, Some(i))
            if run.tasks[i].orch.refresh == Some(RefreshState::InFlight(op)) =>
        {
            run.tasks[i].orch.refresh = Some(RefreshState::Due);
        }
        // Milestone 9.5 ruling RR-4: so is a race lane's.
        (OpKind::HandBack { .. }, Some(i)) if lane_refresh(&mut run.tasks[i], op).is_some() => {
            if let Some(refresh) = lane_refresh(&mut run.tasks[i], op) {
                *refresh = Some(RefreshState::Due);
            }
        }
        // Carry T14-R2: a lost merge-queue hand-back is sent again by the running pass.
        (OpKind::HandBack { .. }, Some(i)) if run.tasks[i].merge_op == Some(op) => {
            let task = &mut run.tasks[i];
            task.merge_op = None;
            task.handback_due = task.state == TaskState::MergeQueue;
        }
        // Carry T13: `start_gates` and `dispatch_reviewers` re-issue a gate's op.
        // Milestone 9.1 decision 29: tier 1 is a check gate's op too. Ruling FW-2 (e):
        // a review task's target is resolved again (`kinds::dispatch`).
        (
            OpKind::Proof { .. }
            | OpKind::Check { .. }
            | OpKind::PrepareReview { .. }
            | OpKind::Tier(_)
            | OpKind::ResolveTarget { .. },
            Some(i),
        ) if run.tasks[i].gate_op == Some(op) => {
            run.tasks[i].gate_op = None;
        }
        // The final fix wave's A-I1 (a): a race lane's gate op is the lane's.
        (
            OpKind::Proof { .. }
            | OpKind::Check { .. }
            | OpKind::PrepareReview { .. }
            | OpKind::Tier(_),
            Some(i),
        ) if lane_gate_op(&mut run.tasks[i], op).is_some() => {
            if let Some(gate_op) = lane_gate_op(&mut run.tasks[i], op) {
                *gate_op = None;
            }
        }
        // Milestone 9.1 decision 29: a lost tier-3 job is started again by the next
        // idle or completion pass.
        (OpKind::Tier(_), None) if run.full_op == Some(op) => run.full_op = None,
        // Decision 29 (task M9.1.15): a lost bisect probe is issued again by the next
        // running pass (`bisect::pass`).
        (OpKind::TestAt(_), None) => {
            super::bisect::lost(run, op);
        }
        // M8b decision 18: a dropped decider is queued again under its own id, so the
        // task waiting for it still names it.
        (
            OpKind::Decide {
                decider_id,
                task_ids,
                request,
            },
            _,
        ) => run.decider_queue.push(crate::run::model::QueuedDecider {
            decider_id: *decider_id,
            task_ids: task_ids.clone(),
            request: request.clone(),
            queued_at: now,
        }),
        (OpKind::CreateOrchestrator { .. }, _) => super::orch_window::launch_lost(run, op),
        (OpKind::Accept { .. } | OpKind::Discard { .. }, _) => {
            let text = "an accept or discard did not finish before the restart; request it again";
            log(run, now, text);
        }
        // `VerifyDone`, `CountCommits` and `ResumeSession` had their waiters cleared;
        // `DiffSoFar`, `VerifyRefs`, the final check and an N5 hand-back are sent again
        // by the scheduler, which sees none in flight.
        _ => {}
    }
}

/// The gate op of the race lane whose gate op `op` is.
fn lane_gate_op(task: &mut crate::run::model::Task, op: OpId) -> Option<&mut Option<OpId>> {
    let lanes = task.race.iter_mut().flat_map(|r| r.lanes.iter_mut());
    let mut ops = lanes.map(|l| &mut l.gates.gate_op);
    ops.find(|o| **o == Some(op))
}

/// The refresh of the race lane whose refresh `op` is.
fn lane_refresh(task: &mut crate::run::model::Task, op: OpId) -> Option<&mut Option<RefreshState>> {
    let lanes = task.race.iter_mut().flat_map(|r| r.lanes.iter_mut());
    let mut refreshes = lanes.map(|l| &mut l.gates.refresh);
    refreshes.find(|r| **r == Some(RefreshState::InFlight(op)))
}
