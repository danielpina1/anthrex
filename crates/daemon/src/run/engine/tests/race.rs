//! Milestone 9.5 task M9.5.17a, part 1: a racing task's dispatch (decisions 18–19,
//! rulings RR-1, RR-9, T1-3): two lane checkouts on two runtimes, the fallbacks to one
//! worker, the head of the line, the runtimes' caps and the writer slots lanes hold.
//! Part 2 (each lane's gates, elimination, the crown) and part 3 (the worker checks)
//! are in `race_lanes.rs`.

use proto::{AgentRole, LaneState, RaceLane, Runtime, TaskState};
use serde_json::json;

use super::dispatch::{replies, task_path};
use super::fixture::*;
use crate::run::contract::{DONE_ACCEPTED, WORKER_CONTRACT};
use crate::run::engine::schedule::{writer_slots, writers_busy};
use crate::run::engine::{Effect, EventKind, OpKind, OpResult};
use crate::run::model::{Lane, OpId, RuntimeConcurrency, task_branch};

/// A racing task's extra line.
pub(super) const RACING: &str = "race = true";
/// The second lane's runtime, and a task on it.
const CODEX: &str = "[task.route]\nruntime = \"codex\"\nmodel = \"\"";

/// `config` with its small and medium implementer rows racing on Codex: milestone 9.8
/// decision 28's second racer is the row's fallback (`codex:default`, the peer route
/// milestone 9.5's racer took), else the task's own route.
pub(super) fn with_racers(mut config: config::Orchestrator) -> config::Orchestrator {
    use proto::models::{ModelRef, Role};
    for role in [Role::ImplementerSmall, Role::ImplementerMedium] {
        let row = (config.roles.rows)
            .entry(role)
            .or_insert_with(|| config::models::builtin_choice(role));
        row.fallback
            .get_or_insert_with(|| ModelRef::default_of(Runtime::Codex));
    }
    config
}

/// [`Fixture::new`] with [`with_racers`].
pub(super) fn racers(plan: &str) -> Fixture {
    Fixture::with_config(plan, with_racers(config::Orchestrator::default()))
}

/// A running run of `tasks` on `profile` (with `config` and [`with_racers`]), every
/// dispatched session launched.
pub(super) fn launched(profile: &str, tasks: &[String], config: config::Orchestrator) -> Fixture {
    let plan = plan_with(profile, tasks);
    let mut fx = Fixture::with_config(&plan, with_racers(config));
    fx.ready(true);
    fx.launch_all();
    fx
}

/// A running racing `t1` (M, owning `crates/a/**`): its fixture and each lane's racer
/// window.
pub(super) fn racing() -> (Fixture, u32, u32) {
    let fx = launched(
        PROFILE,
        &[task("t1", "M", "a", RACING)],
        config::Orchestrator::default(),
    );
    let (a, b) = (window(&fx, RaceLane::A), window(&fx, RaceLane::B));
    (fx, a, b)
}

/// The window of `t1`'s latest racer in `lane`.
pub(super) fn window(fx: &Fixture, lane: RaceLane) -> u32 {
    (fx.task("t1").rounds.iter())
        .rfind(|r| r.role == AgentRole::Racer && r.lane == Some(lane))
        .and_then(|r| r.window_id)
        .unwrap_or_else(|| panic!("no racer window in lane {lane:?}"))
}

/// `t1`'s lane `lane`.
pub(super) fn lane(fx: &Fixture, lane: RaceLane) -> Lane {
    (fx.task("t1").race.as_ref().expect("a race").lanes.iter())
        .find(|l| l.lane == lane)
        .cloned()
        .expect("the lane")
}

/// The lane `op` was sent for, while it is pending.
pub(super) fn lane_of(fx: &Fixture, op: OpId) -> Option<RaceLane> {
    fx.run().pending_ops.get(&op).expect("a pending op").lane
}

/// Lane `lane`'s checkout path and branch.
pub(super) fn lane_path(lane: RaceLane) -> std::path::PathBuf {
    task_path(&format!("t1.{}", lane.label()))
}

