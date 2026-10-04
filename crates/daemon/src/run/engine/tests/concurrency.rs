//! Milestone 9.5 task M9.5.13: adaptive concurrency (decision 16; rulings RC-1, RC-3).
//! A rate-limit event in a task session halves that runtime's writer cap inside the
//! run, never below one; a quiet `recover_after_secs` adds one back.

use proto::{Effort, GateCounts, LaneState, RaceLane, Route, Runtime, Spend, Strength, TaskState};

use super::fixture::*;
use super::gates_review::reviewed;
use super::kinds_research::researching;
use super::orch::{ORCH, launched};
use super::run_scouts::scout;
use crate::headless::FailureKind;
use crate::run::engine::concurrency::{cap, on_rate_limit, on_tick, writers_busy_on};
use crate::run::engine::schedule::writers_busy;
use crate::run::engine::{AgentSignal, Effect, EngineState, EventKind, OpResult, TurnOutcome};
use crate::run::model::{Lane, Race, Run};
use crate::run::snapshot::snapshot;

const CODEX: &str = "[task.route]\nruntime = \"codex\"\nmodel = \"\"";

/// The 9.3 `run.json` milestone 9.5 task 2 captured with the base's code.
const M93_RUN: &str = include_str!("../../../../tests/fixtures/run/m93-run.json");

/// The default config with `[orchestrator.tuning]`'s two timings, and no stall or
/// minutes budget firing during a test's long quiet spans.
fn config(hold_secs: u64, recover_mins: u64) -> config::Orchestrator {
    let mut c = config::Orchestrator::default();
    c.tuning.table.halve_hold_secs = hold_secs;
    c.tuning.table.recover_after_mins = recover_mins;
    c.stall_after_secs = 100_000;
    c.budget_s.minutes = 100_000;
    c
}

/// A running run (`--yes`) of `tasks` at `max_writers`, its writers launched.
fn running(max_writers: u8, tasks: &[String], config: config::Orchestrator) -> Fixture {
    let plan = plan_with(
        &profile_with(&format!("max_writers = {max_writers}")),
        tasks,
    );
    let mut fx = Fixture::with_config(&plan, config);
    fx.ready(true);
    fx.launch_all();
    fx
}

/// `id`'s latest window.
fn window(fx: &Fixture, id: &str) -> u32 {
    let round = fx.task(id).rounds.last().expect("a round");
    round.window_id.expect("a window")
}

fn retry() -> AgentSignal {
    AgentSignal::ApiRetry {
        error: "rate_limit".into(),
        delay_ms: 1_000,
    }
}

/// One rate-limit event from `window` at `now`: a retry streak the stream then leaves.
fn event(fx: &mut Fixture, window: u32, now: u64) -> Vec<Effect> {
    let mut effects = fx.send(
        now,
        EventKind::Signal {
            window_id: window,
            signal: retry(),
        },
    );
    effects.extend(fx.send(
        now,
        EventKind::Signal {
            window_id: window,
            signal: AgentSignal::Activity,
        },
    ));
    effects
}

/// One rate-limit event from task `id`'s session, a second after the last step.
fn limited(fx: &mut Fixture, id: &str) -> Vec<Effect> {
    let (w, now) = (window(fx, id), fx.now + 1);
    event(fx, w, now)
}

fn caps(fx: &Fixture) -> std::collections::BTreeMap<String, u8> {
    snapshot(&fx.state, fx.now).runs[0].writer_caps.clone()
}

fn attention(fx: &Fixture) -> Vec<String> {
    snapshot(&fx.state, fx.now).runs[0].attention.clone()
}

fn logged(fx: &Fixture, needle: &str) -> Vec<String> {
    (fx.run().log.iter())
        .filter(|e| e.text.contains(needle))
        .map(|e| e.text.clone())
        .collect()
}

fn codex(id: &str, module: &str) -> String {
    task(id, "S", module, CODEX)
}

