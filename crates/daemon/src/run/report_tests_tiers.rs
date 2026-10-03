//! Milestone 9.1 task M9.1.20: the report's "Testing" section and each task's last
//! tier-1 line and signals (decision 59). Pure.

use proto::{Finding, RunState, Severity, TaskOrigin, TaskState, Verdict};

use super::*;
use crate::run::model::{
    BisectEnd, CheckRecord, FixOf, ReviewRecord, StageLayout, StageRecord, TierRecord,
};
use crate::run::orch::test_support::{route, run_with, task_mut, task_toml};
use crate::run::tiers::Signal;

fn tier_record(tier: u8, ok: bool, flaky: &[&str], failing: &[&str], commit: &str) -> TierRecord {
    TierRecord {
        tier,
        affected: "2 modules (a, b)".into(),
        steps: 2,
        cached: 1,
        ok,
        secs: 130,
        flaky: flaky.iter().map(|s| s.to_string()).collect(),
        failing: failing.iter().map(|s| s.to_string()).collect(),
        at: 5,
        commit: commit.into(),
    }
}

fn check(tier: TierRecord) -> CheckRecord {
    CheckRecord {
        at: 5,
        ok: tier.ok,
        code: Some(0),
        timed_out: false,
        tail: String::new(),
        secs: tier.secs,
        on_candidate: false,
        summary: None,
        summary_source: None,
        tier: Some(tier),
        lane: None,
    }
}

