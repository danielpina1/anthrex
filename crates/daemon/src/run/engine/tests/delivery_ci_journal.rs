//! The final fix wave's A4: the journal never carries a CI log's text. Its `done` line
//! for a `FailedLogs` answer keeps the file's path and size with an empty tail, so a
//! replayed answer has no text: the engine fetches that log again. And `run.json`
//! keeps the log's text only until the summary; from then on, only the lines the fix
//! task's brief quotes.
//!
//! Milestone 9.7 decision 14 (DH §3.2): the queued `ci_summary` request, the `Decide`
//! op's journal intent and the pending op persist no log text either; `dispatch`
//! refills an empty log from the summarising record before every path that reads it.

use crate::decider::DeciderRequest;
use crate::host::LogFile;
use crate::run::delivery::CiPhase;
use crate::run::delivery::ops::HostResult;
use crate::run::engine::{Effect, EngineState, EventKind, OpKind, OpResult};
use crate::run::journal::JournalLine;
use crate::run::model::{OpId, Run};
use proto::{CiCategory, DeciderMode};

use super::control::resume;
use super::delivery_ci::{
    RUN_A, answer_logs, ci_decide, ci_record, deciding, log_fetches, logged, red_view, summary,
    test_red,
};
use super::delivery_open::answer;
use super::delivery_watch::{poll_with, watched};
use super::fixture::{Fixture, RUN_ID};
use super::merge::commit;

/// A `FailedLogs` answer as a replay of its journal line reads it: no text.
fn replayed(bytes: u64) -> HostResult {
    HostResult::Logs(LogFile {
        path: "/tmp/data/delivery/ci-28000000001.log".into(),
        bytes,
        truncated: false,
        tail: String::new(),
    })
}

#[test]
fn a_replayed_log_without_its_text_is_fetched_again() {
    let mut fx = watched();
    deciding(&mut fx);
    poll_with(&mut fx, red_view(&commit(1), test_red(RUN_A)));
    let (op, ci_run, _) = log_fetches(&fx)[0];
    answer(&mut fx, op, replayed(4_096));
    let rec = ci_record(&fx);
    assert_eq!(rec.phase, CiPhase::Logs, "not summarised without it");
    assert!(rec.fetched.is_empty(), "{rec:#?}");
    let line = format!(
        "stage 1: the failed log of CI run {ci_run} came back without its text (a journal replay); fetching it again"
    );
    assert!(logged(&fx, &line), "{:#?}", fx.run().log);
    let at = fx.now + 100;
    fx.send(at, crate::run::engine::EventKind::Tick);
    let again: Vec<u64> = log_fetches(&fx).iter().map(|(_, r, _)| *r).collect();
    assert_eq!(again, [ci_run]);
    answer_logs(&mut fx, "--- FAIL: a::works");
    let (_, input) = ci_decide(&fx);
    assert_eq!(input.log, "--- FAIL: a::works");
    // An empty log is no replay: it is summarised as it is.
    let mut fx = watched();
    deciding(&mut fx);
    poll_with(&mut fx, red_view(&commit(1), test_red(RUN_A)));
    let (op, _, _) = log_fetches(&fx)[0];
    answer(&mut fx, op, replayed(0));
    assert_eq!(ci_decide(&fx).1.log, "");
}

#[test]
fn after_the_summary_the_record_keeps_only_the_briefs_lines() {
    let mut fx = watched();
    deciding(&mut fx);
    poll_with(&mut fx, red_view(&commit(1), test_red(RUN_A)));
    let log: Vec<String> = (0..600)
        .map(|k| format!("line {k:03} of the log"))
        .collect();
    answer_logs(&mut fx, &log.join("\n"));
    assert_eq!(
        ci_record(&fx).text.lines().count(),
        600,
        "the decider's input"
    );
    let (op, _) = ci_decide(&fx);
    fx.decided(op, summary(&["boom"], &["a::works"], CiCategory::Test));
    let rec = ci_record(&fx);
    assert_ne!(rec.phase, CiPhase::Summarising);
    let kept: Vec<&str> = rec.text.lines().collect();
    assert_eq!(kept.len(), 200, "the brief's last 200 lines");
    assert_eq!(kept[0], "line 400 of the log");
    assert_eq!(kept[199], "line 599 of the log");
}

