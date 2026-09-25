//! M8a.11: dispatch order, writer and reader slots, the hub rule, the window limit, the
//! snapshot and the revision counter.

use proto::{BlockReason, TaskState};

use super::fixture::*;
use crate::run::engine::{AgentSignal, Effect, OpResult};
use crate::run::snapshot::snapshot;

fn first_prepared(plan: &str) -> Vec<String> {
    let mut fx = Fixture::new(plan);
    fx.ready(true);
    tasks_of(&fx.log, "PrepareWorktree")
}

#[test]
fn critical_path_orders_dispatch() {
    let one = profile_with("max_writers = 1");
    // b's chain (M then M) is longer than a's (S), though a is first in plan order.
    let plan = plan_with(
        &one,
        &[
            task("a", "S", "a", ""),
            task("b", "M", "b", ""),
            task("c", "M", "c", "deps = [\"b\"]"),
        ],
    );
    assert_eq!(first_prepared(&plan), vec!["b"]);
    // Equal critical paths: the higher priority first, then plan order.
    let plan = plan_with(
        &one,
        &[task("x", "S", "x", ""), task("y", "S", "y", "priority = 2")],
    );
    assert_eq!(first_prepared(&plan), vec!["y"]);
    let plan = plan_with(&one, &[task("p", "S", "p", ""), task("q", "S", "q", "")]);
    assert_eq!(first_prepared(&plan), vec!["p"]);
    // A negative priority loses to the default.
    let plan = plan_with(
        &one,
        &[
            task("p", "S", "p", "priority = -1"),
            task("q", "S", "q", ""),
        ],
    );
    assert_eq!(first_prepared(&plan), vec!["q"]);
}

