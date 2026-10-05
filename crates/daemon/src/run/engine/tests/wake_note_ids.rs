//! Ruling T20-1: every wake-up the engine emits names each note it holds by its seq,
//! so the driver can paste each note at most once. The engine emits the same notes
//! again with any change before it applies the paste's `OrchestratorWoken` (its
//! failure recovery: a paste that failed is answered by the next change's wake-up).

use proto::PlanEdit;

use super::dispatch::edit;
use super::wake_notes::{approved, notes};
use crate::run::engine::{Effect, notes_seq};

/// Each `WakeOrchestrator`'s notes, with their seqs, and its `notes_seq`.
fn emitted(effects: &[Effect]) -> Vec<(Vec<(u64, String)>, u64)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::WakeOrchestrator {
                notes, notes_seq, ..
            } => Some((notes.clone(), *notes_seq)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_wake_up_names_each_note_by_its_seq() {
    let mut fx = approved();
    let first = emitted(&edit(&mut fx, vec![PlanEdit::Pause]));
    let seqs = fx
        .run()
        .orch
        .orchestrator
        .as_ref()
        .unwrap()
        .note_seqs
        .clone();
    let held: Vec<(u64, String)> = seqs.iter().copied().zip(notes(&fx)).collect();
    assert_eq!(first, [(held.clone(), notes_seq(fx.run()))]);
    // A change before the paste's `OrchestratorWoken` emits the same notes again, by
    // the same seqs, with the new one after them.
    let second = emitted(&edit(&mut fx, vec![PlanEdit::Resume]));
    assert_eq!(second.len(), 1, "{second:?}");
    let (notes, seq) = &second[0];
    assert_eq!(notes[..held.len()], held[..]);
    assert_eq!(notes.len(), held.len() + 1);
    assert_eq!(notes.last().unwrap().0, *seq);
    assert!(*seq > held.last().unwrap().0);
}
