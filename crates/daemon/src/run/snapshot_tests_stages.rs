//! Milestone 9.1 task M9.1.20: the snapshot's stages, slots, and each task's stage,
//! origin, fix, last tier record and test-weakening signals (decision 55). Pure.

use proto::{
    Finding, FullState, RunState, Severity, SignalInfo, TaskOrigin, TaskState, TierInfo, Verdict,
};

use super::*;
use crate::run::model::{FixOf, ReviewRecord, StageLayout, StageRecord, TierRecord};
use crate::run::orch::test_support::*;
use crate::run::tiers::Signal;

fn tier_record(tier: u8, ok: bool) -> TierRecord {
    TierRecord {
        tier,
        affected: "2 modules (a, b)".into(),
        steps: 2,
        cached: 1,
        ok,
        secs: 130,
        flaky: vec!["a::x".into()],
        failing: vec![],
        at: 5,
        commit: String::new(),
    }
}

fn finding(severity: Severity, text: &str) -> Finding {
    Finding {
        severity,
        file: None,
        line: None,
        input: None,
        text: text.into(),
    }
}

/// A `Multi` run with stage 1 created and green and stage 2 not created yet: `t1`
/// reviewed with three signals, `t2` in stage 2, and the bisect fix task `fix1`.
fn staged_run() -> Run {
    let mut run = run_with(&[
        task_toml("t1", "S", "[\"crates/a/**\"]", ""),
        task_toml("t2", "S", "[\"crates/b/**\"]", "stage = 2"),
    ]);
    run.state = RunState::Running;
    run.stage_layout = StageLayout::Multi;
    run.test_slots = 8;
    let h1 = "1".repeat(40);
    let mut s1 = StageRecord::new(1, run.stage_branch(1), &run.base_sha, Default::default(), 0);
    s1.head = h1.clone();
    s1.full.green_at = Some(h1.clone());
    run.stages = vec![s1];

    let head = "c".repeat(40);
    let t1 = task_mut(&mut run, "t1");
    t1.state = TaskState::Merged;
    t1.head = Some(head.clone());
    let check = |tier: Option<TierRecord>| crate::run::model::CheckRecord {
        at: 5,
        ok: true,
        code: Some(0),
        timed_out: false,
        tail: String::new(),
        secs: 130,
        on_candidate: tier.as_ref().is_some_and(|t| t.tier == 2),
        summary: None,
        summary_source: None,
        tier,
    };
    t1.checks = vec![check(Some(tier_record(1, true))), check(None)];
    t1.signals = vec![
        Signal::DeletedTestFile {
            path: "tests/old.rs".into(),
        },
        Signal::SkipMarker {
            path: "src/a.rs".into(),
            line: 12,
            marker: "#[ignore]".into(),
        },
        Signal::AssertionLoss {
            path: "src/a.rs".into(),
            line: 40,
            removed: 3,
            added: 1,
        },
    ];
    t1.reviews = vec![ReviewRecord {
        round: 1,
        route: route(),
        base: run_base(),
        head,
        verdict: Some(Verdict::Changes),
        summary: String::new(),
        findings: vec![
            finding(
                Severity::Minor,
                "W1 accepted: the tests moved to tests/new.rs",
            ),
            finding(Severity::Critical, "W2 hides a real failure"),
            finding(
                Severity::Important,
                "W3 (src/a.rs:40) was not justified by the review",
            ),
        ],
    }];
    let mut fix = run.tasks[1].clone();
    fix.spec.id = "fix1".into();
    fix.origin = TaskOrigin::Bisect;
    fix.fixes = Some(FixOf::Bisect {
        culprit: "t2".into(),
        stage: 2,
        tests: vec!["b::works".into()],
    });
    run.tasks.push(fix);
    run
}

fn run_base() -> String {
    "b".repeat(40)
}