#[test]
fn a_rate_limit_halves_the_runtimes_cap_never_below_one() {
    let mut fx = running(8, &[codex("c1", "a")], config(60, 10));
    let w = window(&fx, "c1");
    let t0 = fx.now + 1;
    let mut seen = Vec::new();
    for k in 0..4 {
        event(&mut fx, w, t0 + k * 60);
        seen.push(cap(fx.run(), Runtime::Codex));
        assert_eq!(
            cap(fx.run(), Runtime::Claude),
            8,
            "claude stays at max_writers"
        );
    }
    assert_eq!(seen, vec![4, 2, 1, 1]);
    assert_eq!(caps(&fx), [("codex".to_string(), 1)].into());
    assert_eq!(fx.run().rate_limits.get("codex"), Some(&4));
    let entry = fx.run().concurrency["codex"];
    assert_eq!((entry.halvings, entry.recoveries), (3, 0), "{entry:?}");
    assert!(!fx.run().concurrency.contains_key("claude"));
}

#[test]
fn a_runtime_first_seen_mid_run_starts_at_max_writers() {
    // Task M9.5.3b review m2: an entry is created at `max_writers`, never at 0.
    let mut fx = running(
        4,
        &[codex("c1", "a"), task("k1", "S", "b", "")],
        config(60, 10),
    );
    assert_eq!(cap(fx.run(), Runtime::Codex), 4, "no entry: max_writers");
    limited(&mut fx, "k1");
    assert_eq!(cap(fx.run(), Runtime::Claude), 2);
    assert_eq!(cap(fx.run(), Runtime::Codex), 4);
    limited(&mut fx, "c1");
    assert_eq!(
        cap(fx.run(), Runtime::Codex),
        2,
        "halved from 4, not from 0"
    );
    assert_eq!(fx.run().concurrency["codex"].halvings, 1);
}

#[test]
fn a_failed_turn_after_a_streak_is_one_event() {
    let failed = TurnOutcome::Failed {
        error: "rate_limit".into(),
        kind: FailureKind::RateLimit,
    };
    // No hold: every event halves, so two events would show as 4 → 1.
    let mut fx = running(4, &[codex("c1", "a")], config(0, 10));
    let w = window(&fx, "c1");
    fx.signal(w, retry());
    fx.signal(w, retry());
    fx.turn_ended(w, failed.clone());
    assert_eq!(cap(fx.run(), Runtime::Codex), 2, "the streak ran into it");
    assert_eq!(fx.run().concurrency["codex"].halvings, 1);
    assert_eq!(fx.run().rate_limits.get("codex"), Some(&1));
    // A failed turn with no streak before it is an event of its own.
    let mut fx = running(4, &[codex("c1", "a")], config(0, 10));
    let w = window(&fx, "c1");
    fx.turn_ended(w, failed);
    assert_eq!(cap(fx.run(), Runtime::Codex), 2);
}

/// Review focus 5, on the pure functions at exact times (offsets from `T`).
#[test]
fn flapping_rate_limits_halve_once_per_hold_and_recover_one_step_at_a_time() {
    const T: u64 = 10_000;
    let fx = running(4, &[codex("c1", "a")], config(60, 10));
    let mut run: Run = fx.run().clone();
    assert_eq!(run.limits.recover_after_secs, 600);
    let codex = |run: &Run| cap(run, Runtime::Codex);
    let quiet = |run: &mut Run, from: u64, to: u64| {
        for t in from..to {
            let before = run.clone();
            assert!(!on_tick(run, T + t), "tick at {t} changed a cap");
            assert_eq!(*run, before, "tick at {t} changed the run");
        }
    };
    assert!(on_rate_limit(&mut run, Runtime::Codex, T));
    assert_eq!(codex(&run), 2);
    assert!(!on_rate_limit(&mut run, Runtime::Codex, T + 10));
    assert!(!on_rate_limit(&mut run, Runtime::Codex, T + 50));
    assert_eq!(codex(&run), 2, "within the hold");
    assert!(on_rate_limit(&mut run, Runtime::Codex, T + 70));
    assert_eq!(codex(&run), 1);
    quiet(&mut run, 71, 670);
    assert!(on_tick(&mut run, T + 670));
    assert_eq!(codex(&run), 2);
    assert!(
        !on_rate_limit(&mut run, Runtime::Codex, T + 671),
        "the hold runs to 730"
    );
    assert_eq!(codex(&run), 2);
    quiet(&mut run, 672, 731);
    assert!(on_rate_limit(&mut run, Runtime::Codex, T + 731));
    assert_eq!(codex(&run), 1);
    quiet(&mut run, 732, 1331);
    for (at, want) in [(1331, 2), (1931, 3), (2531, 4)] {
        quiet(&mut run, at - 599, at);
        assert!(on_tick(&mut run, T + at), "recovery at {at}");
        assert_eq!(codex(&run), want, "at {at}");
    }
    quiet(&mut run, 2532, 4000);
    assert_eq!(codex(&run), 4, "never above max_writers");
    let entry = run.concurrency["codex"];
    assert_eq!((entry.halvings, entry.recoveries), (3, 4), "{entry:?}");
    assert_eq!(entry.last_rate_limit_at, Some(T + 731));
    assert_eq!(entry.last_change_at, Some(T + 2531));
}

