//! Milestone 9 decision 39, engine side: the orchestrator's wake notes, their cap, the
//! wake-up effect, and what clears them (a delivered wake-up, a digest read). Every
//! note source goes through [`note`]. Pure (design decision 2).

use proto::{BlockReason, TaskState};

use super::Effect;
use crate::run::model::Run;
use crate::run::orch::contract::wake_text;

/// Decision 39: the most notes kept; the oldest become one `+<n> earlier changes`.
pub const NOTES_MAX: usize = 20;

/// A note's revision until the step that added it ends ([`settle`]).
const PENDING: u64 = u64::MAX;

/// A blocked task's note carries this many characters of its block's text.
const BLOCK_TEXT_CHARS: usize = 120;

/// The count an `+<n> earlier changes` line stands for.
fn earlier(text: &str) -> Option<u64> {
    text.strip_prefix('+')?
        .strip_suffix(" earlier changes")?
        .parse()
        .ok()
}

/// Adds `text` to the orchestrator's pending notes, if the run has an orchestrator.
/// Past [`NOTES_MAX`] the oldest are folded into the first line, `+<n> earlier
/// changes`, which counts every note it replaced.
pub(super) fn note(run: &mut Run, text: String) {
    let Some(o) = run.orch.orchestrator.as_mut() else {
        return;
    };
    o.note_revs.resize(o.notes.len(), 0);
    o.notes.push(text);
    o.note_revs.push(PENDING);
    if o.notes.len() <= NOTES_MAX {
        return;
    }
    let cut = o.notes.len() - (NOTES_MAX - 1);
    let dropped: Vec<String> = o.notes.drain(..cut).collect();
    let revs: Vec<u64> = o.note_revs.drain(..cut).collect();
    let count: u64 = dropped.iter().map(|n| earlier(n).unwrap_or(1)).sum();
    o.notes.insert(0, format!("+{count} earlier changes"));
    o.note_revs.insert(0, revs.into_iter().max().unwrap_or(0));
}

/// The notes added by this step take the digest revision it ended at (called after
/// the revision moved, `engine::finish`).
pub(super) fn settle(run: &mut Run) {
    let rev = run.orch.digest_rev;
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.note_revs.resize(o.notes.len(), 0);
        for r in o.note_revs.iter_mut().filter(|r| **r == PENDING) {
            *r = rev;
        }
    }
}

/// Drops the notes of `revision` and earlier: the orchestrator has seen them.
fn drop_up_to(run: &mut Run, revision: u64) {
    let Some(o) = run.orch.orchestrator.as_mut() else {
        return;
    };
    o.note_revs.resize(o.notes.len(), 0);
    let keep: Vec<(String, u64)> = o
        .notes
        .drain(..)
        .zip(o.note_revs.drain(..))
        .filter(|(_, r)| *r > revision)
        .collect();
    (o.notes, o.note_revs) = keep.into_iter().unzip();
}

/// Decisions 16 and 39: the orchestrator read the digest at `revision`; the notes up to
/// it are dropped, so an orchestrator that polls is never pasted at.
pub(super) fn digest_read(run: &mut Run, revision: u64, now: u64) {
    run.orch.digest_read_at = Some(now);
    drop_up_to(run, revision);
}

/// Decision 39: the driver pasted the wake-up of `revision`.
pub(super) fn woken(run: &mut Run, revision: u64) {
    drop_up_to(run, revision);
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.last_wake_rev = o.last_wake_rev.max(revision);
        o.wakes = o.wakes.saturating_add(1);
    }
}

/// Decision 39's wake-up, for a run the step changed: notes pending, the orchestrator's
/// window live, `wake_orchestrator` on, and a digest revision newer than the last
/// wake-up's.
pub(super) fn effect(run: &Run) -> Option<Effect> {
    let o = run.orch.orchestrator.as_ref()?;
    let window_id = o.window_id?;
    let due = !o.notes.is_empty()
        && o.live
        && run.limits.orch.wake_orchestrator
        && run.orch.digest_rev > o.last_wake_rev;
    due.then(|| Effect::WakeOrchestrator {
        run_id: run.id.clone(),
        window_id,
        text: wake_text(&run.id, &o.notes),
        digest_revision: run.orch.digest_rev,
    })
}

/// Decision 39: a note for each task this step blocked, for any reason but
/// `message_pause`, unless the step was the orchestrator's own edit (`quiet`).
pub(super) fn blocked_notes(before: Option<&Run>, run: &mut Run, quiet: bool) {
    let (Some(before), false) = (before, quiet) else {
        return;
    };
    if run.orch.orchestrator.is_none() {
        return;
    }
    let mut lines = Vec::new();
    for task in &run.tasks {
        let Some(info) = task
            .block
            .as_ref()
            .filter(|_| task.state == TaskState::Blocked)
        else {
            continue;
        };
        if info.reason == BlockReason::MessagePause {
            continue;
        }
        let was = before.task(task.id());
        let new = was.is_none_or(|w| w.state != TaskState::Blocked || w.block != task.block);
        if !new {
            continue;
        }
        let label = serde_json::to_value(info.reason)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let text: String = info.text.chars().take(BLOCK_TEXT_CHARS).collect();
        lines.push(format!("{} blocked ({label}): {text}", task.id()));
    }
    for line in lines {
        note(run, line);
    }
}
