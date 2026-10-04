//! Whole-branch review D, I-1: decision 23's single-writer notion in the TUI. A racer and
//! a test writer write their task as a worker does, so the Cancel confirm, DETAIL's
//! stage words and worker row, and the detail's freshness key see them (the alert
//! detail's worker row is pinned in `ui/alerts_detail_tests.rs`).

use super::run_task_sections as project;
use super::run_tests::app_of;
use crate::app::actions::cancel_stops;
use crate::app::task_detail::detail_key;
use crate::tree::run_fixtures::{pair_writing_fixture as writing, racers_live_fixture as racing};
use proto::{AgentRole, RaceLane, TaskState};

#[test]
fn the_cancel_confirm_counts_racers_and_test_writers() {
    let (snapshot, _) = racing();
    assert_eq!(
        cancel_stops(&snapshot.runs[0]),
        "2 workers: t2 racer a, t2 racer b"
    );
    let (snapshot, _) = writing();
    assert_eq!(cancel_stops(&snapshot.runs[0]), "1 worker: t3 test writer");
}

#[test]
fn detail_says_who_is_writing_a_racing_or_paired_task() {
    let app = app_of(racing());
    let (run, t2) = (&app.runs.runs[0], &app.runs.runs[0].tasks[0]);
    assert_eq!(project::stage_words(run, t2), "racers are working");
    let line = project::worker_line(t2, &app).expect("the racers");
    let lines: Vec<&str> = line.lines().collect();
    assert_eq!(lines.len(), 2, "{line}");
    assert!(lines[0].starts_with("racer a · claude"), "{line}");
    assert!(lines[1].starts_with("racer b r2 · codex"), "{line}");

    // Crowned: the winner's racer alone.
    let (mut snapshot, windows) = racing();
    let t2 = &mut snapshot.runs[0].tasks[0];
    t2.race.as_mut().unwrap().winner = Some(RaceLane::B);
    let app = app_of((snapshot, windows));
    let line = project::worker_line(&app.runs.runs[0].tasks[0], &app).expect("the winner");
    assert!(
        line.starts_with("racer b r2 · codex") && !line.contains('\n'),
        "{line}"
    );

    let app = app_of(writing());
    let (run, t3) = (&app.runs.runs[0], &app.runs.runs[0].tasks[0]);
    assert_eq!(t3.state, TaskState::Working);
    assert_eq!(project::stage_words(run, t3), "test writer is working");
    let line = project::worker_line(t3, &app).expect("the test writer");
    assert!(line.starts_with("test writer #1 · codex"), "{line}");
}

#[test]
fn a_racers_or_test_writers_turn_makes_the_detail_stale() {
    for (fixture, role) in [
        (racing(), AgentRole::Racer),
        (writing(), AgentRole::TestWriter),
    ] {
        let (mut snapshot, _) = fixture;
        let task = &mut snapshot.runs[0].tasks[0];
        let before = detail_key(task);
        let round = (task.rounds.iter_mut())
            .find(|round| round.role == role)
            .unwrap();
        round.turns += 1;
        assert_ne!(detail_key(task), before, "{role:?}");
    }
}
