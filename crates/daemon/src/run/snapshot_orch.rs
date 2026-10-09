//! Milestone 9's part of the pushed snapshot (task M9.6): decision 16a's plan text
//! only at the gate, and decision 42d's message counts and bounded task notes. Pure.

use proto::{
    HoldInfo, IntegrationInfo, IntegrationState, OrchestratorInfo, PlannerInfo, PlannerState,
    TaskNoteInfo,
};

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

/// Decision 42c: a `paused(message)` task is an attention line once it has lasted this
/// long.
pub const PAUSED_ATTENTION_SECS: u64 = 600;

/// Decision 42c: `<t> paused(message) for <n> min`, once the pause has lasted
/// [`PAUSED_ATTENTION_SECS`].
pub(super) fn paused_line(task: &Task, now: u64) -> Option<String> {
    let lasted = now.saturating_sub(task.phase_since);
    (lasted >= PAUSED_ATTENTION_SECS)
        .then(|| format!("{} paused(message) for {} min", task.id(), lasted / 60))
}

/// Decision 42i: the last three `discovery` and `risk` notes across tasks, newest
/// first, as `<t> noted a <kind>: <first 80 characters>`.
pub(super) fn noted_lines(run: &Run) -> Vec<String> {
    let mut notes: Vec<(&Task, &crate::run::orch::WorkerNote)> = run
        .tasks
        .iter()
        .flat_map(|t| t.orch.worker_notes.iter().map(move |n| (t, n)))
        .filter(|(_, n)| n.kind != proto::TaskNoteKind::Progress)
        .collect();
    notes.sort_by_key(|(_, n)| std::cmp::Reverse((n.at, n.seq)));
    notes
        .into_iter()
        .take(3)
        .map(|(t, n)| {
            let first: String = n
                .text
                .chars()
                .take(80)
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            let kind = crate::run::orch::json::label(&n.kind);
            format!("{} noted a {kind}: {first}", t.id())
        })
        .collect()
}

/// Decision 11's orchestrator, as the run view shows it (never its OTLP token).
pub(super) fn orchestrator(run: &Run) -> Option<OrchestratorInfo> {
    let o = run.orch.orchestrator.as_ref()?;
    Some(OrchestratorInfo {
        route: o.route.clone(),
        window_id: o.window_id,
        live: o.live,
        started_at: o.started_at,
        plan_submitted: o.plan_submitted,
        summary: o.summary.clone(),
        notes: o.notes.clone(),
        wakes: o.wakes,
        wake_held: run.orch.wake_held,
        // Milestone 9.9: `handled` from `RunOrch` (M9.9.2); the rest by M9.9.6 and M9.9.7.
        stuck: None,
        ask: None,
        handled: run
            .orch
            .handled
            .iter()
            .map(|h| proto::HandledInfo {
                at: h.at,
                op: h.op.clone(),
                target: h.target.clone(),
                reason: h.reason.clone(),
            })
            .collect(),
        handled_total: run.orch.handled_total,
    })
}

/// Decision 28's approval holds, in creation order.
pub(super) fn holds(run: &Run) -> Vec<HoldInfo> {
    run.orch
        .gate_holds
        .iter()
        .map(|h| HoldInfo {
            id: h.id.clone(),
            kind: h.kind.clone(),
            state: h.state,
            tasks: h.tasks.clone(),
            created_at: h.created_at,
            decided_at: h.decided_at,
            decided_by: h.decided_by.clone(),
        })
        .collect()
}

/// Decision 33: one `PlannerInfo` per epic. A queued planner shows `Planning` (M8c's
/// `PlannerState` has no queued); `window_id` is the latest session's, `started_at` the
/// first session's (the epic's when none has started) and `ended_at` the last's.
pub(super) fn planners(run: &Run) -> Vec<PlannerInfo> {
    use crate::run::orch::PlannerPhase;
    run.orch
        .epics
        .iter()
        .map(|e| PlannerInfo {
            epic: e.epic.clone(),
            title: e.title.clone(),
            area: e.area.clone(),
            route: e.route.clone(),
            window_id: e.sessions.last().and_then(|s| s.window_id),
            state: match e.phase {
                PlannerPhase::Queued | PlannerPhase::Planning => PlannerState::Planning,
                PlannerPhase::Finished => PlannerState::Finished,
                PlannerPhase::Failed { .. } => PlannerState::Failed,
            },
            started_at: e.sessions.first().map_or(e.started_at, |s| s.started_at),
            ended_at: e.ended_at,
            edits_accepted: e.edits_accepted,
            edits_rejected: e.edits_rejected,
            last_rejection: e.last_rejection.clone(),
            replans: e.replans.clone(),
            note: e.note.clone(),
            covers: e.covers.clone(),
        })
        .collect()
}

/// Decision 37: each epic whose integration review started, with its base, its merge
/// commits and its `<e>-int<n>` tasks.
pub(super) fn integration(run: &Run) -> Vec<IntegrationInfo> {
    run.orch
        .epics
        .iter()
        .filter(|e| e.integration_state != IntegrationState::NotYet)
        .map(|e| IntegrationInfo {
            epic: e.epic.clone(),
            state: e.integration_state,
            base: e.base.clone(),
            merges: e.merges.iter().map(|(_, c)| c.clone()).collect(),
            tasks: run
                .tasks
                .iter()
                .filter(|t| t.orch.integration_of.as_deref() == Some(e.epic.as_str()))
                .map(|t| t.id().to_string())
                .collect(),
        })
        .collect()
}

/// Decision 35: `<data_dir>/research.md`, once a research task reported.
pub(super) fn research_report(run: &Run) -> Option<std::path::PathBuf> {
    run.tasks
        .iter()
        .any(|t| t.orch.research.is_some())
        .then(|| run.data_dir.join(super::report::RESEARCH_FILE))
}
