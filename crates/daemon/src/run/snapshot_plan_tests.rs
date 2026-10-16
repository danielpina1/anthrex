//! Milestone 9.17 task M9.17.4 (decision 10): what the user sees of a `brief` note in
//! the run's attention lines. Pure.

use proto::TaskNoteKind;

use crate::run::engine::EngineState;
use crate::run::model::Run;
use crate::run::orch::add_worker_note;
use crate::run::orch::test_support::{note, orchestrator, run_of};
use crate::run::snapshot::snapshot;

const STICKY: &str =
    "t1: the reviewer says the brief is wrong (1 notes); read them in the task's notes";

/// Run `run`'s attention lines as the snapshot carries them.
fn attention(run: &Run, now: u64) -> Vec<String> {
    let mut state = EngineState::default();
    state.runs.insert(run.id.clone(), run.clone());
    snapshot(&state, now).runs.remove(0).attention
}

/// `run_of(2)` with one `brief` note on `t1` and then four later `risk` notes.
fn noted_run() -> Run {
    let mut run = run_of(2);
    let text = "review round 1 (critical): the brief asks for the wrong table";
    assert!(add_worker_note(
        &mut run,
        "t1",
        note(100, TaskNoteKind::Brief, text)
    ));
    for n in 0..4 {
        let risk = note(200 + n, TaskNoteKind::Risk, &format!("risk {n}"));
        assert!(add_worker_note(&mut run, "t0", risk));
    }
    run
}

#[test]
fn plan_file_run_has_a_sticky_brief_attention_line() {
    let run = noted_run();
    assert!(run.orch.orchestrator.is_none());
    let lines = attention(&run, 1_000);
    assert!(lines.contains(&STICKY.to_string()), "{lines:?}");
    assert!(
        !lines.iter().any(|l| l.contains("review flagged the brief")),
        "pushed out of the last three notes: {lines:?}"
    );
    // The count follows the notes.
    let mut run = run;
    let more = note(300, TaskNoteKind::Brief, "review round 2 (minor): again");
    assert!(add_worker_note(&mut run, "t1", more));
    let two = STICKY.replace("(1 notes)", "(2 notes)");
    assert!(attention(&run, 1_000).contains(&two));
    // A finished run has nobody left to act.
    run.state = proto::RunState::Accepted;
    assert!(
        !attention(&run, 1_000)
            .iter()
            .any(|l| l.contains("brief is wrong"))
    );
}

#[test]
fn orchestrator_run_has_no_sticky_line() {
    let mut run = noted_run();
    run.orch.orchestrator = Some(orchestrator());
    let lines = attention(&run, 1_000);
    assert!(
        !lines.iter().any(|l| l.contains("brief is wrong")),
        "{lines:?}"
    );
}

/// Decision 7: a round whose only serious findings are `brief` did not block, and the
/// snapshot's `ReviewInfo.blocking` says so.
#[test]
fn brief_only_review_is_not_blocking() {
    use crate::run::orch::test_support::review;
    use proto::{Severity, Verdict};
    let mut run = run_of(2);
    let mut brief_only = review(1, Some(Verdict::Changes), &[(Severity::Critical, "b")]);
    brief_only.findings[0].target = proto::FindingTarget::Brief;
    let code = review(2, Some(Verdict::Changes), &[(Severity::Important, "c")]);
    run.tasks[1].reviews = vec![brief_only, code];
    let mut state = EngineState::default();
    state.runs.insert(run.id.clone(), run);
    let snap = snapshot(&state, 1_000);
    let blocking: Vec<bool> = (snap.runs[0].tasks.iter())
        .find(|t| t.id == "t1")
        .map(|t| t.reviews.iter().map(|r| r.blocking).collect())
        .unwrap();
    assert_eq!(blocking, [false, true]);
}

#[test]
fn noted_line_words_a_brief_note() {
    let mut run = run_of(2);
    let long = format!("review round 1 (important): {}", "y".repeat(100));
    assert!(add_worker_note(
        &mut run,
        "t1",
        note(100, TaskNoteKind::Brief, &long)
    ));
    assert!(add_worker_note(
        &mut run,
        "t0",
        note(90, TaskNoteKind::Risk, "r")
    ));
    let first: String = long.chars().take(80).collect();
    let lines = attention(&run, 1_000);
    assert!(
        lines.contains(&format!("t1 review flagged the brief: {first}")),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"t0 noted a risk: r".to_string()),
        "{lines:?}"
    );
}
