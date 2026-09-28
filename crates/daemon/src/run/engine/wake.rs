//! Milestone 9 decision 39, engine side: the orchestrator's wake notes, their cap, the
//! wake-up effect, and what clears them (a delivered wake-up, a digest read). Every
//! note source goes through [`note`]. Pure (design decision 2).

use proto::{BlockReason, TaskState};

use super::Effect;
use crate::run::model::Run;
use crate::run::orch::contract::wake_text;

/// Decision 39: the most notes kept; the oldest become one `+<n> earlier changes`.
pub const NOTES_MAX: usize = 20;

/// A blocked task's note carries this many characters of its block's text.
const BLOCK_TEXT_CHARS: usize = 120;

/// The count an `+<n> earlier changes` line stands for.
fn earlier(text: &str) -> Option<u64> {
    text.strip_prefix('+')?
        .strip_suffix(" earlier changes")?
        .parse()
        .ok()
}

/// M9.9 review fixes, I1: a note is one line. Every control character (`\n`, `\r`,
/// U+0085 among them) and the Unicode line and paragraph separators become spaces, so
/// no source can start a line of its own in the pasted wake-up.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// Adds `text` to the orchestrator's pending notes, if the run has an orchestrator,
/// with the next note seq. Past [`NOTES_MAX`] the oldest are folded into the first
/// line, `+<n> earlier changes`, which counts every note it replaced and takes the
/// highest seq among them.
pub(super) fn note(run: &mut Run, text: String) {
    let Some(o) = run.orch.orchestrator.as_mut() else {
        return;
    };
    o.note_seqs.resize(o.notes.len(), 0);
    o.last_note_seq = o.last_note_seq.saturating_add(1);
    o.notes.push(one_line(&text));
    o.note_seqs.push(o.last_note_seq);
    if o.notes.len() <= NOTES_MAX {
        return;
    }
    let cut = o.notes.len() - (NOTES_MAX - 1);
    let dropped: Vec<String> = o.notes.drain(..cut).collect();
    let seqs: Vec<u64> = o.note_seqs.drain(..cut).collect();
    let count: u64 = dropped.iter().map(|n| earlier(n).unwrap_or(1)).sum();
    o.notes.insert(0, format!("+{count} earlier changes"));
    o.note_seqs.insert(0, seqs.into_iter().max().unwrap_or(0));
}

/// The highest seq among the pending notes, 0 with none: what a digest answer or a
/// wake-up built now includes (M9.9 review fixes, M6).
pub fn notes_seq(run: &Run) -> u64 {
    run.orch
        .orchestrator
        .as_ref()
        .and_then(|o| o.note_seqs.iter().copied().max())
        .unwrap_or(0)
}

/// Drops the notes of seq `seq` and lower: the orchestrator has seen them. A note
/// added after the answer or wake-up was built has a higher seq and stays.
fn drop_up_to(run: &mut Run, seq: u64) {
    let Some(o) = run.orch.orchestrator.as_mut() else {
        return;
    };
    o.note_seqs.resize(o.notes.len(), 0);
    let keep: Vec<(String, u64)> = o
        .notes
        .drain(..)
        .zip(o.note_seqs.drain(..))
        .filter(|(_, s)| *s > seq)
        .collect();
    (o.notes, o.note_seqs) = keep.into_iter().unzip();
}

/// Decisions 16 and 39: the orchestrator read a digest whose answer included the
/// notes up to `notes_seq`; those are dropped, so an orchestrator that polls is never
/// pasted at.
pub(super) fn digest_read(run: &mut Run, notes_seq: u64, now: u64) {
    run.orch.digest_read_at = Some(now);
    drop_up_to(run, notes_seq);
}

/// Decision 39: the driver pasted the wake-up of `revision`, which held the notes up
/// to `notes_seq`.
pub(super) fn woken(run: &mut Run, revision: u64, notes_seq: u64) {
    drop_up_to(run, notes_seq);
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
        notes_seq: notes_seq(run),
    })
}

/// Decision 39: a note for each task blocked since `before`, for any reason but
/// `message_pause`. For the orchestrator's own tool call, `before` is the run as the
/// call left it (M9.9 review fixes, I2), so only the blocks its edit caused are quiet.
pub(super) fn blocked_notes(before: Option<&Run>, run: &mut Run) {
    let Some(before) = before else {
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
