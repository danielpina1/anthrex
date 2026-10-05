//! Ruling T20-1: each wake note is pasted at most once.
//!
//! The engine emits a notes-only wake-up again with any change until it applies the
//! `OrchestratorWoken` of the one pasted (`engine/wake.rs`, `effect`), and a window that
//! was just pasted at still reads `Idle` for a moment. So the driver remembers, per run,
//! the newest note seq it pasted (a note's seq is its id: seqs only grow within a run).
//! A notes-only wake-up taken for delivery loses every note at or below it, its text
//! built again from the rest; with none left it is not pasted. A failed or stopped
//! paste is not remembered, so its notes go with the next wake-up. A round's request
//! wake is D13's (`Wakes::pasted`) and is never cut; its notes count as pasted.

use std::collections::HashMap;

use super::Pending;
use crate::run::orch::contract::wake_text;

/// The newest note seq pasted, per run.
#[derive(Default)]
pub(super) struct NotesPasted(std::sync::Mutex<HashMap<String, u64>>);

impl NotesPasted {
    /// `p` was pasted into `run_id`'s window: its notes are delivered.
    pub(super) fn remember(&self, run_id: &str, p: &Pending) {
        if p.notes_seq == 0 {
            return;
        }
        let mut pasted = crate::lock(&self.0);
        let seq = pasted.entry(run_id.to_string()).or_default();
        *seq = (*seq).max(p.notes_seq);
    }

    /// `p`, about to be pasted into `run_id`'s window, without the notes already
    /// pasted; `None` when nothing new is left to say.
    pub(super) fn unpasted(&self, run_id: &str, mut p: Pending) -> Option<Pending> {
        if p.request.is_some() {
            return Some(p);
        }
        let Some(&seq) = crate::lock(&self.0).get(run_id) else {
            return Some(p);
        };
        if p.notes_seq <= seq {
            return None;
        }
        let before = p.notes.len();
        p.notes.retain(|(s, _)| *s > seq);
        if p.notes.len() != before {
            let texts: Vec<String> = p.notes.iter().map(|(_, t)| t.clone()).collect();
            p.text = wake_text(run_id, &texts);
        }
        Some(p)
    }
}