/// A log whose every line carries a marker no other text in the run has.
const MARKER: &str = "zq-ci-log-marker";

fn marked_log() -> String {
    (0..50)
        .map(|k| format!("{MARKER} line {k:02}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `ci_summary` requests among `effects`' ops.
fn emitted_requests(effects: &[Effect]) -> Vec<crate::decider::CiSummaryInput> {
    (effects.iter())
        .filter_map(|e| match e {
            Effect::Op {
                kind:
                    OpKind::Decide {
                        request: DeciderRequest::CiSummary(input),
                        ..
                    },
                ..
            } => Some(input.clone()),
            _ => None,
        })
        .collect()
}

/// A summarising stage: the red's log answered with [`marked_log`], its `Decide` emitted.
fn summarising() -> (Fixture, Vec<Effect>) {
    let mut fx = watched();
    deciding(&mut fx);
    poll_with(&mut fx, red_view(&commit(1), test_red(RUN_A)));
    let effects = answer_logs(&mut fx, &marked_log());
    assert_eq!(ci_record(&fx).phase, CiPhase::Summarising);
    (fx, effects)
}

/// A new daemon: the run as `journal::save_run` wrote it, read back, with `replay` as
/// the journal's results (every other op is lost).
fn restart_from_disk(fx: &mut Fixture, replay: Vec<(OpId, OpResult)>) -> Vec<Effect> {
    let replay = (replay.into_iter())
        .map(|(op, result)| (RUN_ID.to_string(), op, result))
        .collect();
    let json = serde_json::to_string(fx.run()).expect("serializes");
    let run: Run = serde_json::from_str(&json).expect("deserializes");
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        held: Vec::new(),
        runs: vec![run],
        replay,
    })
}

/// How many copies of [`marked_log`] `json` holds: every line carries the marker once.
fn copies(json: &str) -> usize {
    json.matches(MARKER).count() / marked_log().lines().count()
}

#[test]
fn the_queued_summary_request_persists_no_log() {
    // `run.json` keeps one copy, the record's own (`CiRecord.text`, kept until the
    // summary so a restore can refill the request: DH §3.2); the request none.
    let (fx, _) = summarising();
    let run = serde_json::to_string(fx.run()).expect("serializes");
    assert_eq!(copies(&run), 1, "only CiRecord.text");
    assert_eq!(ci_record(&fx).text, marked_log());
    let pending = serde_json::to_string(&fx.run().pending_ops).expect("serializes");
    assert_eq!(copies(&pending), 0, "the pending op carries the log");
    let (op, input) = ci_decide(&fx);
    assert_eq!(input.log, marked_log(), "in memory, the pending op has it");
    let kind = fx.run().pending_ops[&op].kind.clone();
    let intent = serde_json::to_string(&JournalLine::Intent { op, kind }).expect("serializes");
    assert!(intent.contains("\"CiSummary\""), "{intent}");
    assert_eq!(copies(&intent), 0, "the journal's intent carries the log");
    // Queued (no reader slot), it persists none either.
    let mut fx = watched();
    deciding(&mut fx);
    fx.run_mut().limits.max_readers = 0;
    poll_with(&mut fx, red_view(&commit(1), test_red(RUN_A)));
    answer_logs(&mut fx, &marked_log());
    let queued = fx.queued_deciders();
    assert_eq!(queued.len(), 1, "{queued:#?}");
    let json = serde_json::to_string(&fx.run().decider_queue).expect("serializes");
    assert_eq!(copies(&json), 0, "the queued request carries the log");
    let run = serde_json::to_string(fx.run()).expect("serializes");
    assert_eq!(copies(&run), 1, "only CiRecord.text");
}

#[test]
fn the_emitted_decide_carries_the_full_log() {
    let (fx, effects) = summarising();
    let emitted = emitted_requests(&effects);
    assert_eq!(emitted.len(), 1, "{effects:#?}");
    assert_eq!(emitted[0].log, marked_log());
    assert_eq!(
        emitted[0].log,
        ci_record(&fx).text,
        "log_text of the record"
    );
}

#[test]
fn a_restore_requeues_and_refills_the_log() {
    let (mut fx, _) = summarising();
    let (lost, _) = ci_decide(&fx);
    restart_from_disk(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    let emitted = emitted_requests(&effects);
    assert_eq!(emitted.len(), 1, "{effects:#?}");
    assert_eq!(emitted[0].log, marked_log());
    let (again, input) = ci_decide(&fx);
    assert_ne!(again, lost);
    assert_eq!(input.log, marked_log());
    // Its answer still completes the summary.
    fx.decided(again, summary(&["boom"], &["a::works"], CiCategory::Test));
    assert_eq!(ci_record(&fx).lines, ["boom"]);
}

#[test]
fn the_fallback_reads_the_refilled_log() {
    let (mut fx, _) = summarising();
    restart_from_disk(&mut fx, Vec::new());
    fx.run_mut().limits.decider_mode = DeciderMode::Off;
    resume(&mut fx);
    let rec = ci_record(&fx);
    assert_ne!(rec.phase, CiPhase::Summarising, "{rec:#?}");
    assert!(
        rec.source
            .as_deref()
            .is_some_and(|s| s.starts_with("fallback")),
        "{rec:#?}"
    );
    let log = marked_log();
    let last: Vec<&str> = log.lines().skip(10).collect();
    assert_eq!(rec.lines, last, "the log's last 40 lines (BR-11)");
}

#[test]
fn a_replayed_failure_falls_back_on_the_refilled_log() {
    // The pending op read back from `run.json` has no log: the fallback for its
    // replayed failure reads the record's.
    let (mut fx, _) = summarising();
    let (op, _) = ci_decide(&fx);
    let failed = OpResult::Failed {
        message: "no decider".into(),
    };
    restart_from_disk(&mut fx, vec![(op, failed)]);
    let rec = ci_record(&fx);
    assert_ne!(rec.phase, CiPhase::Summarising, "{rec:#?}");
    let log = marked_log();
    let last: Vec<&str> = log.lines().skip(10).collect();
    assert_eq!(rec.lines, last);
}

#[test]
fn only_the_requests_own_summarising_record_refills_it() {
    use crate::run::engine::delivery::refill_log;
    let (mut fx, _) = summarising();
    let (_, mut input) = ci_decide(&fx);
    let id = ci_record(&fx).decider.expect("summarising");
    input.log.clear();
    refill_log(fx.run(), id + 1, &mut input);
    assert_eq!(input.log, "", "another decider's record");
    // A log already there is kept.
    input.log = "kept".into();
    refill_log(fx.run(), id, &mut input);
    assert_eq!(input.log, "kept");
    // A record no longer summarising refills nothing.
    input.log.clear();
    set_phase(&mut fx, CiPhase::Logs);
    refill_log(fx.run(), id, &mut input);
    assert_eq!(input.log, "");
    set_phase(&mut fx, CiPhase::Summarising);
    refill_log(fx.run(), id, &mut input);
    assert_eq!(input.log, marked_log());
}

fn set_phase(fx: &mut Fixture, phase: CiPhase) {
    let stages = &mut fx.run_mut().delivery.stages;
    let rec = (stages.iter_mut()).find_map(|s| s.ci.last_mut());
    rec.expect("a record").phase = phase;
}
