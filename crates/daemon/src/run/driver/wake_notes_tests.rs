//! Ruling T20-1: each wake note is pasted at most once. The engine emits a notes-only
//! wake-up again with any change before it applies the `OrchestratorWoken` of the one
//! pasted; that re-emission, or a later wake-up holding the same notes, must not paste
//! them again.

use std::time::Duration;

use super::*;
use crate::run::orch::contract::wake_text;

/// The engine's view of run `r1`'s live orchestrator in window 1, its notes up to
/// `last_note_seq` still held (the `OrchestratorWoken` not applied yet).
fn seen(last_note_seq: u64) -> Seen {
    Seen {
        run_id: "r1".into(),
        window_id: 1,
        live: true,
        exited: false,
        terminal: false,
        launching: false,
        launches: 1,
        notes: true,
        last_note_seq,
        request: None,
        quiet: Duration::from_secs(1),
    }
}

/// A notes-only wake-up for `r1`'s window 1 holding the notes 1 to `notes_seq`, as the
/// engine builds it.
fn noted(notes_seq: u64) -> Pending {
    let notes: Vec<(u64, String)> = (1..=notes_seq).map(|n| (n, format!("note {n}"))).collect();
    let texts: Vec<String> = notes.iter().map(|(_, t)| t.clone()).collect();
    Pending {
        window_id: 1,
        text: wake_text("r1", &texts),
        digest_revision: 5,
        notes_seq,
        quiet: Duration::from_secs(1),
        generation: 0,
        request: None,
        notes,
    }
}

/// A full check (a tick or the window watch, the window ready) on an engine whose
/// newest note is `last_note_seq`: what it takes to paste.
fn check(wakes: &Wakes, last_note_seq: u64) -> Vec<(String, Pending)> {
    let (judged, epoch) = (wakes.generations(), wakes.epoch());
    let seen = [seen(last_note_seq)];
    wakes.confirm(&seen, epoch);
    wakes.keep_live(&seen, &judged);
    wakes.take(wakes.deliverable(&judged))
}

/// The race of the task 20 report: the paste ends (`delivered`), then the engine
/// re-emits the same notes before it applies the `OrchestratorWoken`, and the window
/// still reads `Idle`. The re-emission is not pasted.
#[test]
fn a_notes_wake_is_pasted_once_across_the_woken_race() {
    let wakes = Wakes::default();
    wakes.insert("r1".into(), noted(5));
    let taken = check(&wakes, 5);
    assert_eq!(taken.len(), 1, "the first paste");
    wakes.delivered("r1", Some(&taken[0].1));
    wakes.insert("r1".into(), noted(5));
    assert!(check(&wakes, 5).is_empty(), "the same notes pasted twice");
    // Nor a re-emission that arrives while the paste is under way.
    wakes.insert("r1".into(), noted(6));
    let taken = check(&wakes, 6);
    assert_eq!(taken.len(), 1, "a newer note is pasted");
    wakes.insert("r1".into(), noted(6));
    assert!(check(&wakes, 6).is_empty(), "taken while delivering");
    wakes.delivered("r1", Some(&taken[0].1));
    assert!(check(&wakes, 6).is_empty(), "pasted twice");
}

/// A paste that failed (or was stopped) delivered nothing: the notes' next wake-up is
/// pasted.
#[test]
fn a_failed_paste_leaves_its_notes_to_the_next_wake() {
    let wakes = Wakes::default();
    wakes.insert("r1".into(), noted(5));
    let taken = check(&wakes, 5);
    assert_eq!(taken.len(), 1);
    wakes.delivered("r1", None);
    wakes.insert("r1".into(), noted(5));
    assert_eq!(check(&wakes, 5).len(), 1, "the notes were never pasted");
}

/// A wake-up holding a note pasted already and a newer one (the engine added note 3
/// after the paste of 1 and 2, before it applied that paste's `OrchestratorWoken`)
/// pastes the newer note alone.
#[test]
fn a_later_wake_pastes_only_its_new_notes() {
    let wakes = Wakes::default();
    wakes.insert("r1".into(), noted(2));
    let taken = check(&wakes, 2);
    wakes.delivered("r1", Some(&taken[0].1));
    wakes.insert("r1".into(), noted(3));
    let taken = check(&wakes, 3);
    assert_eq!(taken.len(), 1);
    let p = &taken[0].1;
    assert_eq!(p.text, wake_text("r1", &["note 3".to_string()]));
    assert_eq!(p.notes, [(3, "note 3".to_string())]);
    assert_eq!(
        p.notes_seq, 3,
        "its OrchestratorWoken still covers every note"
    );
}

/// A round's request wake is D13's: pasted whole, its notes never cut; they count as
/// pasted for the notes-only wake-ups after it.
#[test]
fn a_request_wake_is_never_cut_and_its_notes_count_as_pasted() {
    let wakes = Wakes::default();
    wakes.insert("r1".into(), noted(2));
    let taken = check(&wakes, 2);
    wakes.delivered("r1", Some(&taken[0].1));
    let request = Pending {
        request: Some(2),
        ..noted(3)
    };
    wakes.insert("r1".into(), request.clone());
    let (judged, _) = (wakes.generations(), wakes.epoch());
    let taken = wakes.take(wakes.deliverable(&judged));
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].1.text, request.text, "a request wake was cut");
    wakes.delivered("r1", Some(&taken[0].1));
    wakes.insert("r1".into(), noted(3));
    assert!(
        check(&wakes, 3).is_empty(),
        "the request's notes pasted twice"
    );
}