#[test]
fn a_capped_runtime_does_not_block_the_other() {
    let tasks = [
        codex("c1", "a"),
        codex("c2", "b"),
        codex("c3", "c"),
        task("k1", "S", "d", ""),
    ];
    let mut fx = running(2, &tasks, config(60, 10));
    assert_eq!(fx.task("c1").state, TaskState::Working);
    assert_eq!(fx.task("c2").state, TaskState::Working);
    limited(&mut fx, "c1");
    assert_eq!(cap(fx.run(), Runtime::Codex), 1);
    fx.force("c2", TaskState::Review);
    assert_eq!(writers_busy_on(fx.run(), Runtime::Codex), 1);
    assert_eq!(
        fx.task("c3").state,
        TaskState::Queued,
        "codex is at its cap"
    );
    assert_eq!(
        fx.task("k1").state,
        TaskState::Preparing,
        "claude takes the slot"
    );
}

#[test]
fn a_lower_cap_preempts_nothing() {
    let ids = ["c1", "c2", "c3", "c4", "c5"];
    let tasks: Vec<String> = (ids.iter().zip(["a", "b", "c", "d", "e"]))
        .map(|(id, m)| codex(id, m))
        .collect();
    let mut fx = running(4, &tasks, config(60, 10));
    let effects = limited(&mut fx, "c1");
    assert_eq!(cap(fx.run(), Runtime::Codex), 2);
    assert!(
        !effects.iter().any(|e| matches!(
            e,
            Effect::KillWindow { .. } | Effect::Interrupt { .. } | Effect::RetireWindow { .. }
        )),
        "{effects:#?}"
    );
    for id in &ids[..4] {
        assert_eq!(fx.task(id).state, TaskState::Working, "{id}");
        assert!(!fx.task(id).rounds.last().unwrap().ended, "{id}");
    }
    assert_eq!(writers_busy_on(fx.run(), Runtime::Codex), 4);
    // Nothing new on codex until its writers are below the cap.
    fx.force("c4", TaskState::Review);
    assert_eq!(fx.task("c5").state, TaskState::Queued);
    fx.force("c3", TaskState::Review);
    assert_eq!(fx.task("c5").state, TaskState::Queued, "2 busy, cap 2");
    fx.force("c2", TaskState::Review);
    assert_eq!(fx.task("c5").state, TaskState::Preparing);
}

/// Ruling RC-1 (review ruling I4): a worker's, a reviewer's and a research task's
/// rate limit each halve their runtime's cap; an orchestrator's or a run scout's do not.
#[test]
fn every_task_session_counts() {
    // A worker (claude, the default runtime).
    let mut fx = running(3, &[task("t1", "S", "a", "")], config(60, 10));
    limited(&mut fx, "t1");
    assert_eq!(caps(&fx), [("claude".to_string(), 1)].into());

    // A reviewer: on the other runtime than its claude author.
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    let reviewer = fx.task("t1").rounds.last().unwrap().route.runtime;
    assert_eq!(reviewer, Runtime::Codex);
    let now = fx.now + 1;
    event(&mut fx, rwindow, now);
    assert_eq!(caps(&fx), [("codex".to_string(), 1)].into());

    // A research task's session.
    let (mut fx, rwindow) = researching();
    let runtime = fx.task("r1").rounds.last().unwrap().route.runtime;
    let now = fx.now + 1;
    event(&mut fx, rwindow, now);
    assert_eq!(caps(&fx), [(runtime.label().to_string(), 1)].into());

    // The orchestrator and a run scout: not task sessions.
    let mut fx = launched(false);
    let now = fx.now + 1;
    event(&mut fx, ORCH, now);
    scout(&mut fx, "api");
    let (op, _) = fx.op("StartScout");
    fx.done(op, OpResult::ScoutStarted { window_id: 77 });
    let now = fx.now + 1;
    event(&mut fx, 77, now);
    assert!(
        fx.run().concurrency.is_empty(),
        "{:?}",
        fx.run().concurrency
    );
    assert!(fx.run().rate_limits.is_empty());
    assert!(caps(&fx).is_empty());
}