#[test]
fn writer_and_reader_slots_are_separate() {
    let plan = plan_with(
        &profile_with("max_writers = 1\nmax_readers = 1"),
        &[
            task("t1", "S", "a", "priority = 3"),
            task("t2", "S", "b", "priority = 2"),
            task("t3", "S", "c", "priority = 1"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1"]);
    fx.launch_all();
    let effects = fx.force("t1", TaskState::Review);
    // t1's reviewer takes the reader slot; t1 no longer holds a writer slot.
    assert_eq!(tasks_of(&effects, "PrepareReview"), vec!["t1"]);
    assert_eq!(tasks_of(&effects, "PrepareWorktree"), vec!["t2"]);
    fx.launch_all();
    let effects = fx.force("t2", TaskState::Review);
    assert!(
        tasks_of(&effects, "PrepareReview").is_empty(),
        "{effects:#?}"
    );
    assert_eq!(tasks_of(&effects, "PrepareWorktree"), vec!["t3"]);
    // t1's review session starts, and still holds the slot while it is live.
    let (op, _) = fx.op("PrepareReview");
    let effects = fx.done(
        op,
        OpResult::Review {
            base: BASE.into(),
            head: "d1".repeat(20),
            patch: "diff --git a/x b/x".into(),
        },
    );
    let windows = ops_in(&effects, "CreateWindow");
    assert_eq!(windows.len(), 1, "{effects:#?}");
    let crate::run::engine::OpKind::CreateWindow { name, .. } = &windows[0].1 else {
        unreachable!()
    };
    assert_eq!(name, &format!("{H4}/t1.r1"));
    fx.complete_windows();
    fx.tick();
    assert_eq!(tasks_of(&fx.log, "PrepareReview"), vec!["t1"]);
    // Its round ends; the waiting reviewer gets the slot.
    fx.end_review("t1");
    let effects = fx.tick();
    assert_eq!(tasks_of(&effects, "PrepareReview"), vec!["t2"]);
}

#[test]
fn hub_runs_alone() {
    // The hub task has the longest critical path, so it goes first and holds everyone.
    let plan = plan_with(
        &profile_with("max_writers = 3"),
        &[
            task("t1", "S", "a", ""),
            task("h", "M", "proto", ""),
            task("t3", "S", "c", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["h"]);
    fx.launch_all();
    fx.tick();
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["h"]);
    fx.merge("h", &"c1".repeat(20));
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["h", "t1", "t3"]);

    // A hub task waits while any writer slot is held.
    let plan = plan_with(
        &profile_with("max_writers = 3"),
        &[
            task("t1", "M", "a", "priority = 9"),
            task("h", "M", "proto", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1"]);
    fx.launch_all();
    fx.force("t1", TaskState::Check);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1"]);
    // Final review A-I2: nor while one is in review or the merge queue, from where it
    // can come back to `working` (a rejection, a hand-back) beside the hub.
    fx.force("t1", TaskState::Review);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1"]);
    fx.end_review("t1");
    fx.tick();
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1"]);
    fx.merge("t1", &"c1".repeat(20));
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1", "h"]);
}

/// Final review A-I2: a started hub task holds the hub until it finishes, wherever it
/// is, so nothing else starts while it is in review, in the merge queue or blocked on
/// a question, from where it comes back to `working`.
#[test]
fn a_started_hub_task_holds_the_hub_until_it_finishes() {
    let plan = plan_with(
        &profile_with("max_writers = 3"),
        &[
            task("h", "M", "proto", ""),
            task("t4", "S", "a", ""),
            task("t5", "S", "b", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["h"]);
    fx.launch_all();
    let effects = fx.force("h", TaskState::Review);
    // Its own reviewer starts.
    assert_eq!(tasks_of(&effects, "PrepareReview"), vec!["h"]);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["h"]);
    fx.end_review("h");
    fx.tick();
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["h"]);
    let h = fx.task_mut("h");
    h.state = TaskState::Blocked;
    h.block = Some(proto::BlockInfo {
        reason: BlockReason::Question,
        text: "which one?".into(),
    });
    fx.tick();
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["h"]);
    fx.merge("h", &"c1".repeat(20));
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["h", "t4", "t5"]);
}

/// F3 review N1: a started hub task held on a dependency it gained (`add_dep` while it
/// is blocked, M8a.6 ruling N5) does not hold the hub against that dependency, or the
/// run freezes: the dependency never dispatches and the hub waits for it forever. Only
/// the held hub's unfinished dependencies start meanwhile; once they are merged the hub
/// holds it again.
#[test]
fn a_hub_task_held_on_a_new_dependency_lets_that_dependency_run() {
    let plan = plan_with(
        &profile_with("max_writers = 3"),
        &[
            task("h", "M", "proto", ""),
            task("t4", "S", "a", ""),
            task("t5", "S", "b", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    fx.launch_all();
    let h = fx.task_mut("h");
    h.state = TaskState::Blocked;
    h.block = Some(proto::BlockInfo {
        reason: BlockReason::Question,
        text: "which one?".into(),
    });
    fx.tick();
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["h"]);
    let effects = super::dispatch::edit(&mut fx, vec![super::holds::add_dep("h", "t4")]);
    assert!(
        super::dispatch::replies(&effects)[0].is_ok(),
        "{effects:#?}"
    );
    assert!(fx.task("h").awaiting_deps, "h is held on t4");
    fx.tick();
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["h", "t4"]);
    fx.launch_all();
    fx.merge("t4", &"c4".repeat(20));
    fx.tick();
    assert_eq!(
        tasks_of(&fx.log, "PrepareWorktree"),
        vec!["h", "t4"],
        "t5 waits for the hub"
    );
}

#[test]
fn window_limit_blocks_the_task_as_environment() {
    let config = config::Orchestrator {
        max_windows: 1,
        ..Default::default()
    };
    let plan = plan_with(
        &profile_with("max_writers = 2"),
        &[
            task("t1", "S", "a", "priority = 1"),
            task("t2", "S", "b", ""),
        ],
    );
    let mut fx = Fixture::with_config(&plan, config);
    fx.ready(true);
    fx.complete_prepares();
    assert_eq!(tasks_of(&fx.log, "CreateWindow"), vec!["t1"]);
    let task = fx.task("t2");
    assert_eq!(task.state, TaskState::Blocked);
    let block = task.block.as_ref().unwrap();
    assert_eq!(block.reason, BlockReason::Environment);
    assert_eq!(block.text, "run window limit (1) reached");
    assert_eq!(fx.run().windows_created, 1);
}

#[test]
fn snapshot_marks_critical_path_and_wave() {
    let plan = plan_with(
        &profile_with("max_writers = 1"),
        &[
            task("a", "S", "a", ""),
            task("b", "M", "b", ""),
            task("c", "M", "c", "deps = [\"b\"]"),
            task("d", "S", "d", "deps = [\"c\"]"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let snap = snapshot(&fx.state, fx.now);
    assert_eq!(snap.revision, fx.state.revision);
    let info = &snap.runs[0];
    assert_eq!(info.run_id, RUN_ID);
    assert_eq!(info.revision, fx.run().revision);
    assert_eq!(info.critical_path, vec!["b", "c", "d"]);
    let marks: Vec<(String, bool, u32)> = info
        .tasks
        .iter()
        .map(|t| (t.id.clone(), t.on_critical_path, t.wave))
        .collect();
    assert_eq!(
        marks,
        vec![
            ("a".into(), false, 0),
            ("b".into(), true, 0),
            ("c".into(), true, 1),
            ("d".into(), true, 2),
        ]
    );
    assert_eq!(info.writers_busy, 1);
    assert_eq!(info.readers_busy, 0);
    assert_eq!(info.run_branch, format!("anthrex/{RUN_ID}/integration"));
    fx.launch_all();
    fx.merge("b", &"c1".repeat(20));
    let info = &snapshot(&fx.state, fx.now).runs[0];
    assert_eq!(info.critical_path, vec!["c", "d"]);
    assert!(
        !info.tasks[1].on_critical_path,
        "a merged task is off the path"
    );
    assert_eq!(info.tasks[2].wave, 1, "waves count merged dependencies too");
}

#[test]
fn revision_bumps_on_every_change_and_only_then() {
    let plan = plan_with(PROFILE, &[task("t1", "S", "a", "")]);
    let mut fx = Fixture::new(&plan);
    let effects = fx.start(false);
    assert_eq!(fx.state.revision, 1);
    assert_eq!(fx.run().revision, 1, "a new run starts at 1");
    assert!(effects.contains(&Effect::Persist {
        run_id: RUN_ID.into(),
        urgent: true
    }));
    assert!(effects.iter().any(|e| matches!(e, Effect::Publish { .. })));

    let mut last = (fx.state.revision, fx.run().revision);
    let mut expect_bump = |fx: &Fixture, effects: &[Effect], what: &str| {
        let now = (fx.state.revision, fx.run().revision);
        assert_eq!(now, (last.0 + 1, last.1 + 1), "{what}");
        assert!(
            matches!(effects.first(), Some(Effect::Persist { .. })),
            "{what}: Persist comes first: {effects:#?}"
        );
        assert!(effects.iter().any(|e| matches!(e, Effect::Publish { .. })));
        last = now;
    };
    let (op, _) = fx.op("CreateRunBranch");
    let effects = fx.done(op, OpResult::Worktree { head: BASE.into() });
    expect_bump(&fx, &effects, "run branch done");
    let effects = fx.complete_prepares();
    expect_bump(&fx, &effects, "pre-warm done");
    let effects = fx.approve();
    expect_bump(&fx, &effects, "approve");
    let mark = fx.log.len();
    let windows = fx.complete_windows();
    let effects = fx.log[mark..].to_vec();
    expect_bump(&fx, &effects, "window");
    let effects = fx.signal(
        windows[0].1,
        AgentSignal::ToolUse {
            name: "Bash".into(),
        },
    );
    expect_bump(&fx, &effects, "a counter");

    // Nothing due: nothing changes, nothing is persisted or published.
    for effects in [fx.tick(), fx.done(999, OpResult::RefsOk)] {
        assert!(effects.is_empty(), "{effects:#?}");
        assert_eq!((fx.state.revision, fx.run().revision), last);
    }
    // A signal from a window no run knows changes nothing either.
    let effects = fx.signal(4242, AgentSignal::Activity);
    assert!(effects.is_empty(), "{effects:#?}");
}

/// Review minor 6: a counter-only change is persisted lazily and published as a
/// counter update; a structural one is urgent.
#[test]
fn counter_only_changes_are_neither_urgent_nor_structural() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    for signal in [
        AgentSignal::ToolUse {
            name: "Bash".into(),
        },
        AgentSignal::Activity,
    ] {
        let effects = fx.signal(window, signal);
        assert_eq!(
            effects,
            vec![
                Effect::Persist {
                    run_id: RUN_ID.into(),
                    urgent: false
                },
                Effect::Publish { structural: false },
            ]
        );
    }
    let effects = fx.signal(
        window,
        AgentSignal::Init {
            session_id: "s-1".into(),
        },
    );
    assert!(effects.contains(&Effect::Persist {
        run_id: RUN_ID.into(),
        urgent: true
    }));
    assert!(effects.contains(&Effect::Publish { structural: true }));
}
