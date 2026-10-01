//! Ruling F-2 (2026-10-01): a failed turn that the engine will continue says so in the
//! task's history, which the task log and `REPORT.md` show: `<role> round <n>: turn
//! failed (<error, one line, capped>); continuing at <time>`. A failed turn that blocks
//! adds only the block's line.

use super::fixture::*;
use super::gates_review::reviewed;
use super::kinds_research::researching;
use super::turns::working;
use crate::headless::FailureKind;
use crate::run::engine::TurnOutcome;
use crate::run::model::FailedTurn;
use crate::run::report::{format_utc, render};
use crate::run::snapshot::snapshot;

fn failed(kind: FailureKind, error: &str) -> TurnOutcome {
    TurnOutcome::Failed {
        error: error.into(),
        kind,
    }
}

fn last_history(fx: &Fixture, id: &str) -> String {
    fx.task(id).history.last().unwrap().text.clone()
}

/// The continue's due time of task `id`'s latest round.
fn continue_at(fx: &Fixture, id: &str) -> u64 {
    match fx.task(id).rounds.last().unwrap().failed_turn {
        FailedTurn::WaitingContinue { at, .. } => at,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_workers_failed_turn_is_noted_with_its_continue_time() {
    for kind in [FailureKind::Other, FailureKind::RateLimit] {
        let (mut fx, window) = working();
        fx.turn_ended(window, failed(kind, "overloaded"));
        let at = continue_at(&fx, "t1");
        let line = format!(
            "worker round 1: turn failed (overloaded); continuing at {}",
            format_utc(at)
        );
        assert_eq!(last_history(&fx, "t1"), line, "{kind:?}");
        // The task log and the report show it.
        let info = snapshot(&fx.state, fx.now);
        let log = &info.runs[0].tasks[0].history;
        assert!(log.iter().any(|e| e.text == line), "{log:#?}");
        let report = render(fx.run(), fx.now);
        assert!(report.contains(&line), "{report}");
    }
}

#[test]
fn a_reviewers_failed_turn_is_noted() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    fx.turn_ended(rwindow, failed(FailureKind::Other, "stream disconnected"));
    let at = continue_at(&fx, "t1");
    assert_eq!(
        last_history(&fx, "t1"),
        format!(
            "reviewer round 1: turn failed (stream disconnected); continuing at {}",
            format_utc(at)
        )
    );
}

#[test]
fn a_research_sessions_failed_turn_is_noted() {
    let (mut fx, window) = researching();
    fx.turn_ended(window, failed(FailureKind::Other, "stream disconnected"));
    let at = continue_at(&fx, "r1");
    assert_eq!(
        last_history(&fx, "r1"),
        format!(
            "research round 1: turn failed (stream disconnected); continuing at {}",
            format_utc(at)
        )
    );
}

/// The error is one line, its control and format characters made safe, and capped.
#[test]
fn the_noted_error_is_one_safe_line_and_capped() {
    let (mut fx, window) = working();
    let error = format!("first\nsecond\x1b[31m red\u{202e} {}", "x".repeat(1_000));
    fx.turn_ended(window, failed(FailureKind::Other, &error));
    let line = last_history(&fx, "t1");
    assert!(
        !line.chars().any(|c| c.is_control() || c == '\u{202e}'),
        "{line:?}"
    );
    assert!(
        line.starts_with("worker round 1: turn failed (first second"),
        "{line}"
    );
    let error_part = line
        .strip_prefix("worker round 1: turn failed (")
        .and_then(|rest| rest.split("); continuing at ").next())
        .unwrap();
    assert!(
        error_part.chars().count() <= 201,
        "{}",
        error_part.chars().count()
    );
    assert!(error_part.ends_with('…'), "{error_part}");
}

/// A failed turn that blocks is not continued, so it has no continue line.
#[test]
fn a_blocking_failure_adds_no_continue_line() {
    for kind in [FailureKind::ClientError, FailureKind::Authentication] {
        let (mut fx, window) = working();
        fx.turn_ended(window, failed(kind, "model not supported"));
        let history = &fx.task("t1").history;
        assert!(
            !history.iter().any(|e| e.text.contains("turn failed")),
            "{history:#?}"
        );
        assert_eq!(
            last_history(&fx, "t1"),
            "blocked (environment): model not supported"
        );
    }
    // A second failure in a row blocks too: its own line is the block's.
    let (mut fx, window) = working();
    fx.turn_ended(window, failed(FailureKind::Other, "overloaded"));
    let at = continue_at(&fx, "t1");
    fx.send(at, crate::run::engine::EventKind::Tick);
    fx.turn_ended(window, failed(FailureKind::Other, "overloaded again"));
    let lines: Vec<&str> = fx
        .task("t1")
        .history
        .iter()
        .map(|e| e.text.as_str())
        .filter(|t| t.contains("turn failed") || t.starts_with("blocked"))
        .collect();
    assert_eq!(lines.len(), 2, "{lines:#?}");
    assert_eq!(lines[1], "blocked (environment): overloaded again");
}