#[test]
fn off_means_today() {
    // An older run reads decision 16's settings as off.
    let old: Run = serde_json::from_str(M93_RUN).expect("m93-run.json parses");
    let l = &old.limits;
    assert_eq!(
        (
            l.adaptive_concurrency,
            l.recover_after_secs,
            l.halve_hold_secs
        ),
        (false, 0, 0)
    );
    let mut fx = Fixture::new("");
    fx.next(EventKind::Restore {
        runs: vec![old],
        replay: Vec::new(),
        held: Vec::new(),
    });
    let id = fx.state.runs.keys().next().unwrap().clone();
    let run = fx.state.runs.get_mut(&id).unwrap();
    assert!(!on_rate_limit(run, Runtime::Codex, 5_000));
    assert!(!on_tick(run, 100_000));
    assert!(run.concurrency.is_empty());
    assert_eq!(cap(run, Runtime::Codex), run.limits.max_writers);

    // A run started with `adaptive_concurrency = false` schedules as 9.3 did.
    let mut off = config(60, 10);
    off.tuning.table.adaptive_concurrency = false;
    let tasks = [
        task("t1", "S", "a", ""),
        task("t2", "S", "b", ""),
        task("t3", "S", "c", ""),
    ];
    let mut fx = running(2, &tasks, off);
    assert!(!fx.run().limits.adaptive_concurrency);
    limited(&mut fx, "t1");
    assert_eq!(fx.run().rate_limits.get("claude"), Some(&1));
    assert!(fx.run().concurrency.is_empty());
    assert_eq!(cap(fx.run(), Runtime::Claude), 2);
    assert!(caps(&fx).is_empty());
    assert!(logged(&fx, "writers").is_empty());
    assert!(!attention(&fx).iter().any(|l| l.contains("capped")));
    fx.force("t2", TaskState::Review);
    assert_eq!(fx.task("t3").state, TaskState::Preparing, "no cap holds t3");

    // The same run with it on holds t3: claude's cap is 1 and t1 holds it.
    let mut fx = running(2, &tasks, config(60, 10));
    assert!(fx.run().limits.adaptive_concurrency);
    limited(&mut fx, "t1");
    fx.force("t2", TaskState::Review);
    assert_eq!(fx.task("t3").state, TaskState::Queued);
}

#[test]
fn caps_survive_a_restart() {
    let mut fx = running(3, &[codex("c1", "a")], config(60, 10));
    let at = fx.now + 1;
    let w = window(&fx, "c1");
    event(&mut fx, w, at);
    let before = fx.run().concurrency.clone();
    assert_eq!(before["codex"].cap, 1);
    // Written as `run.json` and read back by a new daemon.
    let json = serde_json::to_string(fx.run()).unwrap();
    let run: Run = serde_json::from_str(&json).unwrap();
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        runs: vec![run],
        replay: Vec::new(),
        held: Vec::new(),
    });
    assert_eq!(fx.run().concurrency, before);
    assert_eq!(cap(fx.run(), Runtime::Codex), 1);
    assert_eq!(caps(&fx), [("codex".to_string(), 1)].into());
    // Recovery runs from the recorded times.
    fx.send(at + 599, EventKind::Tick);
    assert_eq!(cap(fx.run(), Runtime::Codex), 1);
    fx.send(at + 600, EventKind::Tick);
    assert_eq!(cap(fx.run(), Runtime::Codex), 2);
}

