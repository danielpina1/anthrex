//! Milestone 9.5 task M9.5.21 (decision 30): `REPORT.md`'s race and pair lines per
//! task, and the run's `## Concurrency` and `tuning:` lines.

use proto::{Effort, LaneState, PairPhase, Route, Runtime, Strength};

use super::render;
use crate::run::model::{LogEntry, Pair, Run, RuntimeConcurrency};
use crate::run::test_support::{PROFILE, plan_with, race_of, run_ok, task_toml};

fn route(runtime: Runtime, model: &str) -> Route {
    Route {
        runtime,
        model: model.into(),
        strength: Strength::Standard,
        effort: Effort::Medium,
    }
}

fn run() -> Run {
    let tasks: Vec<String> = ["t1", "t2", "t3"]
        .iter()
        .map(|id| task_toml(id, "M", &format!("[\"crates/{id}/**\"]"), ""))
        .collect();
    run_ok(&plan_with(PROFILE, &tasks))
}

/// The lines of `report` from the one starting with `first`, up to the next blank line.
fn block<'a>(report: &'a str, first: &str) -> Vec<&'a str> {
    let lines: Vec<&str> = report.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.starts_with(first))
        .unwrap_or_else(|| panic!("no line starting {first:?} in:\n{report}"));
    lines[at..]
        .iter()
        .take_while(|l| !l.is_empty())
        .copied()
        .collect()
}

#[test]
fn report_has_race_pair_concurrency_and_tuning() {
    let mut run = run();
    // t1: lane a won; lane b lost, salvaged after its stale locks were removed.
    let t1 = &mut run.tasks[0];
    let mut race = race_of(t1, [LaneState::Won, LaneState::Lost]);
    race.lanes[0].route = route(Runtime::Claude, "claude-opus-5-5");
    race.lanes[1].route = route(Runtime::Codex, "gpt-6.1-sol");
    race.lanes[1].reason = Some("racer a won".into());
    race.lanes[1].salvage_ref = Some("refs/anthrex/salvage/r1/t1/2".into());
    race.lanes[1].cleared_locks = vec!["index.lock".into(), "HEAD.lock".into()];
    t1.race = Some(race);
    // t2: lane a went out and its racer did not exit (kept); lane b was adopted.
    let t2 = &mut run.tasks[1];
    let mut race = race_of(t2, [LaneState::Out, LaneState::Adopted]);
    race.lanes[0].route = route(Runtime::Claude, "claude-opus-5-5");
    race.lanes[1].route = route(Runtime::Codex, "gpt-6.1-sol");
    race.lanes[0].reason = Some("the check failed twice".into());
    race.lanes[0].salvage_ref = Some("refs/anthrex/salvage/r1/t2/1".into());
    race.lanes[0].kept = true;
    t2.race = Some(race);
    // t3: paired; the red check failed twice before the implementer started.
    run.tasks[2].pair = Some(Pair {
        phase: PairPhase::Implementing,
        writer_route: route(Runtime::Codex, "gpt-6.1-sol"),
        test: Some("t_feat".into()),
        red: Some("a1b2c3d".repeat(5) + "aaaaa"),
        red_checked: Some(true),
        writer_failures: 2,
        writer_sessions: 1,
        escalated_from: None,
        writer_signals: Vec::new(),
        writer_signals_more: 0,
    });
    // Two cap changes on Codex: halved twice from 3 to 1.
    run.limits.adaptive_concurrency = true;
    run.rate_limits.insert("codex".into(), 2);
    let mut codex = RuntimeConcurrency::new(run.limits.max_writers);
    (codex.cap, codex.halvings) = (1, 2);
    run.concurrency.insert("codex".into(), codex);
    // The start's tuning lines (decision 12), one of them `configured`.
    let start = [
        "tuning: budget S 40 calls 15m configured (refit would be 55 calls 18m)",
        "tuning: path weights S 550s, M 1650s (derived), hub 1650s (derived)",
    ];
    let mut log: Vec<LogEntry> = start
        .iter()
        .map(|text| LogEntry {
            at: 1_000,
            text: text.to_string(),
        })
        .collect();
    log.append(&mut run.log);
    run.log = log;

    let report = render(&run, 2_000);

    assert_eq!(
        block(&report, "Race: "),
        [
            "Race: racer a won",
            "- racer a: claude claude-opus-5-5 (standard/medium); won",
            "- racer b: codex gpt-6.1-sol (standard/medium); lost; salvaged refs/anthrex/salvage/r1/t1/2; removed stale index.lock, HEAD.lock; reason: racer a won",
        ]
    );
    let t2 = &report[report.find("## t2:").unwrap()..];
    assert_eq!(
        block(t2, "Race: "),
        [
            "Race: racer b adopted after racer a went out",
            "- racer a: claude claude-opus-5-5 (standard/medium); out; salvaged refs/anthrex/salvage/r1/t2/1; checkout kept; reason: the check failed twice",
            "- racer b: codex gpt-6.1-sol (standard/medium); adopted",
        ]
    );
    assert!(
        report.contains(
            "\nPair: test writer codex gpt-6.1-sol (standard/medium); test t_feat; red a1b2c3d, fails at red; writer failures 2; implementing\n"
        ),
        "{report}"
    );
    assert!(
        report.contains(
            "\n## Concurrency\n\n- codex: writers 1 of 3 (2 rate limits, 2 halvings, 0 recoveries)\n"
        ),
        "{report}"
    );
    let tuning = &report[report.find("\n## Tuning\n\n").expect("a tuning section")..];
    assert!(
        tuning.starts_with(&format!(
            "\n## Tuning\n\n- {}\n- {}\n\n",
            start[0], start[1]
        )),
        "{report}"
    );
    // The sections come before the log, which still carries the lines.
    assert!(report.find("## Tuning") < report.find("## Log"));
    assert!(report.find("## Concurrency") < report.find("## Log"));
}

/// Pinning: a task that neither raced nor paired, and a run with no tuning lines, add
/// nothing.
#[test]
fn a_plain_run_has_no_race_pair_or_tuning_lines() {
    let report = render(&run(), 2_000);
    for absent in ["\nRace: ", "\nPair: ", "## Tuning", "## Concurrency"] {
        assert!(!report.contains(absent), "{absent:?} in:\n{report}");
    }
}

/// A race still running, one whose lanes both went out, and a pair still writing.
#[test]
fn a_running_race_an_ended_race_and_a_writing_pair() {
    let mut run = run();
    let t1 = &mut run.tasks[0];
    t1.race = Some(race_of(t1, [LaneState::Working, LaneState::Review]));
    let t2 = &mut run.tasks[1];
    let mut race = race_of(t2, [LaneState::Out, LaneState::Out]);
    race.ended = true;
    t2.race = Some(race);
    run.tasks[2].failures = 1;
    run.tasks[2].pair = Some(Pair {
        phase: PairPhase::Writing,
        writer_route: route(Runtime::Codex, "gpt-6.1-sol"),
        test: None,
        red: None,
        red_checked: None,
        writer_failures: 0,
        writer_sessions: 1,
        escalated_from: None,
        writer_signals: Vec::new(),
        writer_signals_more: 0,
    });
    let report = render(&run, 2_000);
    assert!(report.contains("\nRace: racing\n"), "{report}");
    assert!(
        report.contains("\nRace: no winner: both racers went out\n"),
        "{report}"
    );
    assert!(
        report.contains(
            "\nPair: test writer codex gpt-6.1-sol (standard/medium); writer failures 1; writing the test\n"
        ),
        "{report}"
    );
}
