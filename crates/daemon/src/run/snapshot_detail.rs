//! Milestone 9.0.5: what the snapshot and the task detail request derive from a task's
//! rounds: the live round's activity line (decision 3). Pure, like `snapshot.rs`:
//! everything here reads memory the caller already holds.

use crate::run::model::Task;

/// The activity of the task's live round (the latest one not ended), `None` otherwise,
/// so a terminal run's tasks carry nothing new (decision 2).
pub(crate) fn live_activity(task: &Task) -> Option<String> {
    task.rounds
        .iter()
        .rev()
        .find(|r| !r.ended)
        .and_then(|r| r.activity.clone())
}

/// `text` cut to at most `max` characters, on a character boundary, with `…` after the
/// kept part when anything was cut.
pub(crate) fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_string(),
        Some((end, _)) => format!("{}…", &text[..end]),
    }
}
