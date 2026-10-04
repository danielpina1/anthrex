//! Milestone 9.5 decision 15 (rulings RE-4, T12-1, T12-2): a run's paused time, which
//! the estimate leaves out of both its elapsed time and each task's work. Pure (design
//! decision 2).

use proto::{RunState, TaskState};

use crate::run::model::{Run, Task};

/// Whether time in `state` counts toward a task's active seconds (M8b decision 31's
/// preparing, working, proof, check, review and merge phases).
pub(crate) fn active(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Preparing
            | TaskState::Working
            | TaskState::Proof
            | TaskState::Check
            | TaskState::Review
            | TaskState::MergeQueue
    )
}

/// Whether a run in `state` is stopped: its time is not work.
fn stopped(state: RunState) -> bool {
    matches!(state, RunState::Paused | RunState::Halted)
}

/// A step changed `run` (it was `old`): entering `paused` or `halted` records when,
/// leaving them adds the span to `paused_secs`, and the step's time is recorded. A
/// task whose phase changed in the step moves the paused time of the active phase it
/// left into `paused.active`, and starts its new phase from the run's paused total.
pub(super) fn account(old: &Run, run: &mut Run, now: u64) {
    match (stopped(old.state), stopped(run.state)) {
        (false, true) => run.paused_at = Some(now),
        (true, false) => {
            let since = run.paused_at.take().unwrap_or(now);
            run.paused_secs = run.paused_secs.saturating_add(now.saturating_sub(since));
        }
        _ => {}
    }
    run.last_step_at = now;
    phases(old, run, now);
}

/// Each task of `run` whose phase changed since `old` moves the paused time of the
/// active phase it left into `paused.active`, and starts its new phase from the run's
/// paused total at `now`.
fn phases(old: &Run, run: &mut Run, now: u64) {
    let total = run.paused_total(now);
    for (i, task) in run.tasks.iter_mut().enumerate() {
        let before = (old.tasks.get(i))
            .filter(|t| t.id() == task.id())
            .or_else(|| old.tasks.iter().find(|t| t.id() == task.id()));
        phase_paused(before, task, total);
    }
}

fn phase_paused(before: Option<&Task>, task: &mut Task, total: u64) {
    if before.is_some_and(|b| b.phase_since == task.phase_since) {
        return;
    }
    if let Some(b) = before.filter(|b| b.phase_since > 0 && active(b.state)) {
        let inside = total.saturating_sub(b.paused.base);
        task.paused.active = task.paused.active.saturating_add(inside);
    }
    task.paused.base = total;
}

/// Ruling T12-1: a run that the restore leaves `paused` or `halted` with no pause
/// recorded (one the restart paused, or one stored before milestone 9.5) is paused
/// from its last change, so the daemon's downtime is not work; with none recorded,
/// from the restore.
///
/// Task 12's carry N1: a run new to the state is never `account`ed, so a phase the
/// restore changed (a replayed result) is re-based here against the run as it was
/// stored, `original`, once the downtime is paused time.
pub(super) fn restored(original: &Run, run: &mut Run, now: u64) {
    if stopped(run.state) && run.paused_at.is_none() {
        let last = (run.last_step_at > 0).then_some(run.last_step_at);
        run.paused_at = Some(last.unwrap_or(now));
    }
    phases(original, run, now);
}
