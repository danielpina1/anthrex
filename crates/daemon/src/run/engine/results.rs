//! Routes an op's result to the module that owns the op (moved out of `mod.rs`, whose
//! size limit milestone 9 would otherwise pass). Pure (design decision 2).

use super::{
    Effect, EngineState, OpId, OpKind, OpResult, complete, deciders, dispatch, done, early,
    fallback, gates, history, holds, kinds, ladder, merge, orch_window, outbox, planners, requests,
    review, run_scouts, stages, worker_messages,
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
    match (pending.kind, task) {
        (kind @ OpKind::Proof { .. }, Some(i)) => {
            gates::proof_done(run, i, op, &kind, result, now, fx)
        }
        (kind @ OpKind::Check { .. }, Some(i)) => {
            gates::check_done(run, i, op, &kind, result, now, fx)
        }
        (OpKind::Check { .. }, None) => complete::final_checked(run, result, now, fx),
        (OpKind::VerifyRefs { .. }, _) => complete::refs_verified(run, result, now, fx),
        (OpKind::MergeCandidate { run_branch, .. }, i) => {
            merge::candidate_done(run, (i, op), &run_branch, result, now, fx)
        }
        // Milestone 9.1 decision 48.
        (kind @ OpKind::CreateStageBranch { .. }, _) => stages::created(run, &kind, result, now),
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
