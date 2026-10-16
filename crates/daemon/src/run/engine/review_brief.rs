//! Milestone 9.17 decisions 8 and 9: each `brief` finding of a review round becomes a
//! task note of kind `brief`, which reaches the orchestrator through milestone 9's
//! note path (its wake and the digest's `task_notes`) and the user through the run's
//! attention lines. Pure: it changes the run, no I/O.

use proto::{Finding, TaskNoteKind};

use super::dispatch::history;
use super::wake;
use crate::run::messages::one_line;
use crate::run::model::Run;
use crate::run::orch::json::label;
use crate::run::orch::{WorkerNote, add_worker_note};

/// Decision 8: the most `brief` notes a task keeps.
pub(super) const BRIEF_NOTES_PER_TASK: usize = 5;

/// Decision 8: records round `round`'s `brief` findings of task `i` as notes, in their
/// order. A finding whose severity and text a `brief` note of the task already holds,
/// from this round or an earlier one, is not added again (task 4 review I1); past
/// [`BRIEF_NOTES_PER_TASK`] a finding is dropped with the history line
/// `brief finding dropped: 5 already noted`. Each added note logs
/// `brief finding: <first 120 characters>` and wakes the orchestrator (decision 9),
/// without the reviewer's text for a review or CI fix task (ruling C4).
pub(super) fn record(run: &mut Run, i: usize, round: u32, findings: &[Finding], now: u64) {
    for finding in findings.iter().filter(|f| !f.target.is_code()) {
        let severity = label(&finding.severity);
        // `(<severity>): <text>`, what a later round repeating itself repeats.
        let said = one_line(&format!("({severity}): {}", finding.text));
        let text = format!("review round {round} {said}");
        let task = &run.tasks[i];
        let notes = &task.orch.worker_notes;
        let repeats = |n: &WorkerNote| {
            let rest = n.text.strip_prefix("review round ");
            let rest = rest.and_then(|r| r.split_once(' ')).map(|(_, rest)| rest);
            n.kind == TaskNoteKind::Brief && rest == Some(said.as_str())
        };
        if notes.iter().any(repeats) {
            continue;
        }
        let briefs = notes.iter().filter(|n| n.kind == TaskNoteKind::Brief);
        if briefs.count() >= BRIEF_NOTES_PER_TASK {
            let dropped = format!("brief finding dropped: {BRIEF_NOTES_PER_TASK} already noted");
            history(run, i, now, dropped);
            continue;
        }
        let task_id = task.id().to_string();
        let untrusted = crate::run::orch::untrusted::fix_kind(task).is_some();
        let first: String = text.chars().take(120).collect();
        let note = WorkerNote {
            at: now,
            kind: TaskNoteKind::Brief,
            text,
            seq: 0,
            lane: None,
        };
        add_worker_note(run, &task_id, note);
        history(run, i, now, format!("brief finding: {first}"));
        let wake = match untrusted {
            true => {
                format!("{task_id}'s reviewer says the brief is wrong; read it with run_status")
            }
            false => format!("{task_id}'s reviewer says the brief is wrong: {first}"),
        };
        wake::note(run, wake);
    }
}
