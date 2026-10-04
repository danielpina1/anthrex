//! Milestone 9.5 task M9.5.17a, fix round 1: the race decision latched once per
//! dispatch (ruling T17a-1), the reader slots lanes hold (ruling T17a-2), and the
//! writer slot of a race with a winner (minor m5, ruling T17a-5); fix round 2: the latch
//! cleared by an amend or a return to `pending` (rulings T17a-3, T17a-4).

use proto::{AgentRole, LaneState, RaceLane, Runtime, TaskState};

use super::fixture::*;
use super::gates::{check_result, only_op};
use super::kinds::{changes, report_args, research, submit_report};
use super::race::{RACING, claim, lane, launched, racing, window};
use super::race_lanes::{HEAD_B, passes, proof, submit};
use crate::run::engine::schedule::writer_slots;
use crate::run::engine::{EventKind, OpResult};
use crate::run::model::{RaceDecision, RuntimeConcurrency};

/// A task on Codex, with `extra` lines before its route.
fn on_codex(id: &str, size: &str, module: &str, extra: &str) -> String {
    let route = "[task.route]\nruntime = \"codex\"\nmodel = \"\"";
    task(id, size, module, &format!("{extra}\n{route}"))
}

/// Claude's cap brought down to 1 by a rate limit at the fixture's start.
fn claude_capped(run: &mut crate::run::model::Run) {
    let held = RuntimeConcurrency {
        cap: 1,
        last_rate_limit_at: Some(2_000),
        ..RuntimeConcurrency::new(4)
    };
    run.concurrency.insert("claude".into(), held);
}

/// The run-log lines about task `id` that start with `prefix`.
fn lines(fx: &Fixture, id: &str, prefix: &str) -> usize {
    let prefix = format!("task {id}: {prefix}");
    fx.run()
        .log
        .iter()
        .filter(|l| l.text.starts_with(&prefix))
        .count()
}

fn history_lines(fx: &Fixture, id: &str, prefix: &str) -> usize {
    (fx.task(id).history.iter())
        .filter(|e| e.text.starts_with(prefix))
        .count()
}

fn started(fx: &Fixture, id: &str) -> bool {
    fx.task(id).state != TaskState::Queued && fx.task(id).state != TaskState::Pending
}

/// `t0` (Claude) works, `t4` (Codex) waits for it, and the racing `t1` was decided
/// single (off the critical path behind `t0 → t4`) while Claude's cap holds its one
/// worker back.
fn held_single() -> Fixture {
    let tasks = [
        task("t0", "M", "z", ""),
        on_codex("t4", "M", "e", "deps = [\"t0\"]"),
        task("t1", "M", "a", RACING),
    ];
    let mut fx = Fixture::new(&plan_with(&profile_with("max_writers = 3"), &tasks));
    fx.start_with(true, claude_capped);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx.launch_all();
    assert!(started(&fx, "t0") && !started(&fx, "t1"));
    assert!(
        fx.task("t1").race_decision.is_some(),
        "decided at its first pass"
    );
    fx
}

/// Ruling T17a-1: `t1` runs single (off the critical path, behind `t0 → t4`, `t4` on
/// Codex) while its runtime's cap holds its one worker back: one note, one history
/// line and one run-log line over every pass, and it stays single once it starts,
/// whatever the critical path says by then.
#[test]
fn a_single_racer_held_by_its_runtimes_cap_logs_its_line_once() {
    let mut fx = held_single();
    for _ in 0..4 {
        fx.tick();
    }
    let note = "race skipped: not on the critical path at dispatch";
    assert_eq!(lines(&fx, "t1", "race skipped"), 1);
    assert_eq!(history_lines(&fx, "t1", "race skipped"), 1);
    let decision = RaceDecision::Single {
        reason: note.into(),
    };
    assert_eq!(fx.task("t1").race_decision, Some(decision));
    // `t0` merges: `t1` now leads the critical path, but it was dispatched single.
    fx.merge("t0", HEAD);
    fx.launch_all();
    let t1 = fx.task("t1");
    assert!(started(&fx, "t1") && t1.race.is_none(), "{:?}", t1.state);
    assert_eq!(lines(&fx, "t1", "race skipped"), 1);
    assert!(t1.rounds.iter().all(|r| r.role == AgentRole::Worker));
}

