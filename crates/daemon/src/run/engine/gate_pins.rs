//! Milestone 9.13 decisions 11 and 13: which of the run's gate scripts stay pinned to
//! the base commit, and the halt that follows a gate command the executor refused
//! because one of them changed (`OpResult::GateRefused`). Pure (design decision 2).

use proto::RunState;

use super::{Effect, OpId, OpKind, bisect, complete, dispatch, gate_holds, merge};
use crate::run::gate_words::PinnedFile;
use crate::run::globs::names_literally;
use crate::run::model::Run;

/// Decision 13's text; `{path}` is the refused script.
pub(crate) const GATE_CHANGED: &str =
    "the gate's script {path} differs from the base commit's; the run changed what measures it";

/// [`GATE_CHANGED`] for `path`.
pub(crate) fn gate_changed(path: &str) -> String {
    GATE_CHANGED.replace("{path}", path)
}

/// Decision 11: the run's gate files, less every file that the `owns` of a task that is
/// not cancelled and is released past the plan gate (`gate_holds::released_past_gate`,
/// ruling C18) names literally. A grant in a plan or a hold the user has not approved
/// leaves the file pinned.
pub(crate) fn pinned(run: &Run) -> Vec<PinnedFile> {
    let granted = |file: &PinnedFile| {
        run.tasks.iter().any(|task| {
            task.state != proto::TaskState::Cancelled
                && gate_holds::released_past_gate(run, task)
                && names_literally(&task.spec.owns, &file.path)
        })
    };
    run.gate_files
        .iter()
        .filter(|file| !granted(file))
        .cloned()
        .collect()
}

/// Decision 13: op `op` of `kind` (task `task`, if any) was refused because `path`
/// changed. What the op's kind was waiting for is cleared first (ruling C6), so the
/// normal paths re-issue or settle that work after the user's `run resume`; then the
/// task, if it is not finished, is blocked for the user, and a running or paused run
/// halts for the user. The halt is retryable: a plain `run resume` (the user's only)
/// resumes it, since nothing moved under the run's refs.
pub(super) fn refused(
    run: &mut Run,
    (op, kind, task): (OpId, &OpKind, Option<usize>),
    path: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let text = gate_changed(path);
    let task = clear_waiting(run, (op, kind, task), now, fx);
    if let Some(i) = task.filter(|&i| !run.tasks[i].state.is_finished()) {
        dispatch::block_user_only(run, i, text.clone(), now);
    }
    if matches!(run.state, RunState::Running | RunState::Paused) {
        merge::halt(run, text, now);
        run.halt_retryable = true;
    }
}

/// Ruling C6: clears what the refused op's kind left waiting; the task, unless a
/// deferred cancel took it instead.
fn clear_waiting(
    run: &mut Run,
    (op, kind, task): (OpId, &OpKind, Option<usize>),
    now: u64,
    fx: &mut Vec<Effect>,
) -> Option<usize> {
    match (kind, task) {
        // A merge candidate's check: the merge did not land.
        (OpKind::MergeCandidate { .. }, Some(i)) if run.tasks[i].merge_op == Some(op) => {
            run.tasks[i].merge_op = None;
            let id = run.tasks[i].id().to_string();
            run.merge_queue.retain(|q| *q != id);
            if std::mem::take(&mut run.tasks[i].cancel_deferred) {
                let why = "its cancel, after its merge did not land";
                complete::cancel_now(run, i, why, now, fx);
            }
        }
        // A propagate or base sync: due again (task 6 review I-1), since emitting it took
        // its due entry.
        (OpKind::Propagate(spec), None) => super::propagate::lost(run, spec),
        // A gate's op (the check, the proof, tier 1).
        (_, Some(i)) if run.tasks[i].gate_op == Some(op) => run.tasks[i].gate_op = None,
        // The final check holds nothing: completion issues it again. A tier-3 job, a
        // bisect probe and a CI reproduction each hold `full_op`.
        (_, None) => {
            bisect::lost(run, op);
            for record in run.delivery.stages.iter_mut().flat_map(|s| s.ci.iter_mut()) {
                if record.probe == Some(op) {
                    record.probe = None;
                }
            }
            if run.full_op == Some(op) {
                run.full_op = None;
            }
        }
        _ => {}
    }
    task
}
