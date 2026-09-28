//! Milestone 9's part of the pushed snapshot (task M9.6): decision 16a's plan text
//! only at the gate, and decision 42d's message counts and bounded task notes. Pure.

use proto::TaskNoteInfo;

use super::model::{Run, Task};

/// Task notes a task shows (decision 42d), and its latest message's line.
const TASK_NOTES_SHOWN: usize = 10;
const MESSAGE_LINE_MAX: usize = 80;
/// A task note's text in the snapshot, in characters (every push carries them).
pub const SNAPSHOT_NOTE_MAX: usize = 400;

/// Decision 16a: a task's brief, acceptance and route spec are published only while a
/// client can edit them: the run awaits approval, or the task's approval hold does.
pub(super) fn plan_text_shown(run: &Run, task: &Task) -> bool {
    run.state == proto::RunState::AwaitingApproval
        || task.orch.gate_hold.as_ref().is_some_and(|id| {
            run.orch
                .gate_holds
                .iter()
                .any(|h| &h.id == id && h.state == proto::HoldState::Awaiting)
        })
}

/// Decision 42d: the latest message's first line, control characters dropped, at
/// most 80 characters.
pub(super) fn message_line(task: &Task) -> Option<String> {
    let text = &task.orch.messages.last()?.text;
    let first = text.lines().next().unwrap_or_default();
    Some(
        first
            .chars()
            .filter(|c| !c.is_control())
            .take(MESSAGE_LINE_MAX)
            .collect(),
    )
}

/// The task's last [`TASK_NOTES_SHOWN`] notes, oldest first, each text cut to
/// [`SNAPSHOT_NOTE_MAX`] characters.
pub(super) fn task_notes(task: &Task) -> Vec<TaskNoteInfo> {
    let notes = &task.orch.worker_notes;
    notes[notes.len().saturating_sub(TASK_NOTES_SHOWN)..]
        .iter()
        .map(|n| TaskNoteInfo {
            task_id: task.spec.id.clone(),
            kind: n.kind,
            text: crate::run::orch::json::cut(&n.text, SNAPSHOT_NOTE_MAX),
            at: n.at,
        })
        .collect()
}