#[test]
fn attention_and_log_name_the_cap() {
    let mut fx = running(3, &[codex("c1", "a")], config(60, 10));
    let at = fx.now + 1;
    let w = window(&fx, "c1");
    event(&mut fx, w, at);
    assert_eq!(
        logged(&fx, "writers"),
        vec!["rate limit on codex: writers 3 → 1"]
    );
    let capped = |n: u8| format!("codex writers capped at {n} of 3 after a rate limit");
    assert!(attention(&fx).contains(&capped(1)), "{:?}", attention(&fx));
    let before = fx.run().clone();
    fx.send(at + 599, EventKind::Tick);
    assert_eq!(fx.run().concurrency, before.concurrency);
    fx.send(at + 600, EventKind::Tick);
    assert_eq!(
        logged(&fx, "writers"),
        vec![
            "rate limit on codex: writers 3 → 1",
            "no rate limit on codex for 10 min: writers 1 → 2",
        ]
    );
    assert!(attention(&fx).contains(&capped(2)), "{:?}", attention(&fx));
    assert!(!attention(&fx).contains(&capped(1)));
    fx.send(at + 1200, EventKind::Tick);
    assert_eq!(
        logged(&fx, "writers").last().map(String::as_str),
        Some("no rate limit on codex for 10 min: writers 2 → 3")
    );
    assert!(!attention(&fx).iter().any(|l| l.contains("capped")));
    assert!(caps(&fx).is_empty());
    let report = crate::run::report::render(fx.run(), fx.now);
    let section = report
        .split("\n## ")
        .find(|s| s.starts_with("Concurrency"))
        .unwrap_or_else(|| panic!("no Concurrency section:\n{report}"));
    assert_eq!(
        section,
        "Concurrency\n\n- codex: writers 3 of 3 (1 rate limit, 1 halving, 2 recoveries)\n"
    );
}

#[test]
fn a_run_without_a_rate_limit_has_no_concurrency_section() {
    let fx = running(3, &[codex("c1", "a")], config(60, 10));
    let report = crate::run::report::render(fx.run(), fx.now);
    assert!(!report.contains("## Concurrency"), "{report}");
}

fn lane(lane: RaceLane, runtime: Runtime, state: LaneState) -> Lane {
    let route = Route {
        runtime,
        model: String::new(),
        strength: Strength::Standard,
        effort: Effort::Medium,
    };
    Lane {
        lane,
        route,
        review_route: None,
        checkout: format!("t1.{}", lane.label()),
        state,
        session: 0,
        start_commit: None,
        head: None,
        done: None,
        failures: 0,
        bounces: GateCounts::default(),
        stalls: 0,
        budget_exceeded: 0,
        spent: Spend::default(),
        reason: None,
        salvage_ref: None,
        cleared_locks: Vec::new(),
        kill_sent_at: None,
        exited: false,
        removed: false,
        kept: false,
    }
}

/// Ruling RR-9, ready for task M9.5.17a: a racing task holds one writer slot per live
/// lane, each on its lane's runtime; once decided, the task's own state counts again.
#[test]
fn writer_slots_count_live_lanes_on_their_runtimes() {
    let fx = running(3, &[task("t1", "S", "a", "")], config(60, 10));
    let mut run = fx.run().clone();
    assert_eq!(writers_busy(&run), 1);
    let race = |b: LaneState, winner| Race {
        lanes: vec![
            lane(RaceLane::A, Runtime::Claude, LaneState::Working),
            lane(RaceLane::B, Runtime::Codex, b),
        ],
        winner,
        adopted: false,
        started_at: 0,
    };
    run.tasks[0].race = Some(race(LaneState::Check, None));
    assert_eq!(writers_busy(&run), 2);
    assert_eq!(writers_busy_on(&run, Runtime::Claude), 1);
    assert_eq!(writers_busy_on(&run, Runtime::Codex), 1);
    for out in [LaneState::Review, LaneState::Out, LaneState::Lost] {
        run.tasks[0].race = Some(race(out, None));
        assert_eq!(writers_busy(&run), 1, "{out:?}");
        assert_eq!(writers_busy_on(&run, Runtime::Codex), 0, "{out:?}");
    }
    run.tasks[0].race = Some(race(LaneState::Working, Some(RaceLane::A)));
    assert_eq!(writers_busy(&run), 1, "decided: the task's own slot");
    assert_eq!(writers_busy_on(&run, Runtime::Codex), 0);
}

/// A run that has ended (accepted, discarded or failed) dispatches nothing more: its
/// caps stay as they ended, with no attention line and no recovery. A complete run can
/// still start a round (milestone 9.3), so it keeps recovering.
#[test]
fn an_ended_run_keeps_its_caps_quietly() {
    let mut fx = running(3, &[codex("c1", "a")], config(60, 10));
    let at = fx.now + 1;
    let w = window(&fx, "c1");
    event(&mut fx, w, at);
    let mut run = fx.run().clone();
    run.state = proto::RunState::Accepted;
    assert!(!on_tick(&mut run, at + 600));
    assert_eq!(cap(&run, Runtime::Codex), 1);
    assert!(crate::run::snapshot::attention(&run, at + 600).is_empty());
}