/// Ruling T17a-1: the slot wait gives up once. `t1` holds the head of the line (no
/// second slot: Claude's cap is taken by `t0`), gives up after the wait, and from then
/// on runs single: `t3` behind it starts, and `t1` never holds the line again.
#[test]
fn the_slot_wait_gives_up_once() {
    let tasks = [
        on_codex("tdep", "S", "d", ""),
        task("t0", "M", "z", ""),
        task("t1", "S", "a", "race = true\ndeps = [\"tdep\"]"),
        on_codex("t3", "S", "c", "deps = [\"tdep\"]"),
        task("t4", "M", "e", "deps = [\"t1\"]"),
    ];
    let mut fx = Fixture::new(&plan_with(&profile_with("max_writers = 3"), &tasks));
    fx.start_with(true, claude_capped);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx.launch_all();
    fx.merge("tdep", HEAD);
    let since = fx.task("t1").race_wait_since.expect("t1 waits");
    assert!(
        !started(&fx, "t1") && !started(&fx, "t3"),
        "the line is held"
    );
    fx.send(since + 60, EventKind::Tick);
    assert_eq!(
        fx.task("t1").race_wait_since,
        Some(since),
        "never restarted"
    );
    fx.send(since + 120, EventKind::Tick);
    assert!(started(&fx, "t3"), "the line moves on");
    assert!(!started(&fx, "t1"), "its one worker waits for Claude");
    for at in [121, 180, 240, 241, 300] {
        fx.send(since + at, EventKind::Tick);
    }
    let text = "race skipped: no second writer slot within 120 s";
    assert_eq!(lines(&fx, "t1", text), 1);
    assert_eq!(history_lines(&fx, "t1", text), 1);
    assert_eq!(fx.task("t1").race_wait_since, None);
}

/// Ruling T17a-2: a lane holds a reader slot only once its reviewer round or its
/// `PrepareReview` exists. With one reader slot, held by a research task, both lanes
/// reach review and wait; once it frees, both are reviewed in turn (the first lane's
/// review asks for changes: one that approved would win, and the other lane would be
/// lost, task M9.5.17b).
#[test]
fn both_lanes_wait_for_one_reader_slot_and_are_reviewed_in_turn() {
    let mut fx = launched(
        &profile_with("max_writers = 3\nmax_readers = 1"),
        &[task("t1", "M", "a", RACING), research("r1", "")],
        config::Orchestrator::default(),
    );
    let scout = (fx.task("r1").rounds.iter())
        .rfind(|r| r.window_id.is_some())
        .and_then(|r| r.window_id)
        .expect("r1's window");
    for (l, head) in [(RaceLane::A, HEAD), (RaceLane::B, HEAD_B)] {
        let w = window(&fx, l);
        let effects = claim(&mut fx, l, w, head);
        let (op, _) = only_op(&effects, "Proof");
        let effects = fx.done(op, proof(true));
        let (op, _) = only_op(&effects, "Check");
        fx.done(op, check_result(true));
        assert_eq!(lane(&fx, l).state, LaneState::Review);
    }
    assert!(fx.ops("PrepareReview").is_empty(), "r1 holds the one slot");
    // r1 reports: the slot frees, and one lane's review starts.
    let effects = submit_report(&mut fx, scout, "r1", report_args());
    let prepares = ops_in(&effects, "PrepareReview");
    assert_eq!(prepares.len(), 1, "{effects:#?}");
    let first = fx.run().pending_ops[&prepares[0].0].lane.expect("a lane's");
    let rwindow = reviewer_of(&mut fx, prepares[0].0, first);
    let effects = submit(&mut fx, rwindow, changes());
    assert_eq!(lane(&fx, first).state, LaneState::Working);
    let other = match first {
        RaceLane::A => RaceLane::B,
        RaceLane::B => RaceLane::A,
    };
    let prepares = ops_in(&effects, "PrepareReview");
    assert_eq!(prepares.len(), 1, "{effects:#?}");
    assert_eq!(fx.run().pending_ops[&prepares[0].0].lane, Some(other));
}

/// The reviewer window of lane `l`'s `PrepareReview` `op`.
fn reviewer_of(fx: &mut Fixture, op: crate::run::model::OpId, l: RaceLane) -> u32 {
    let review = OpResult::Review {
        base: BASE.into(),
        head: if l == RaceLane::A { HEAD } else { HEAD_B }.into(),
        patch: "diff --git a/x b/x".into(),
    };
    fx.done(op, review);
    let windows = fx.complete_windows();
    windows[0].1
}

/// Minor m5: between `Won` and `Crowned` the race's one slot is the winning lane's,
/// on its runtime (the task takes the lane's route only at the crown).
#[test]
fn a_won_lane_b_holds_its_slot_on_its_runtime_until_the_crown() {
    let (mut fx, _, _) = racing();
    let effects = passes(&mut fx, RaceLane::B, HEAD_B);
    only_op(&effects, "CrownRacer");
    let t1 = fx.task("t1");
    assert_eq!(t1.route.runtime, Runtime::Claude, "lane a's, as yet");
    assert_eq!(t1.state, TaskState::Working);
    assert_eq!(writer_slots(t1), [Runtime::Codex], "won");
}

/// The crowned task handed back into its checkout, `working`: its writer slot is on
/// `task.route`'s runtime, the crowned lane's (`become_lane`); a later route change
/// (rung 2, a retry) moves it.
fn slot_follows_the_route(fx: &mut Fixture) {
    super::race_crown::handed_back(fx);
    let t1 = fx.task("t1");
    assert_eq!(
        t1.route.runtime,
        Runtime::Codex,
        "the crown swapped the route"
    );
    assert_eq!(writer_slots(t1), [Runtime::Codex]);
    fx.task_mut("t1").route.runtime = Runtime::Claude;
    assert_eq!(
        writer_slots(fx.task("t1")),
        [Runtime::Claude],
        "the route's"
    );
}