/// A `Multi` run: stage 1 created, green, with two ended bisects; stage 2 not created;
/// `t1` with a tier-1 record and two answered signals; the bisect fix task `fix1`.
fn staged_run() -> Run {
    let mut run = run_with(&[
        task_toml("t1", "S", "[\"crates/a/**\"]", ""),
        task_toml("t2", "S", "[\"crates/b/**\"]", "stage = 2"),
    ]);
    run.state = RunState::Running;
    run.stage_layout = StageLayout::Multi;
    run.profile.tiers.full_shards = 2;
    run.graph_note = Some("module graph unknown: graph.sh exited 1; every tier runs check".into());
    let h1 = "1".repeat(40);
    let mut s1 = StageRecord::new(1, run.stage_branch(1), &run.base_sha, Default::default(), 0);
    s1.head = h1.clone();
    s1.full.green_at = Some(h1.clone());
    let mut last = tier_record(3, true, &["a::flaky"], &[], &h1);
    last.secs = 2472;
    s1.full.last = Some(last);
    s1.full.ended = vec![
        BisectEnd {
            head: "2".repeat(40),
            range: 6,
            probes: 4,
            culprit: Some("t4".into()),
            fix_task: Some("fix1".into()),
            reason: None,
            at: 3,
        },
        BisectEnd {
            head: "3".repeat(40),
            range: 2,
            probes: 1,
            culprit: None,
            fix_task: None,
            reason: Some("the failing tests already fail at 0000000".into()),
            at: 4,
        },
    ];
    run.stages = vec![s1];

    let head = "c".repeat(40);
    let t1 = task_mut(&mut run, "t1");
    t1.state = TaskState::Merged;
    t1.head = Some(head.clone());
    t1.checks = vec![
        check(tier_record(1, false, &[], &["a::works"], "")),
        check(tier_record(1, true, &["a::x"], &[], "")),
        check(tier_record(2, true, &[], &[], "")),
    ];
    t1.signals = vec![
        Signal::DeletedTestFile {
            path: "tests/old.rs".into(),
        },
        Signal::SkipMarker {
            path: "src/a.rs".into(),
            line: 12,
            marker: "#[ignore]".into(),
        },
    ];
    t1.reviews = vec![ReviewRecord {
        round: 1,
        route: route(),
        base: "b".repeat(40),
        head,
        verdict: Some(Verdict::Approve),
        summary: String::new(),
        findings: vec![Finding {
            severity: Severity::Minor,
            file: None,
            line: None,
            input: None,
            text: "W1 accepted: the tests moved to tests/new.rs".into(),
        }],
        lane: None,
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

/// The part of `text` from the line `from` up to (not including) the line `to`.
fn between<'a>(text: &'a str, from: &str, to: &str) -> &'a str {
    let start = text
        .find(from)
        .unwrap_or_else(|| panic!("no {from:?} in:\n{text}"));
    let end = text[start..].find(to).map_or(text.len(), |e| start + e);
    &text[start..end]
}

#[test]
fn report_has_the_tiers_section() {
    let run = staged_run();
    let id = run.id.clone();
    let text = render(&run, 10);
    assert_eq!(
        between(&text, "## Testing\n", "\n## Log\n"),
        format!(
            "## Testing\n\
             \n\
             module graph unknown: graph.sh exited 1; every tier runs check\n\
             \n\
             ### Stage 1\n\
             \n\
             Branch: anthrex/{id}/stage-1 at 1111111\n\
             Tier 3: green at 1111111, 2472s, 2 shards, flaky: a::flaky\n\
             Bisect of 2222222: 6 merges, 4 probes, culprit t4, fix task fix1\n\
             Bisect of 3333333: 2 merges, 1 probe, no culprit: the failing tests already fail at 0000000\n\
             \n\
             ### Stage 2\n\
             \n\
             Branch: anthrex/{id}/stage-2 (not created)\n\
             Tier 3: none\n\
             \n\
             ### Fix tasks\n\
             \n\
             - fix1 (bisect of t2): pending\n"
        )
    );
    // Each task's section: its last tier-1 line, and its signals with their answers.
    let t1 = between(&text, "## t1: ", "\n## ");
    assert!(
        t1.contains("Tier 1: 2 modules (a, b), 2 steps, 1 cached, 130s, green, flaky: a::x\n"),
        "{t1}"
    );
    assert!(
        t1.contains(
            "Test changes:\n\
             - W1 tests/old.rs: deleted test file: accepted: the tests moved to tests/new.rs\n\
             - W2 src/a.rs:12: added skip marker #[ignore]: not answered\n\n"
        ),
        "{t1}"
    );
}

/// Pinning: an untiered one-stage run with no fix task has no "Testing" section, and
/// its tasks no tier or signal lines: its report is milestone 9's.
#[test]
fn an_untiered_run_has_no_testing_section() {
    let mut run = run_with(&[task_toml("t1", "S", "[\"crates/a/**\"]", "")]);
    run.state = RunState::Running;
    let text = render(&run, 10);
    assert!(!text.contains("## Testing"), "{text}");
    assert!(!text.contains("Tier 1:"), "{text}");
    assert!(!text.contains("Test changes:"), "{text}");
}

/// Decision 52: a stage whose propagate from the stage below is red says so, with the
/// lower stage's head it was red on.
#[test]
fn a_red_propagate_has_its_line() {
    let mut run = staged_run();
    let mut s2 = StageRecord::new(2, run.stage_branch(2), &run.base_sha, Default::default(), 0);
    s2.head = "4".repeat(40);
    s2.propagate_red = Some("1".repeat(40));
    run.stages.push(s2);
    let text = render(&run, 10);
    assert!(
        between(&text, "### Stage 2\n", "\n### Fix tasks")
            .contains("Tier 3: none\nPropagate of stage 1 at 1111111: red\n"),
        "{text}"
    );
}

/// Ruling C-27 (M-2): with the result cache on, the section says what a cached result
/// assumes; with it off (an `"unknown"` toolchain) it does not.
#[test]
fn a_cached_run_says_what_its_cache_assumes() {
    const LINE: &str = "Cached results assume tests read only the checkout.\n";
    let mut run = staged_run();
    run.repo_dir = "/tmp/data/repos/r-1".into();
    run.toolchain = Some("rustc 1.90.0".into());
    let text = render(&run, 10);
    let testing = between(&text, "## Testing\n", "\n## Log\n");
    assert!(
        testing.starts_with(&format!(
            "## Testing\n\nmodule graph unknown: graph.sh exited 1; every tier runs check\n\n{LINE}\n### Stage 1\n"
        )),
        "{testing}"
    );
    run.toolchain = Some("unknown".into());
    assert!(!render(&run, 10).contains(LINE));
}
