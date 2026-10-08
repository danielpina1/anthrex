//! Task M9.5.8's history fixtures: the records the refit tests read, built by the
//! brief's rules and written to `daemon/tests/fixtures/history/` (compared byte for byte
//! by `fixtures_match_their_rules`).

use std::path::{Path, PathBuf};

use proto::{
    AgentRole, DiffStats, Effort, GateCounts, GateTally, HISTORY_VERSION, HistoryLine, PhaseSecs,
    Route, RoutingCandidate, RoutingDecision, RoutingInput, RunPath, Runtime, SeverityTally, Size,
    Strength, TaskKind, TaskOrigin, TaskOutcome, TaskRecord, TestMode, TokenUsage,
};

/// `refit_budget`'s `now`, and the CLI block's (review ruling I5): 2026-09-27 09:06:40
/// UTC.
pub(crate) const NOW: u64 = 1_790_500_000;

/// One fixture record (the brief's common rule): a merged plan code task of size S
/// with `tool_calls`, `w` working seconds and `lines` added lines. Its one worker took
/// the class default (whole-branch review B, I2: only such a record is a route sample).
pub(crate) fn record(
    run: &str,
    task: u32,
    at: u64,
    tool_calls: u32,
    w: u64,
    lines: u32,
) -> TaskRecord {
    let route = Route {
        runtime: Runtime::Claude,
        model: "claude-sonnet-5".into(),
        strength: Strength::Standard,
        effort: Effort::LOW,
    };
    TaskRecord {
        v: HISTORY_VERSION,
        record_id: format!("{run}/t{task}"),
        at,
        run_id: run.into(),
        task_id: format!("t{task}"),
        path: Some(RunPath::Plan),
        kind: TaskKind::Code,
        hub: false,
        test_mode: TestMode::Tdd,
        planned_size: Size::S,
        final_size: Size::S,
        size_check: None,
        routing_decisions: vec![worker_decision(at, &route)],
        route,
        review_routes: Vec::new(),
        outcome: TaskOutcome::Merged,
        block: None,
        diff: Some(DiffStats {
            files: 1,
            hunks: 1,
            added: lines,
            removed: 0,
        }),
        tool_calls,
        worker_usage: TokenUsage::default(),
        reviewer_usage: TokenUsage::default(),
        decider_usage: TokenUsage::default(),
        phases: PhaseSecs {
            queued: 5,
            preparing: 20,
            working: w,
            proof: 0,
            check: 40,
            review: 60,
            merge: 10,
            blocked: 0,
        },
        wall_secs: 0,
        gates: GateTally::default(),
        severities: SeverityTally::default(),
        bounces: GateCounts::default(),
        failures: 0,
        stalls: 0,
        budget_exceeded: 0,
        conflicts: 0,
        max_rung: 0,
        sessions: 1,
        done_signal: None,
        merge_commit: None,
        stage: 1,
        origin: TaskOrigin::Plan,
        pattern: None,
        race_winner: None,
        race_adopted: false,
        writer_failures: 0,
        round: 1,
    }
}

/// The class default's routing decision of a record's first worker at `route`.
fn worker_decision(at: u64, route: &Route) -> RoutingDecision {
    RoutingDecision {
        seq: 1,
        at,
        role: AgentRole::Worker,
        session: 1,
        round: None,
        lane: None,
        trigger: "initial".into(),
        source: "class_default".into(),
        policy_version: "m8a-worker-v1".into(),
        pick_policy: None,
        input: RoutingInput {
            title: String::new(),
            brief: String::new(),
            acceptance: Vec::new(),
            owns: Vec::new(),
            kind: TaskKind::Code,
            size: Size::S,
            hub: false,
            interface_change: false,
            test_mode: TestMode::Tdd,
            languages: Vec::new(),
        },
        chosen: route.clone(),
        selected_index: 0,
        candidates: vec![RoutingCandidate {
            route: route.clone(),
            skipped_reason: None,
        }],
    }
}

/// `record` made size M (`hub` or not), with M's route.
pub(crate) fn sized_m(mut r: TaskRecord, hub: bool) -> TaskRecord {
    r.planned_size = Size::M;
    r.final_size = Size::M;
    r.hub = hub;
    r.route.effort = Effort::MEDIUM;
    for d in &mut r.routing_decisions {
        d.input.size = Size::M;
        d.input.hub = hub;
        d.chosen.effort = Effort::MEDIUM;
        d.candidates[0].route.effort = Effort::MEDIUM;
    }
    r
}

pub(crate) fn task(r: TaskRecord) -> HistoryLine {
    HistoryLine::Task(r)
}

/// The 34 S records of `refit.jsonl`.
fn refit_s() -> Vec<HistoryLine> {
    (0..34u32)
        .map(|i| {
            let run = format!("r{}", 1 + i / 10);
            let at = 1_790_000_000 + 60 * u64::from(i);
            let mut r = record(&run, i, at, 6 + i, 260 + 10 * u64::from(i), 3 + i);
            r.max_rung = if i < 14 { 2 } else { 0 };
            task(r)
        })
        .collect()
}

/// Each fixture's records, built by its rule (the brief's "The fixtures"). Rules the
/// brief leaves open: every record has `sessions = 1`; the M records are of runs `m1`
/// and `m2` (`1 + j / 10`, as S's), not one run `m1`, whose cap of 10 per run and
/// round would show `10/30` against the CLI block's `12/30`; the hub records follow M's rule
/// with `at = 1_790_150_000 + 60 × j`; `quality.jsonl` numbers its tasks within each run
/// (`q2/t3` exists for the bisect culprit) at `1_790_300_000 + 60 × i`, with 20 tool
/// calls, 400 working seconds and 10 lines each.
pub(crate) fn fixture_records(name: &str) -> Vec<HistoryLine> {
    match name {
        "refit" => {
            let mut lines = refit_s();
            lines.extend((0..12u32).map(|j| {
                let run = format!("m{}", 1 + j / 10);
                let at = 1_790_100_000 + 60 * u64::from(j);
                task(sized_m(record(&run, j, at, 60 + j, 1500, 50 + j), false))
            }));
            lines.extend((0..3u32).map(|j| {
                let at = 1_790_150_000 + 60 * u64::from(j);
                task(sized_m(record("h1", j, at, 60 + j, 1500, 50 + j), true))
            }));
            lines
        }
        "too-few" => refit_s().into_iter().take(29).collect(),
        "outlier-run" => {
            let mut lines = refit_s();
            lines.extend((0..25u32).map(|j| {
                let at = 1_790_200_000 + 60 * u64::from(j);
                task(record("r9", j, at, 900, 9000, 900))
            }));
            lines
        }
        "quality" => (0..30u32)
            .map(|i| {
                let run = format!("q{}", 1 + i / 10);
                let at = 1_790_300_000 + 60 * u64::from(i);
                task(record(&run, i % 10, at, 20, 400, 10))
            })
            .collect(),
        other => panic!("no fixture {other}"),
    }
}

pub(crate) const FIXTURES: [&str; 4] = ["refit", "too-few", "outlier-run", "quality"];

pub(crate) fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/history/{name}.jsonl"))
}

pub(crate) fn jsonl(lines: &[HistoryLine]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(&serde_json::to_string(line).unwrap());
        out.push('\n');
    }
    out
}
