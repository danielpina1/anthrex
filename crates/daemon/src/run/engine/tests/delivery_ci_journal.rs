//! The final fix wave's A4: the journal never carries a CI log's text. Its `done` line
//! for a `FailedLogs` answer keeps the file's path and size with an empty tail, so a
//! replayed answer has no text: the engine fetches that log again. And `run.json`
//! keeps the log's text only until the summary; from then on, only the lines the fix
//! task's brief quotes.

use crate::host::LogFile;
use crate::run::delivery::CiPhase;
use crate::run::delivery::ops::HostResult;
use proto::CiCategory;

use super::delivery_ci::{
    RUN_A, answer_logs, ci_decide, ci_record, deciding, log_fetches, logged, red_view, summary,
    test_red,
};
use super::delivery_open::answer;
use super::delivery_watch::{poll_with, watched};
use super::fixture::*;
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