/// Ruling T17a-5: a crowned race counts on `task.route.runtime`, the crowned lane's at
/// the crown and whatever the route becomes after it.
#[test]
fn a_crowned_race_counts_its_slot_on_the_tasks_route() {
    let (mut fx, _, _) = racing();
    let effects = passes(&mut fx, RaceLane::B, HEAD_B);
    let (op, _) = only_op(&effects, "CrownRacer");
    super::race_crown::crowned(&mut fx, op, HEAD_B);
    slot_follows_the_route(&mut fx);
}

/// Ruling T17a-5, the adoption: lane b `Adopted` on its racer's question is crowned
/// through the reducer, which swaps its route in; the answer has the task work again.
#[test]
fn an_adopted_lane_b_counts_its_slot_on_the_tasks_route() {
    let (mut fx, _, _) = racing();
    let t1 = fx.task_mut("t1");
    let mut race = crate::run::test_support::race_of(t1, [LaneState::Out, LaneState::Adopted]);
    race.crowned = false;
    race.lanes[1].head = Some(HEAD_B.into());
    race.lanes[1].gates.block = Some(proto::BlockInfo {
        reason: proto::BlockReason::Question,
        text: "which API?".into(),
    });
    t1.race = Some(race);
    let effects = fx.tick();
    let (op, _) = only_op(&effects, "CrownRacer");
    super::race_crown::crowned(&mut fx, op, HEAD_B);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    super::dispatch::edit(&mut fx, vec![super::holds::answer("use v2")]);
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Working);
    assert_eq!(
        writer_slots(t1),
        [Runtime::Codex],
        "the crown swapped the route"
    );
    fx.task_mut("t1").route.runtime = Runtime::Claude;
    assert_eq!(
        writer_slots(fx.task("t1")),
        [Runtime::Claude],
        "the route's"
    );
}

/// `amend_task` of `t1` with `race` alone.
fn amend_race(id: &str, race: bool) -> proto::PlanEdit {
    proto::PlanEdit::AmendTask {
        task_id: id.into(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps: None,
        stage: None,
        race: Some(race),
        pair: None,
    }
}

/// Rulings T17a-3 and T17a-4: an amend of a queued task's race clears its latch. `t1`,
/// decided single and held by Claude's cap, is amended to `race = false`: the latch
/// goes, and once Claude has room one ordinary worker starts, with no new race line.
#[test]
fn an_amended_race_clears_the_latch() {
    let mut fx = held_single();
    super::dispatch::edit(&mut fx, vec![amend_race("t1", false)]);
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.race_decision.as_ref()),
        (TaskState::Queued, None)
    );
    fx.merge("t0", HEAD);
    fx.launch_all();
    let t1 = fx.task("t1");
    assert!(started(&fx, "t1") && t1.race.is_none(), "{:?}", t1.state);
    assert!(t1.rounds.iter().all(|r| r.role == AgentRole::Worker));
    assert_eq!(
        t1.race_decision, None,
        "a task that does not race decides nothing"
    );
    assert_eq!(
        lines(&fx, "t1", "race skipped"),
        1,
        "the first stint's line only"
    );
}

/// Ruling T17a-4: a queued task sent back to `pending` (`requeue_waiting`: it gains an
/// unfinished dependency) decides again when it is queued again.
#[test]
fn a_task_back_to_pending_decides_again() {
    let mut fx = held_single();
    super::dispatch::edit(&mut fx, vec![super::holds::add_dep("t1", "t0")]);
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.race_decision.as_ref()),
        (TaskState::Pending, None)
    );
    assert_eq!(t1.race_wait_since, None);
    // `t0` merges: `t1` is queued again and decided afresh (it races, or a second
    // line says why not).
    fx.merge("t0", HEAD);
    let t1 = fx.task("t1");
    assert!(t1.race_decision.is_some(), "{:?}", t1.state);
    assert!(t1.race.is_some() || lines(&fx, "t1", "race skipped") == 2);
}

/// Task 17a's second re-review (d): an amend of `size` alone re-resolves the route (and
/// with it the second racer's model), so it clears the latch like a route amend.
#[test]
fn an_amended_size_clears_the_latch() {
    let mut fx = held_single();
    let amend = proto::PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: Some(proto::Size::S),
        deps: None,
        stage: None,
        race: None,
        pair: None,
    };
    let effects = super::dispatch::edit(&mut fx, vec![amend]);
    assert!(
        super::dispatch::replies(&effects)[0].is_ok(),
        "{effects:#?}"
    );
    // The latch went: the same pass decides again, with its own line.
    let t1 = fx.task("t1");
    assert_eq!((t1.size, t1.state), (proto::Size::S, TaskState::Queued));
    assert!(t1.race_decision.is_some());
    assert_eq!(lines(&fx, "t1", "race skipped"), 2, "decided again");
}
