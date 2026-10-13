//! Milestone 9.15 decisions 28 to 30 (L3): the pre-review risk note. A reviewed task's
//! first gate queues one `risk_note` decider, written from its plan text alone; the
//! note never delays review (a queued one is passed over while a reviewer waits, and
//! dropped once the task's reviewer is emitted); a non-empty answer is kept on the task
//! for every later round. Pure (design decision 2): no `std::fs`, `std::process`,
//! `std::thread`, `tokio` or `std::time::SystemTime`.

use proto::{DeciderSource, TaskKind};

use super::schedule::needs_reviewer;
use crate::decider::risk_note::clean_note;
use crate::decider::{DeciderRequest, RiskNoteInput};
use crate::run::model::{QueuedDecider, ReviewLevel, Run};

/// Decision 28: task `i` has entered `proof` or `check`. Queues its one note when the
/// run froze risk notes on (ruling C2: a run without frozen lessons never does), the
/// task is reviewed at `medium` or `frontier`, its kind is neither `research` nor
/// `review`, and it was not asked before.
pub(crate) fn queue_once(run: &mut Run, i: usize, now: u64) {
    let on = run.lessons.as_ref().is_some_and(|l| l.limits.risk_note);
    let task = &run.tasks[i];
    let level = matches!(
        task.review_level,
        Some(ReviewLevel::Medium | ReviewLevel::Frontier)
    );
    let kind = !matches!(task.spec.kind, TaskKind::Research | TaskKind::Review);
    if !on || !level || !kind || task.risk_note_asked {
        return;
    }
    let spec = &task.spec;
    let input = RiskNoteInput::of(
        &spec.id,
        &spec.title,
        &spec.brief,
        &spec.acceptance,
        &spec.owns,
    );
    let id = spec.id.clone();
    run.tasks[i].risk_note_asked = true;
    super::deciders::queue(run, vec![id], DeciderRequest::RiskNote(input), now);
}

/// Decision 29 (ruling C12): whether queued decider `q` is passed over by this pass's
/// dispatch: a risk note, while any task waits for its reviewer.
pub(super) fn passed_over(run: &Run, q: &QueuedDecider) -> bool {
    matches!(q.request, DeciderRequest::RiskNote(_))
        && run.tasks.iter().any(|t| needs_reviewer(run, t))
}

/// Decision 30: a decider's non-empty note for task `i`, cleaned and clipped, is kept
/// for its reviewers. A fallback, or a note that cleans to nothing, changes nothing.
pub(super) fn noted(run: &mut Run, i: usize, note: &str, source: DeciderSource) {
    let note = clean_note(note);
    if source == DeciderSource::Decider && !note.is_empty() {
        run.tasks[i].risk_note = Some(note);
    }
}
