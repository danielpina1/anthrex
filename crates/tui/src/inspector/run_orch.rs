//! Milestone 9's rows in the run view's inspections (task M9.15): the run's
//! orchestrator (decision 11) and a task's messages and notes (decision 42i). Every
//! text an agent wrote goes through `clean`. Pure, as `run.rs` is.

use super::run_format::clean;
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

/// Milestone 9.13 decision 22: the `untrusted` row's text, the time the viewer's local
/// clock (milestone 9.19 decision 3, labelled, so it is not mistaken for the daemon's
/// UTC); `None` while the session read nothing from outside the run.
pub(super) fn untrusted_text(orchestrator: &OrchestratorInfo, app: &App) -> Option<String> {
    let u = orchestrator.untrusted.as_ref()?;
    Some(format!(
        "{} at {}; override, accept_red and approve_hold go to you",
        clean(&u.source),
        app.local_clock(u.at)
    ))
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

/// Milestone 9.17 decision 26: the `replaces` row, the tasks this one replaces.
pub(super) fn replaces_text(task: &TaskInfo) -> Option<String> {
    ids_text(&task.replaces)
}

/// Decision 26: the `replaced by` row, the tasks that replace this one.
pub(super) fn replaced_by_text(task: &TaskInfo) -> Option<String> {
    ids_text(&task.replaced_by)
}

fn ids_text(ids: &[String]) -> Option<String> {
    let ids: Vec<String> = ids.iter().map(|id| clean(id)).collect();
    (!ids.is_empty()).then(|| ids.join(", "))
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
                TaskNoteKind::Brief => "brief",
            };
            format!(
                "{} {kind} from {}: {}",
                app.local_clock(note.at),
                clean(&note.task_id),
                clean(&note.text)
            )
        })
        .collect();
    (!notes.is_empty()).then(|| notes.join(" · "))
}

/// Milestone 9.9 (OFA §4.6): the orchestrator's quiet history, as inspector rows — a
/// `handled` row with the count, then what it resolved, newest first, each with its
/// local time. Empty until it has handled something.
pub(super) fn handled_rows(orchestrator: &OrchestratorInfo, app: &App) -> Vec<(String, String)> {
    if orchestrator.handled_total == 0 {
        return Vec::new();
    }
    let mut rows = vec![(
        "handled".to_owned(),
        format!("orchestrator handled {}", orchestrator.handled_total),
    )];
    rows.extend(orchestrator.handled.iter().rev().map(|h| {
        (
            String::new(),
            format!(
                "{} {} {} — {}",
                app.local_clock(h.at),
                clean(&h.op),
                clean(&h.target),
                clean(&h.reason)
            ),
        )
    }));
    rows
}

/// Milestone 9.16 decision 30: `from <h4> · context <n>% full (<tokens>/<window>)`, or
/// `from <h4> · context <tokens> tokens` with no known window.
pub(super) fn handoff_text(handoff: &proto::HandoffInfo) -> String {
    let from = clean(&handoff.from_run);
    match handoff.window_tokens.filter(|w| *w > 0) {
        Some(window) => format!(
            "from {from} \u{b7} context {}% full ({}/{window})",
            u128::from(handoff.fill_tokens) * 100 / u128::from(window),
            handoff.fill_tokens
        ),
        None => format!("from {from} \u{b7} context {} tokens", handoff.fill_tokens),
    }
}
