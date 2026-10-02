//! Routes an op's result to the module that owns the op (moved out of `mod.rs`, whose
//! size limit milestone 9 would otherwise pass). Pure (design decision 2).

use super::{
    Effect, EngineState, OpId, OpKind, OpResult, bisect, complete, deciders, delivery, dispatch,
    done, early, fallback, full, gates, history, holds, kinds, ladder, merge, orch_window, outbox,
    planners, propagate, requests, review, run_scouts, stages, tiers, worker_messages,
};

/// Routes an op's result by the kind of the op it answers. A result for an op the run
/// no longer has pending (stale, or replayed twice) is ignored.
pub(super) fn op_done(
    state: &mut EngineState,
    run_id: &str,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(run) = state.runs.get_mut(run_id) else {
        return;
    };
    let Some(pending) = run.pending_ops.remove(&op) else {
        return;
    };
    let task = pending
        .task_id
        .as_deref()
        .and_then(|id| run.tasks.iter().position(|t| t.id() == id));
    let mut bound = None;
    // Controller ruling C-21 (3): a later hand-back into a sync task is recorded.
    if let (OpKind::HandBack { run_head, .. }, Some(i)) = (&pending.kind, task) {
        propagate::record_hand_back(run, i, run_head, &result);
    }
    match (pending.kind, task) {
        (kind @ OpKind::Proof { .. }, Some(i)) => {
            gates::proof_done(run, i, op, &kind, result, now, fx)
        }
        (kind @ OpKind::Check { .. }, Some(i)) => {
            gates::check_done(run, i, op, &kind, result, now, fx)
        }
        (OpKind::Check { .. }, None) => complete::final_checked(run, result, now, fx),
        // Milestone 9.1 decision 14: tier 1 is the task's check gate.
        (OpKind::Tier(spec), Some(i)) if spec.tier == 1 => {
            tiers::tier1_done(run, i, op, result, now, fx)
        }
        // Decisions 17-19: tier 3, a run-level job.
        (OpKind::Tier(spec), None) if spec.tier == 3 => {
            full::full_done(run, op, &spec, result, now, fx)
        }
        // Decision 36: a bisect probe (task M9.1.15).
        (OpKind::TestAt(_), None) => bisect::probe_done(run, op, result, now, fx),
        (OpKind::VerifyRefs { .. }, _) => complete::refs_verified(run, result, now, fx),
        (OpKind::MergeCandidate { run_branch, .. }, i) => {
            merge::candidate_done(run, (i, op), &run_branch, result, now, fx)
        }
        // Milestone 9.1 decision 48.
        (kind @ OpKind::CreateStageBranch { .. }, _) => stages::created(run, &kind, result, now),
        // Decisions 50-52.
        (OpKind::Propagate(spec), _) => propagate::done(run, (op, &spec), result, now, fx),
        // Decision 51: a sync task's merge, before its first session.
        (OpKind::HandBack { task_head, .. }, Some(i)) if propagate::sync_due(&run.tasks[i]) => {
            propagate::handed_back(run, i, task_head, result, now, fx)
        }
        (OpKind::CreateRunBranch { .. }, _) => requests::run_branch_done(run, result, now, fx),
        (kind @ (OpKind::Discard { .. } | OpKind::Accept { .. }), _) => {
            complete::finished(run, &kind, result, now, fx)
        }
        (OpKind::PrepareWorktree { from, .. }, Some(i)) => {
            dispatch::worktree_done(run, i, from, result, now, fx)
        }
        (OpKind::CreateWindow { .. }, Some(i)) => {
            if let OpResult::Window { window_id, .. } = result {
                bound = Some(window_id);
            }
            dispatch::window_done(run, i, op, result, now, fx)
        }
        (OpKind::PrepareReview { .. }, Some(i)) => {
            review::review_ready(run, i, op, result, now, fx)
        }
        // Milestone 9 decision 36: a review task's target.
        (OpKind::ResolveTarget { .. }, Some(i)) => kinds::target_done(run, i, op, result, now, fx),
        // Milestone 9 decision 42e: a refresh's hand-back, ahead of M8a's.
        (OpKind::HandBack { run_head, .. }, Some(i))
            if worker_messages::awaits_refresh(run, i, op) =>
        {
            worker_messages::refreshed(run, i, (&run_head, result), now)
        }
        (OpKind::HandBack { run_head, .. }, Some(i)) if merge::awaits(run, i, op) => {
            merge::handed_back(run, i, op, &run_head, result, now, fx)
        }
        (OpKind::HandBack { .. }, Some(i)) => holds::handed_back(run, i, result, now, fx),
        (OpKind::AbortMerge { .. }, Some(i)) => holds::merge_aborted(run, i, result, now),
        (OpKind::RemoveWorktree { path, .. }, Some(i)) => {
            dispatch::removed(run, i, &path, result, now)
        }
        (OpKind::VerifyDone { .. }, Some(i)) => done::checked(run, i, op, result, now, fx),
        (OpKind::CountCommits { .. }, Some(i)) if gates::awaits_override(run, i, op) => {
            gates::override_counted(run, i, result, now, fx)
        }
        (OpKind::CountCommits { .. }, Some(i)) => fallback::counted(run, i, op, result, now, fx),
        (OpKind::DiffSoFar { .. }, Some(i)) => ladder::fresh_diff(run, i, result, now, fx),
        (OpKind::ResumeSession { .. }, Some(i)) => outbox::resumed(run, i, op, result, now, fx),
        (kind @ OpKind::Decide { .. }, _) => deciders::op_done(run, &kind, result, now, fx),
        (OpKind::MeasureDiff { .. }, Some(i)) => history::measured(run, i, result, now, fx),
        (kind @ OpKind::AppendHistory { .. }, _) => history::appended(run, &kind, result, now),
        // Milestone 9 decisions 5 and 11: the orchestrator's window.
        (OpKind::CreateOrchestrator { .. }, _) => orch_window::launched(run, op, result, now, fx),
        (OpKind::RestartOrchestrator { .. }, _) => orch_window::restarted(run, result, now, fx),
        // Decisions 20 and 32: a run scout's and a sub-planner's session.
        (kind @ OpKind::StartScout { .. }, _) => run_scouts::started(run, &kind, result, now, fx),
        // Milestone 9.2 decision 8: a host call's answer.
        (OpKind::Host { op: host, .. }, _) => delivery::host_done(run, host, result, now, fx),
        (kind @ OpKind::StartPlanner { .. }, _) => {
            bound = planners::started(run, &kind, result, now, fx);
        }
        _ => {}
    }
    // The session's events that came before its window, now that its round has it.
    if let Some(window_id) = bound {
        early::replay(state, window_id, fx);
    }
}