pub(super) fn lane_branch(lane: RaceLane) -> String {
    task_branch(RUN_ID, &format!("t1.{}", lane.label()))
}

/// `DoneChecked` for lane `lane` with nothing wrong, at `head`.
pub(super) fn lane_check(fx: &Fixture, lane: RaceLane, head: &str) -> OpResult {
    let mut result = fx.clean_check("t1");
    if let OpResult::DoneChecked {
        head: h,
        head_branch,
        ..
    } = &mut result
    {
        *h = head.to_string();
        *head_branch = Some(lane_branch(lane));
    }
    result
}

/// A tdd claim naming a test and a red.
pub(super) fn tdd() -> serde_json::Value {
    json!({"summary": "did it", "test": "a::works", "red": "abcdef1"})
}

/// Lane `lane`'s racer claims done from `window` at `head`; the claim is accepted and
/// the racer ends its turn. Returns the accepting step's effects.
pub(super) fn claim(fx: &mut Fixture, lane: RaceLane, window: u32, head: &str) -> Vec<Effect> {
    let effects = fx.tool_as(AgentRole::Racer, window, "t1", "task_done", tdd());
    let (op, kind) = ops_in(&effects, "VerifyDone")[0].clone();
    assert_eq!(lane_of(fx, op), Some(lane));
    match kind {
        OpKind::VerifyDone { worktree, .. } => assert_eq!(worktree, lane_path(lane)),
        other => panic!("{other:?}"),
    }
    let result = lane_check(fx, lane, head);
    let effects = fx.done(op, result);
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);
    fx.turn_completed(window);
    effects
}