fn published(run: Run) -> RunInfo {
    let mut state = EngineState::default();
    state.runs.insert(run.id.clone(), run);
    snapshot(&state, 10).runs.remove(0)
}

#[test]
fn snapshot_has_stages_and_task_stage_and_origin() {
    let run = staged_run();
    let id = run.id.clone();
    let info = published(run);
    assert_eq!(info.test_slots, 8);
    let stages: Vec<(u16, String, Option<String>, FullState)> = info
        .stages
        .iter()
        .map(|s| (s.n, s.branch.clone(), s.head.clone(), s.full.state))
        .collect();
    assert_eq!(
        stages,
        [
            (
                1,
                format!("anthrex/{id}/stage-1"),
                Some("1".repeat(40)),
                FullState::Green
            ),
            (2, format!("anthrex/{id}/stage-2"), None, FullState::None),
        ]
    );
    assert_eq!(info.stages[1].fix_tasks, ["fix1"]);
    assert_eq!((info.stages[1].tasks, info.stages[1].merged), (2, 0));

    let task = |id: &str| info.tasks.iter().find(|t| t.id == id).unwrap();
    let t1 = task("t1");
    assert_eq!(
        (t1.stage, t1.origin, t1.fixes.clone()),
        (1, TaskOrigin::Plan, None)
    );
    // The last tier record, though a later check had none.
    let tier = TierInfo {
        tier: 1,
        affected: "2 modules (a, b)".into(),
        steps: 2,
        cached: 1,
        ok: true,
        secs: 130,
        flaky: vec!["a::x".into()],
    };
    assert_eq!(t1.tier, Some(tier.clone()));
    assert_eq!(t1.last_check.as_ref().unwrap().tier, None);
    let signal = |id: &str, kind: &str, line: Option<u32>, text: &str, answered: &str| SignalInfo {
        id: id.into(),
        kind: kind.into(),
        path: if kind == "deleted_test_file" {
            "tests/old.rs".into()
        } else {
            "src/a.rs".into()
        },
        line,
        text: text.into(),
        answered: Some(answered.into()),
    };
    assert_eq!(
        t1.weakening,
        [
            signal(
                "W1",
                "deleted_test_file",
                None,
                "tests/old.rs: deleted test file",
                "accepted: the tests moved to tests/new.rs"
            ),
            signal(
                "W2",
                "skip_marker",
                Some(12),
                "src/a.rs:12: added skip marker #[ignore]",
                "critical"
            ),
            signal(
                "W3",
                "assertion_loss",
                Some(40),
                "src/a.rs:40: 3 assertion lines removed, 1 added",
                "engine finding"
            ),
        ]
    );
    let fix = task("fix1");
    assert_eq!(
        (fix.stage, fix.origin, fix.fixes.as_deref()),
        (2, TaskOrigin::Bisect, Some("bisect of t2"))
    );
    assert_eq!(task("t2").stage, 2);
}

/// Signals are unanswered until a review of the claimed head has a verdict.
#[test]
fn signals_are_unanswered_until_the_claim_is_reviewed() {
    let mut run = staged_run();
    task_mut(&mut run, "t1").head = Some("d".repeat(40));
    let info = published(run);
    let t1 = info.tasks.iter().find(|t| t.id == "t1").unwrap();
    assert_eq!(t1.weakening.len(), 3);
    assert!(t1.weakening.iter().all(|s| s.answered.is_none()));
}

/// A check that ran a tier carries it.
#[test]
fn a_tier_check_carries_its_tier() {
    let mut run = staged_run();
    let t1 = task_mut(&mut run, "t1");
    t1.checks.pop();
    t1.checks[0].tier = Some(tier_record(2, false));
    let info = published(run);
    let t1 = info.tasks.iter().find(|t| t.id == "t1").unwrap();
    let tier = t1.last_check.as_ref().unwrap().tier.clone().unwrap();
    assert_eq!((tier.tier, tier.ok), (2, false));
    assert_eq!(t1.tier, Some(tier));
}
