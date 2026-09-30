//! Task M9.0.5.3: a task's detail (decisions 4 and 7), built from memory. Pure.

use proto::{AgentRole, DoneSignal, RunState, SummarySource, WORKER_SUMMARY_MAX};

use super::*;
use crate::run::engine::EngineState;
use crate::run::model::{AgentRound, DoneClaim};
use crate::run::orch::test_support::*;
use crate::run::snapshot::snapshot;

fn claim(summary: &str) -> DoneClaim {
    DoneClaim {
        summary: summary.into(),
        test: None,
        red: None,
        signal: DoneSignal::TaskDone,
        session: None,
    }
}

/// A worker round whose turn is closed (`turn_open: false`), with `text` as its last
/// message.
fn closed(session: u32, text: &str) -> AgentRound {
    let mut r = round(session, 1, Default::default());
    r.last_text = Some(text.into());
    r
}

fn open(session: u32, text: &str) -> AgentRound {
    let mut r = closed(session, text);
    r.turn_open = true;
    r
}

fn summary(run: &Run) -> (Option<String>, Option<SummarySource>) {
    let d = task_detail(run, "t0").expect("t0 exists");
    (d.worker_summary, d.summary_source)
}

fn running() -> Run {
    let mut run = run_of(1);
    run.state = RunState::Running;
    run
}

#[test]
fn summary_prefers_the_task_done_summary() {
    let mut run = running();
    let t0 = task_mut(&mut run, "t0");
    t0.rounds = vec![closed(1, "Added the stats command and its test.")];
    t0.done = Some(claim("stats: mean, median; tests added"));
    assert_eq!(
        summary(&run),
        (
            Some("stats: mean, median; tests added".into()),
            Some(SummarySource::TaskDone)
        )
    );
    // Decision 32's fallback claim has an empty summary: the last message stands in.
    task_mut(&mut run, "t0").done = Some(DoneClaim {
        signal: DoneSignal::TurnEndFallback,
        ..claim(" \n ")
    });
    assert_eq!(
        summary(&run),
        (
            Some("Added the stats command and its test.".into()),
            Some(SummarySource::LastMessage)
        )
    );
}

#[test]
fn summary_falls_back_to_the_last_message_of_a_closed_turn() {
    let mut run = running();
    task_mut(&mut run, "t0").rounds = vec![closed(1, "first round's words")];
    assert_eq!(
        summary(&run),
        (
            Some("first round's words".into()),
            Some(SummarySource::LastMessage)
        )
    );
    // A later round mid-turn: its text is not a summary yet, the closed round's is.
    task_mut(&mut run, "t0")
        .rounds
        .push(open(2, "Looking at the tests"));
    assert_eq!(summary(&run).0.as_deref(), Some("first round's words"));
    // A round that ended with its turn still open closed it for good.
    let latest = task_mut(&mut run, "t0").rounds.last_mut().unwrap();
    latest.ended = true;
    latest.ended_at = Some(1_700);
    assert_eq!(summary(&run).0.as_deref(), Some("Looking at the tests"));
    // A reviewer's round is never the worker's summary.
    let mut reviewer = closed(3, "LGTM");
    reviewer.role = AgentRole::Reviewer;
    task_mut(&mut run, "t0").rounds.push(reviewer);
    assert_eq!(summary(&run).0.as_deref(), Some("Looking at the tests"));
}

#[test]
fn summary_is_none_before_any_turn_ends() {
    let mut run = running();
    assert_eq!(summary(&run), (None, None));
    task_mut(&mut run, "t0").rounds = vec![open(1, "Looking at the tests")];
    assert_eq!(summary(&run), (None, None));
    // A closed turn that said nothing, or only blanks, has no summary either.
    task_mut(&mut run, "t0").rounds = vec![round(1, 3, Default::default()), closed(2, "\n \t")];
    assert_eq!(summary(&run), (None, None));
}

#[test]
fn summary_is_capped_and_sanitised_keeping_line_breaks() {
    let hostile = format!(
        "\u{1b}[2Jdone\u{202e}\u{7}\r\nnext line\u{200b}\r{}",
        "é".repeat(3000)
    );
    let check = |(text, _): (Option<String>, Option<SummarySource>)| {
        let text = text.expect("a summary");
        assert_eq!(text.chars().count(), WORKER_SUMMARY_MAX + 1, "{text:.60}");
        assert!(text.ends_with('…'));
        assert!(text.starts_with(" [2Jdone \nnext line\n"), "{text:.60?}");
        assert!(
            !text.contains(['\u{1b}', '\u{202e}', '\u{7}', '\r', '\u{200b}']),
            "{text:.60?}"
        );
    };
    let mut run = running();
    task_mut(&mut run, "t0").done = Some(claim(&hostile));
    check(summary(&run));
    let t0 = task_mut(&mut run, "t0");
    t0.done = None;
    t0.rounds = vec![closed(1, &hostile)];
    check(summary(&run));
}

/// Review focus 2: the detail carries the plan text that decision 16a keeps out of the
/// same run's snapshot.
#[test]
fn detail_carries_the_brief_and_acceptance_of_a_running_task() {
    let run = running();
    let mut state = EngineState::default();
    state.runs.insert(run.id.clone(), run.clone());
    let snap = snapshot(&state, 5_000);
    let info = &snap.runs[0].tasks[0];
    assert!(info.brief.is_empty() && info.acceptance.is_empty());

    let detail = task_detail(&run, "t0").unwrap();
    assert_eq!(detail.run_id, run.id);
    assert_eq!(detail.task_id, "t0");
    assert_eq!(detail.brief, "Brief t0");
    assert_eq!(detail.acceptance, ["Accept t0"]);
}

#[test]
fn unknown_run_or_task_is_none() {
    let run = running();
    assert_eq!(task_detail(&run, "t9"), None);
    // The service looks the run up first; an unknown one has no detail either.
    let state = EngineState::default();
    assert_eq!(
        state.runs.get(&run.id).and_then(|r| task_detail(r, "t0")),
        None
    );
}