/// Every op in `fx`'s log, in order.
pub(super) fn all_ops(fx: &Fixture) -> Vec<OpKind> {
    (fx.log.iter())
        .filter_map(|e| match e {
            Effect::Op { kind, .. } => Some(kind.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_race_prepares_two_lane_checkouts_on_two_runtimes() {
    let plan = plan_with(PROFILE, &[task("t1", "M", "a", RACING)]);
    let mut fx = racers(&plan);
    let effects = fx.ready(true);
    let prepares = ops_in(&effects, "PrepareWorktree");
    let shown: Vec<_> = (prepares.iter())
        .map(|(op, kind)| match kind {
            OpKind::PrepareWorktree {
                branch, from, path, ..
            } => (
                lane_of(&fx, *op),
                path.clone(),
                branch.clone(),
                from.clone(),
            ),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        shown,
        [RaceLane::A, RaceLane::B].map(|l| (Some(l), lane_path(l), lane_branch(l), BASE.into()))
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    fx.complete_prepares();
    let windows = fx.ops("CreateWindow");
    let shown: Vec<_> = (windows.iter())
        .map(|(_, kind)| match kind {
            OpKind::CreateWindow {
                name,
                spec,
                worktree,
                ..
            } => {
                let mcp = spec.mcp.as_ref().expect("an MCP target");
                let run_ref = spec.run_ref.as_ref().expect("a run reference");
                assert_eq!(spec.instructions, WORKER_CONTRACT);
                assert_eq!((&spec.cwd, run_ref.lane), (worktree, mcp.lane));
                (
                    name.clone(),
                    mcp.role,
                    mcp.lane,
                    spec.runtime,
                    worktree.clone(),
                )
            }
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        shown,
        [
            (
                format!("{H4}/t1.aw1"),
                AgentRole::Racer,
                Some(RaceLane::A),
                Runtime::Claude,
                lane_path(RaceLane::A),
            ),
            (
                format!("{H4}/t1.bw2"),
                AgentRole::Racer,
                Some(RaceLane::B),
                Runtime::Codex,
                lane_path(RaceLane::B),
            ),
        ]
    );
    let rounds: Vec<_> = (fx.task("t1").rounds.iter())
        .map(|r| (r.role, r.session, r.lane))
        .collect();
    assert_eq!(
        rounds,
        [
            (AgentRole::Racer, 1, Some(RaceLane::A)),
            (AgentRole::Racer, 2, Some(RaceLane::B)),
        ]
    );
    // Nothing touches the task's own checkout or branch before a crown.
    let (own_path, own_branch) = (task_path("t1"), task_branch(RUN_ID, "t1"));
    for kind in all_ops(&fx) {
        let text = format!("{kind:?}");
        assert!(!text.contains(&format!("{own_branch}\"")), "{text}");
        assert!(
            !text.contains(&format!("{}\"", own_path.display())),
            "{text}"
        );
    }
}

/// `t1`'s single worker, started with `note` (decision 18).
fn single(fx: &Fixture, note: &str) {
    let t1 = fx.task("t1");
    assert!(t1.race.is_none());
    assert_eq!(t1.notes.iter().filter(|n| *n == note).count(), 1);
    let line = format!("task t1: {note}");
    assert_eq!(fx.run().log.iter().filter(|l| l.text == line).count(), 1);
    let names: Vec<String> = (fx.ops("CreateWindow").into_iter())
        .filter_map(|(_, kind)| match kind {
            OpKind::CreateWindow { name, .. } if name.contains("/t1.") => Some(name),
            _ => None,
        })
        .collect();
    assert_eq!(names, [format!("{H4}/t1.w1")]);
    assert_eq!(fx.task("t1").rounds[0].role, AgentRole::Worker);
}

#[test]
fn a_race_off_the_critical_path_runs_single() {
    // `t2` (M) leads the critical path; `t1` (S) is off it.
    let fx = launched(
        PROFILE,
        &[task("t1", "S", "a", RACING), task("t2", "M", "b", "")],
        config::Orchestrator::default(),
    );
    single(&fx, "race skipped: not on the critical path at dispatch");
}

#[test]
fn max_writers_1_runs_single() {
    let fx = launched(
        &profile_with("max_writers = 1"),
        &[task("t1", "M", "a", RACING)],
        config::Orchestrator::default(),
    );
    single(&fx, "race skipped: max_writers is 1");
}

/// Task M9.5.14's review: a plan file is validated against nothing installed, so the
/// second racer's runtime may turn out not installed at dispatch. Milestone 9.8
/// decision 28: lane b then races on the task's own route.
#[test]
fn a_race_whose_fallback_is_not_installed_races_on_the_tasks_route() {
    let plan = plan_with(PROFILE, &[task("t1", "M", "a", RACING)]);
    let mut fx = racers(&plan);
    fx.start_with(true, |run| {
        run.orch.installed.insert("codex".into(), false);
    });
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx.launch_all();
    let t1 = fx.task("t1");
    let lanes = &t1.race.as_ref().expect("it races").lanes;
    assert_eq!(lanes[1].route, t1.route);
    assert_eq!(lanes[1].route.runtime, Runtime::Claude);
}

/// `tdep` (S) and `t0` (M) start first; `t1` races once `tdep` merges ([`unblock`]),
/// then leads the critical path (with `t4` after it), with `t0` still holding a writer
/// slot; `t3` (S) comes last.
pub(super) fn behind_one_slot(limits: &str, t0_extra: &str) -> Fixture {
    let tasks = [
        task("tdep", "S", "d", ""),
        task("t0", "M", "z", t0_extra),
        task("t1", "S", "a", "race = true\ndeps = [\"tdep\"]"),
        task("t3", "S", "c", ""),
        task("t4", "M", "e", "deps = [\"t1\"]"),
    ];
    let fx = launched(
        &profile_with(limits),
        &tasks,
        config::Orchestrator::default(),
    );
    assert_eq!(fx.task("t0").state, TaskState::Working);
    fx
}

/// `tdep` merges: `t1` is runnable.
pub(super) fn unblock(fx: &mut Fixture) {
    fx.merge("tdep", HEAD);
}

fn started(fx: &Fixture, id: &str) -> bool {
    fx.ops("CreateWindow")
        .iter()
        .chain(fx.ops("PrepareWorktree").iter())
        .any(|(_, kind)| format!("{kind:?}").contains(&format!("/{id}")))
}

#[test]
fn a_race_holds_the_head_of_the_line_then_gives_up() {
    let mut fx = behind_one_slot("max_writers = 2", "");
    unblock(&mut fx);
    let since = fx.now;
    assert_eq!(fx.task("t1").race_wait_since, Some(since));
    assert!(
        !started(&fx, "t1") && !started(&fx, "t3"),
        "the line is held"
    );
    fx.send(since + 119, EventKind::Tick);
    assert!(!started(&fx, "t1") && !started(&fx, "t3"));
    fx.send(since + 120, EventKind::Tick);
    let t1 = fx.task("t1");
    assert!(t1.race.is_none() && t1.race_wait_since.is_none());
    assert_eq!(t1.state, TaskState::Preparing);
    let note = "race skipped: no second writer slot within 120 s";
    assert!(t1.notes.iter().any(|n| n == note));
    assert!(!started(&fx, "t3"), "the one slot left is t1's");
}

#[test]
fn lanes_respect_runtime_caps() {
    // `t0` works on Codex, whose cap a rate limit brought down to 1: lane b has no room.
    let mut fx = behind_one_slot("max_writers = 4", CODEX);
    assert_eq!(fx.task("t0").route.runtime, Runtime::Codex);
    assert!(fx.run().limits.adaptive_concurrency);
    let now = fx.now;
    fx.run_mut().concurrency.insert(
        "codex".into(),
        RuntimeConcurrency {
            cap: 1,
            last_rate_limit_at: Some(now),
            ..RuntimeConcurrency::new(4)
        },
    );
    unblock(&mut fx);
    assert_eq!(writers_busy(fx.run()), 2, "t0 and t3: two slots are free");
    assert!(fx.task("t1").race_wait_since.is_some());
    assert!(!started(&fx, "t1"), "two slots free, but none on Codex");
    // Codex is free again: the race starts, one lane on each runtime.
    fx.merge("t0", "5555555555555555555555555555555555555555");
    let t1 = fx.task("t1");
    let runtimes: Vec<Runtime> = (t1.race.iter().flat_map(|r| &r.lanes))
        .map(|l| l.route.runtime)
        .collect();
    assert_eq!(runtimes, [Runtime::Claude, Runtime::Codex]);
    assert!(t1.race_wait_since.is_none());
}

#[test]
fn writers_busy_counts_live_lanes() {
    let (mut fx, _, _) = racing();
    assert_eq!(
        writers_busy(fx.run()),
        2,
        "ruling RR-9: one slot per live lane"
    );
    assert_eq!(
        writer_slots(fx.task("t1")),
        [Runtime::Claude, Runtime::Codex]
    );
    let race = fx.task_mut("t1").race.as_mut().expect("a race");
    race.lanes[1].state = LaneState::Out;
    assert_eq!(writers_busy(fx.run()), 1);
    assert_eq!(writer_slots(fx.task("t1")), [Runtime::Claude]);
}

/// Task M9.5.13's review: lanes count only while the task races. A crowned or adopted
/// race is its task's (one slot while it writes, none in the merge queue); a cancelled
/// task holds none, whatever its lanes say.
#[test]
fn a_race_that_ended_counts_as_its_task() {
    let (mut fx, _, _) = racing();
    let t1 = fx.task_mut("t1");
    t1.race = Some(crate::run::test_support::race_of(
        t1,
        [LaneState::Lost, LaneState::Won],
    ));
    t1.state = TaskState::MergeQueue;
    assert_eq!(writers_busy(fx.run()), 0, "crowned, in the merge queue");
    let t1 = fx.task_mut("t1");
    t1.race = Some(crate::run::test_support::race_of(
        t1,
        [LaneState::Adopted, LaneState::Out],
    ));
    t1.state = TaskState::Working;
    assert_eq!(
        writer_slots(fx.task("t1")),
        [Runtime::Claude],
        "adopted, working"
    );
    let t1 = fx.task_mut("t1");
    t1.race = Some(crate::run::test_support::race_of(
        t1,
        [LaneState::Working, LaneState::Working],
    ));
    t1.state = TaskState::Cancelled;
    assert_eq!(writers_busy(fx.run()), 0, "cancelled");
}
