//! M8b decision 31: a task's time in each state. Every assignment of a task's state
//! outside tests goes through [`set_state`]. Pure (M8b decision 1).

use proto::{PhaseSecs, TaskState};

use super::model::Task;

/// The phase a state's time counts toward; `pending` and the finished states count
/// nowhere.
pub fn phase_mut(phases: &mut PhaseSecs, state: TaskState) -> Option<&mut u64> {
    Some(match state {
        TaskState::Queued => &mut phases.queued,
        TaskState::Preparing => &mut phases.preparing,
        TaskState::Working => &mut phases.working,
        TaskState::Proof => &mut phases.proof,
        TaskState::Check => &mut phases.check,
        TaskState::Review => &mut phases.review,
        TaskState::MergeQueue => &mut phases.merge,
        TaskState::Blocked => &mut phases.blocked,
        TaskState::Pending | TaskState::Merged | TaskState::Cancelled | TaskState::Reported => {
            return None;
        }
    })
}

/// Whether `task`'s test writer is at work (milestone 9.5 decision 25): its rung is
/// the writer's.
pub fn writing(task: &Task) -> bool {
    (task.pair.as_ref()).is_some_and(|p| p.phase == proto::PairPhase::Writing)
}

/// Moves `task` to `state` at `now`: the time since `phase_since` is added to the
/// phase of the state it leaves, `phase_since` becomes `now`, and `max_rung` takes the
/// task's rung (not a test writer's). Setting the state it is already in only tracks the rung, so a pass
/// that re-asserts a state changes nothing. A `phase_since` of 0 (a task from before
/// milestone 8b) counts nothing.
pub fn set_state(task: &mut Task, state: TaskState, now: u64) {
    // The final fix wave (review B's M2): a test writer's rung is the writer's own
    // escalation, never the task's class route's.
    if !writing(task) {
        task.max_rung = task.max_rung.max(task.rung);
    }
    if task.state == state {
        return;
    }
    let since = task.phase_since;
    if since > 0
        && let Some(phase) = phase_mut(&mut task.phases, task.state)
    {
        *phase = phase.saturating_add(now.saturating_sub(since));
    }
    task.phase_since = now;
    task.state = state;
    // Milestone 9.5 ruling T17a-4: the race decision belongs to one queued stint; a
    // task back to `pending` (it gained a dependency, or a cancelled one was replaced)
    // decides again when it is queued again.
    if state == TaskState::Pending {
        task.race_decision = None;
        task.race_wait_since = None;
    }
}
