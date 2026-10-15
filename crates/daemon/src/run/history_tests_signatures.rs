//! Task M9.15.2: decision 6's signatures and `owns` on a task record.

use proto::{SigGate, TaskOutcome};

use super::fixtures::{check, passed_proof, run_of};
use crate::run::history::task_record;
use crate::run::model::TierRecord;

fn failed(tail: &str) -> crate::run::model::CheckRecord {
    let mut record = check(false, false);
    record.tail = tail.into();
    record
}

#[test]
fn task_record_carries_deduped_signatures() {
    let mut run = run_of(&["t1", "t2"]);
    let task = &mut run.tasks[0];
    // The same failure twice, its durations differing; a passing check in between.
    task.checks = vec![
        failed("running 1 test\ntest a::b ... FAILED in 0.42s\n"),
        check(true, false),
        failed("running 1 test\ntest a::b ... FAILED in 13ms\n"),
    ];
    let mut tier2 = failed("error: the tier-2 suite failed");
    tier2.tier = Some(TierRecord {
        tier: 2,
        affected: String::new(),
        steps: 1,
        cached: 0,
        ok: false,
        secs: 9,
        flaky: Vec::new(),
        failing: Vec::new(),
        at: 1_300,
        commit: String::new(),
    });
    task.checks.push(tier2);
    let mut proof = passed_proof(1_100);
    proof.head_passed = false;
    proof.head_tail = "thread 'x' panicked: assertion failed".into();
    task.proofs = vec![passed_proof(1_000), proof];
    let task = run.tasks[0].clone();
    let record = task_record(&run, &task, TaskOutcome::Merged, 2_000);
    let gates: Vec<SigGate> = record.signatures.iter().map(|s| s.gate).collect();
    assert_eq!(
        gates,
        [SigGate::Check, SigGate::Tier2, SigGate::Proof],
        "{:?}",
        record.signatures
    );
    assert_eq!(
        record.signatures[0].headline,
        "test a::b ... FAILED in <dur>"
    );
    assert_eq!(record.owns, task.spec.owns);
    assert!(!record.owns.is_empty());
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["signatures"][1]["gate"], "tier2", "{json}");

    // Only passing checks: both lists empty, neither key written.
    let mut clean = run.tasks[1].clone();
    clean.spec.owns.clear();
    clean.checks = vec![check(true, false)];
    let record = task_record(&run, &clean, TaskOutcome::Merged, 2_000);
    assert!(record.signatures.is_empty());
    assert!(record.owns.is_empty());
    let json = serde_json::to_value(&record).unwrap();
    assert!(json.get("signatures").is_none(), "{json}");
    assert!(json.get("owns").is_none(), "{json}");
}

/// Ruling C21: a proof whose test was not red fails as the history tally counts it, and
/// its signature comes from the red tail; a red-only check is no proof.
#[test]
fn a_proof_that_was_not_red_signs_its_red_tail() {
    let mut run = run_of(&["t1"]);
    let mut not_red = passed_proof(1_000);
    not_red.red_failed = false;
    not_red.red_tail = "error: expected the test to fail at red".into();
    let mut red_only = passed_proof(1_100);
    red_only.red_only = true;
    red_only.head_passed = false;
    red_only.head_tail = "error: a red-only run".into();
    run.tasks[0].proofs = vec![red_only, not_red];
    let task = run.tasks[0].clone();
    let record = task_record(&run, &task, TaskOutcome::Merged, 2_000);
    let got: Vec<_> = (record.signatures.iter())
        .map(|s| (s.gate, s.headline.as_str()))
        .collect();
    assert_eq!(
        got,
        [(SigGate::Proof, "error: expected the test to fail at red")]
    );
}

/// Decision 6: at most `SIGS_PER_TASK`, the first kept.
#[test]
fn a_task_keeps_at_most_eight_signatures() {
    let mut run = run_of(&["t1"]);
    run.tasks[0].checks = (0..12)
        .map(|n| failed(&format!("error: failure number {}", "x".repeat(n + 1))))
        .collect();
    let task = run.tasks[0].clone();
    let record = task_record(&run, &task, TaskOutcome::Merged, 2_000);
    assert_eq!(
        record.signatures.len(),
        crate::lessons::signature::SIGS_PER_TASK
    );
    assert_eq!(record.signatures[0].headline, "error: failure number x");
}

/// The controller's ruling (task M9.15.9): a failing check whose output carries a token
/// is stored in the task record redacted, and the same failure keeps its fingerprint.
#[test]
fn a_stored_signature_headline_is_redacted() {
    let token = format!("ghp_{}", "FAKE".repeat(9));
    let record_of = |tail: String| {
        let mut run = run_of(&["t1"]);
        run.tasks[0].checks = vec![failed(&tail)];
        let task = run.tasks[0].clone();
        task_record(&run, &task, TaskOutcome::Merged, 2_000)
    };
    let tail = format!("error: push rejected for token {token}\n");
    let record = record_of(tail.clone());
    let json = serde_json::to_string(&record).unwrap();
    assert!(!json.contains("ghp_"), "{json}");
    assert!(!json.contains("FAKEFAKE"), "{json}");
    assert_eq!(
        record.signatures[0].headline,
        "error: push rejected for token [redacted token]"
    );
    assert_eq!(record_of(tail).signatures, record.signatures);
}
