//! Milestone 9's rows in the run view's inspections (task M9.15): the run's
//! orchestrator (decision 11) and a task's messages and notes (decision 42i). Every
//! text an agent wrote goes through `clean`. Pure, as `run.rs` is.

use super::run_format::{clean, local_hhmm};
use crate::app::App;
use proto::{MessageKind, OrchestratorInfo, TaskInfo, TaskNoteKind};

/// Milestone 9 decision 11: the orchestrator's route, window, wakes and plan.
pub(super) fn orchestrator_text(orchestrator: &OrchestratorInfo) -> String {
    let route = &orchestrator.route;
    let mut text = route.runtime.label().to_owned();
    if !route.model.is_empty() {
        text.push_str(&format!(" {}", clean(&route.model)));
    }
    match orchestrator.window_id {
        Some(id) => text.push_str(&format!(" · window #{id}")),
        None => text.push_str(" · no window yet"),
    }
    text.push_str(if orchestrator.live {
        " · live"
    } else {
        " · exited"
    });
    match orchestrator.wakes {
        0 => text.push_str(" · no wakes"),
        1 => text.push_str(" · 1 wake"),
        n => text.push_str(&format!(" · {n} wakes")),
    }
    text.push_str(if orchestrator.plan_submitted {
        " · plan submitted"
    } else {
        " · plan not submitted"
    });
    text
}

/// Decision 42i's `messages` row: the count, and the latest message's kind and first
/// line; `None` before the first message.
pub(super) fn messages_text(task: &TaskInfo) -> Option<String> {
    if task.message_count == 0 {
        return None;
    }
    let mut text = task.message_count.to_string();
    if let Some(kind) = task.last_message_kind {
        let kind = match kind {
            MessageKind::Info => "info",
            MessageKind::Change => "change",
            MessageKind::StopAndWait => "stop and wait",
        };
        text.push_str(&format!(" · latest {kind}"));
        if let Some(line) = &task.last_message_line {
            text.push_str(&format!(": \"{}\"", clean(line)));
        }
    }
    Some(text)
}

/// Decision 42i: the task's `discovery` and `risk` notes, newest first, each with its
/// local time and the task it came from; `progress` notes are the history's.
pub(super) fn notes_text(task: &TaskInfo, app: &App) -> Option<String> {
    let notes: Vec<String> = (task.task_notes.iter().rev())
        .filter(|note| note.kind != TaskNoteKind::Progress)
        .map(|note| {
            let kind = match note.kind {
                TaskNoteKind::Discovery => "discovery",
                TaskNoteKind::Risk => "risk",
                TaskNoteKind::Progress => "progress",
            };
            format!(
                "{} {kind} from {}: {}",
                local_hhmm(note.at, app.utc_offset_secs),
                clean(&note.task_id),
                clean(&note.text)
            )
        })
        .collect();
    (!notes.is_empty()).then(|| notes.join(" · "))
}
