//! Milestone 9.17 decision 26 (ruling C14): statistics judge a replaced task by its
//! replacement. A task record named by another record's `replaces` (the key is
//! `(run_id, task_id)`) is left out of the class rows and counted in `replaced`.

use std::path::Path;

use proto::{HistoryLine, Size, TaskOutcome, TaskRecord};

use super::tests::{PATH, merged, row, run, summarise_at, task};
use super::{aggregate, render};

/// `t1` of `run_id` cancelled as M after a session and two review bounces.
fn cancelled(run_id: &str) -> TaskRecord {
    let mut t1 = task(run_id, "t1", Size::M, false, TaskOutcome::Cancelled, 1);
    t1.bounces.review = 2;
    t1
}

/// `t2` of `run_id` merged as M, one done bounce, replacing `replaces`.
fn replacement(run_id: &str, replaces: &[&str]) -> TaskRecord {
    let mut t2 = merged(run_id, "t2", Size::M, (10, 4, 1_000, 60));
    t2.bounces.done = 1;
    t2.replaces = replaces.iter().map(|s| s.to_string()).collect();
    t2
}

fn lines(records: Vec<TaskRecord>) -> Vec<HistoryLine> {
    records.into_iter().map(HistoryLine::Task).collect()
}

#[test]
fn replaced_records_leave_the_rows() {
    let stats = aggregate(
        &lines(vec![cancelled("r1"), replacement("r1", &["t1"])]),
        Path::new(PATH),
    );
    let m = row(&stats, "M");
    assert_eq!((m.tasks, m.merged, m.bounces), (1, 1, 1), "{m:?}");
    assert_eq!(stats.replaced, 1);
    assert_eq!(stats.task_records, 2, "still a task record");

    // Without the link, t1 counts against its class as before.
    let unlinked = aggregate(
        &lines(vec![cancelled("r1"), replacement("r1", &[])]),
        Path::new(PATH),
    );
    let m = row(&unlinked, "M");
    assert_eq!((m.tasks, m.merged, m.bounces), (2, 1, 3), "{m:?}");
    assert_eq!(unlinked.replaced, 0);
}

/// The key is `(run_id, task_id)`: `r2`'s `t2` replacing `t1` says nothing of `r1`'s.
#[test]
fn same_task_id_in_another_run_is_not_excluded() {
    let stats = aggregate(
        &lines(vec![
            cancelled("r1"),
            cancelled("r2"),
            replacement("r2", &["t1"]),
        ]),
        Path::new(PATH),
    );
    let m = row(&stats, "M");
    assert_eq!((m.tasks, m.merged, m.bounces), (2, 1, 3), "{m:?}");
    assert_eq!(stats.replaced, 1);
}

/// `summarise` reads the last line of each `record_id`: a replaced record written twice
/// counts once, and a replacement whose last line names `t1` excludes it although its
/// first line did not.
#[test]
fn replaced_count_survives_a_rewritten_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    let text: Vec<String> = [
        HistoryLine::Task(cancelled("r1")),
        HistoryLine::Task(replacement("r1", &[])),
        HistoryLine::Task(cancelled("r1")),
        HistoryLine::Task(replacement("r1", &["t1"])),
        HistoryLine::Run(run("r1", 0, 0)),
    ]
    .iter()
    .map(|l| serde_json::to_string(l).unwrap())
    .collect();
    std::fs::write(&path, text.join("\n") + "\n").unwrap();
    let stats = summarise_at(dir.path(), &path);
    assert!(stats.problems.is_empty(), "{:?}", stats.problems);
    assert_eq!(stats.task_records, 2);
    assert_eq!(stats.replaced, 1);
    let m = row(&stats, "M");
    assert_eq!((m.tasks, m.merged, m.bounces), (1, 1, 1), "{m:?}");
}

/// `run stats` says how many records were judged by their replacements, after the
/// deciders' line, and says nothing while there are none.
#[test]
fn render_says_replaced_only_above_zero() {
    let none = aggregate(
        &lines(vec![cancelled("r1"), replacement("r1", &[])]),
        Path::new(PATH),
    );
    assert!(!render(&none).contains("replaced"), "{}", render(&none));
    let one = aggregate(
        &lines(vec![cancelled("r1"), replacement("r1", &["t1"])]),
        Path::new(PATH),
    );
    let text = render(&one);
    let deciders = text
        .lines()
        .position(|l| l.starts_with("deciders: "))
        .expect("the deciders' line");
    assert_eq!(
        text.lines().nth(deciders + 1),
        Some("replaced 1 (judged by their replacements)"),
        "{text}"
    );
    assert_eq!(render(&none).lines().count() + 1, text.lines().count());
}

/// Task 8 review m1: a chain is judged by its last link. `c` is cancelled, `b`
/// replaced it and was cancelled too, `a` replaced `b` and merged: both `b` and `c`
/// leave the rows, whatever their replacement's outcome, and only `a` counts.
#[test]
fn a_replacement_chain_is_judged_by_its_last_link() {
    let c = task("r1", "c", Size::M, false, TaskOutcome::Cancelled, 1);
    let mut b = task("r1", "b", Size::M, false, TaskOutcome::Cancelled, 1);
    b.bounces.review = 2;
    b.replaces = vec!["c".into()];
    let mut a = merged("r1", "a", Size::M, (10, 4, 1_000, 60));
    a.replaces = vec!["b".into()];
    let stats = aggregate(&lines(vec![c, b, a]), Path::new(PATH));
    let m = row(&stats, "M");
    assert_eq!((m.tasks, m.merged, m.bounces), (1, 1, 0), "{m:?}");
    assert_eq!(stats.replaced, 2);
    assert_eq!(stats.task_records, 3);
}
