//! Milestone 9.17 (decision 11, rulings C10 and C14): a task record counts its `brief`
//! findings apart from the code findings. Pure.

use proto::{FindingTarget, Severity, SeverityTally, TaskOutcome, TaskState, Verdict};

use super::task_record;
use super::tests::fixtures::{finding, review, run_of};

fn brief(severity: Severity) -> proto::Finding {
    proto::Finding {
        target: FindingTarget::Brief,
        ..finding(severity)
    }
}

#[test]
fn brief_findings_are_counted_apart() {
    let mut run = run_of(&["t1"]);
    run.tasks[0].reviews = vec![
        review(
            1,
            Verdict::Changes,
            vec![finding(Severity::Critical), brief(Severity::Critical)],
        ),
        review(2, Verdict::Approve, vec![brief(Severity::Minor)]),
    ];
    let task = run.tasks[0].clone();
    let record = task_record(&run, &task, TaskOutcome::Merged, 1_000);
    assert_eq!(
        record.severities,
        SeverityTally {
            critical: 1,
            important: 0,
            minor: 0,
        }
    );
    assert_eq!(record.brief_findings, 2);
    assert_eq!(record.gates.review_rounds, 2);
    assert_eq!(record.gates.reviews_rejected, 1);
}

/// Ruling C10: a `changes` round whose critical and important findings are all
/// `brief` is not a rejection; a minor-only `changes` round still is, as before.
#[test]
fn brief_only_changes_round_is_not_rejected() {
    let mut run = run_of(&["t1"]);
    run.tasks[0].reviews = vec![
        review(1, Verdict::Changes, vec![brief(Severity::Important)]),
        review(
            2,
            Verdict::Changes,
            vec![brief(Severity::Critical), finding(Severity::Minor)],
        ),
        review(3, Verdict::Changes, vec![finding(Severity::Minor)]),
        review(4, Verdict::Changes, vec![]),
    ];
    let task = run.tasks[0].clone();
    let record = task_record(&run, &task, TaskOutcome::Merged, 1_000);
    assert_eq!(record.gates.review_rounds, 4);
    assert_eq!(record.gates.reviews_rejected, 2, "rounds 3 and 4 only");
    assert_eq!(record.brief_findings, 2);
}

/// Decision 26: a replacement's record names the tasks it replaces, from
/// `Task.replaces`; a task that replaces nothing records an empty list. The run is
/// one validation allows (task 8 review m2): only a cancelled or blocked task is
/// replaced, so `t1` and `t2` are cancelled, `t3` replaced `t1` and was cancelled in
/// turn, and `t4` replaces `t2` and `t3` and merges.
#[test]
fn replacement_record_names_what_it_replaces() {
    let mut run = run_of(&["t1", "t2", "t3", "t4"]);
    for i in 0..3 {
        run.tasks[i].state = TaskState::Cancelled;
    }
    run.tasks[3].state = TaskState::Merged;
    run.tasks[2].replaces = vec!["t1".to_string()];
    run.tasks[3].replaces = vec!["t2".to_string(), "t3".to_string()];
    let record = |i: usize, outcome: TaskOutcome| {
        let task = run.tasks[i].clone();
        task_record(&run, &task, outcome, 1_000)
    };
    assert!(record(0, TaskOutcome::Cancelled).replaces.is_empty());
    assert_eq!(record(2, TaskOutcome::Cancelled).replaces, ["t1"]);
    assert_eq!(record(3, TaskOutcome::Merged).replaces, ["t2", "t3"]);
}
