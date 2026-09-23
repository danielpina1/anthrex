//! M8a.11: plan edits as the scheduler sees them: implicit dependencies after a cancel,
//! a split or an added task, the clean-up of a cancelled task's worktree (M8a.6's F5),
//! and an answer held until new dependencies merge (M8a.6 ruling N5).

use std::path::PathBuf;

use proto::{BlockReason, PlanEdit, PlanTask, TaskState};

use super::dispatch::{edit, replies, task_path};
use super::fixture::*;
use crate::run::contract::answer_message;
use crate::run::engine::{AgentSignal, Effect, OpKind, OpResult, TurnOutcome};

/// One plan task, parsed from TOML exactly as a plan's would be.
fn plan_task(id: &str, owns: &str) -> PlanTask {
    let text = plan_with(PROFILE, &[task_toml(id, "S", owns, "")]);
    crate::run::plan::parse_plan(&text).unwrap().tasks.remove(0)
}

#[test]
fn cancelled_implicit_dep_releases_the_later_task() {
    let plan = plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", "[\"crates/a/**\"]", ""),
            task_toml("t2", "S", "[\"crates/a/src/**\"]", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(false);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1"]);
    fx.complete_prepares();
    let effects = edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    // The pre-warmed worktree of the cancelled task is salvaged and removed (decision
    // 14, M8a.6's F5), and t2 is released and pre-warmed in its place.
    let removes = ops_in(&effects, "RemoveWorktree");
    assert_eq!(removes.len(), 1, "{effects:#?}");
    assert!(effects.contains(&Effect::UnwatchWorktree {
        root: task_path("t1")
    }));
    assert_eq!(tasks_of(&effects, "PrepareWorktree"), vec!["t2"]);
    fx.complete_prepares();
    let effects = fx.approve();
    assert_eq!(tasks_of(&effects, "CreateWindow"), vec!["t2"]);
}

#[test]
fn cancel_of_a_rung3_blocked_task_salvages_its_worktree() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    let windows = fx.launch_all();
    let window = windows[0].1;
    // Rung 3 (M8a.12's): the session killed and gone, the worktree kept.
    fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: true,
            pid: 7,
        },
    );
    let task = fx.task_mut("t1");
    task.state = TaskState::Blocked;
    task.block = Some(proto::BlockInfo {
        reason: BlockReason::MisSized,
        text: "changed files outside owns: x.rs".into(),
    });
    for round in &mut task.rounds {
        round.ended = true;
    }
    let effects = edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    assert!(effects.contains(&Effect::UnwatchWorktree {
        root: task_path("t1")
    }));
    let removes = ops_in(&effects, "RemoveWorktree");
    assert_eq!(removes.len(), 1, "{effects:#?}");
    let OpKind::RemoveWorktree {
        root,
        path,
        salvage_ref,
    } = &removes[0].1
    else {
        unreachable!()
    };
    assert_eq!(root, &PathBuf::from("/tmp/x"));
    assert_eq!(path, &task_path("t1"));
    let reference = format!("refs/anthrex/salvage/{RUN_ID}/t1/1");
    assert_eq!(salvage_ref, &reference);
    fx.done(
        removes[0].0,
        OpResult::Removed {
            salvage_ref: Some(reference.clone()),
        },
    );
    assert_eq!(fx.task("t1").salvage_refs, vec![reference]);
    assert!(!fx.task("t1").worktree_live);
    fx.tick();
    assert_eq!(fx.ops("RemoveWorktree").len(), 1, "removed once");
}

#[test]
fn add_dep_then_answer_waits_for_the_dependency_then_hands_back() {
    let plan = plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", "[\"crates/a/**\"]", ""),
            task_toml("t2", "S", "[\"crates/a/src/**\"]", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    fx.signal(
        window,
        AgentSignal::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: None,
            denials: vec![],
        },
    );
    // t1 is started and blocked on a question (M8a.12's task_blocked).
    let task = fx.task_mut("t1");
    task.state = TaskState::Blocked;
    task.block = Some(proto::BlockInfo {
        reason: BlockReason::Question,
        text: "which table?".into(),
    });
    let effects = edit(
        &mut fx,
        vec![
            PlanEdit::AddDep {
                task_id: "t1".into(),
                dep: "t2".into(),
            },
            PlanEdit::Answer {
                task_id: "t1".into(),
                text: "the users table".into(),
            },
        ],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Deliver { .. })),
        "{effects:#?}"
    );
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    assert!(fx.task("t1").awaiting_deps);
    // t2 is no longer waiting on t1 and runs.
    assert_eq!(tasks_of(&effects, "PrepareWorktree"), vec!["t2"]);
    fx.launch_all();
    for state in [TaskState::Working, TaskState::Review] {
        let effects = fx.force("t2", state);
        assert!(!effects.iter().any(|e| matches!(e, Effect::Deliver { .. })));
        assert!(ops_in(&effects, "HandBack").is_empty());
        assert_eq!(fx.task("t1").state, TaskState::Blocked);
    }
    let head = "c2".repeat(20);
    let effects = fx.merge("t2", &head);
    let hand_backs = ops_in(&effects, "HandBack");
    assert_eq!(hand_backs.len(), 1, "{effects:#?}");
    let OpKind::HandBack { worktree, run_head } = &hand_backs[0].1 else {
        unreachable!()
    };
    assert_eq!(worktree, &task_path("t1"));
    assert_eq!(run_head, &head);
    assert!(!effects.iter().any(|e| matches!(e, Effect::Deliver { .. })));
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    let effects = fx.done(hand_backs[0].0, OpResult::HandedBack { files: vec![] });
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert!(!fx.task("t1").awaiting_deps);
    let delivered: Vec<(u32, String)> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Deliver {
                window_id, text, ..
            } => Some((*window_id, text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(delivered, vec![(window, answer_message("the users table"))]);
}

#[test]
fn a_started_task_is_always_the_one_waited_for() {
    // t2 (higher priority) works; t1 waits for the one writer slot.
    let plan = plan_with(
        &profile_with("max_writers = 1"),
        &[
            task_toml("t1", "S", "[\"crates/z/**\"]", ""),
            task_toml("t2", "S", "[\"crates/a/src/**\"]", "priority = 5"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    fx.launch_all();
    assert_eq!(fx.task("t2").state, TaskState::Working);
    // Split t1 into a child that overlaps t2 and sits before it in plan order.
    let effects = edit(
        &mut fx,
        vec![
            PlanEdit::SplitTask {
                task_id: "t1".into(),
                into: vec![
                    plan_task("t1a", "[\"crates/a/**\"]"),
                    plan_task("t1b", "[\"crates/z/**\"]"),
                ],
            },
            PlanEdit::AddTask {
                task: plan_task("t9", "[\"crates/a/src/x.rs\"]"),
            },
        ],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(fx.task("t1a").implicit_deps, vec!["t2"]);
    assert!(fx.task("t9").implicit_deps.contains(&"t2".to_string()));
    assert!(fx.task("t2").implicit_deps.is_empty());
    assert_eq!(fx.task("t2").state, TaskState::Working);
    assert_eq!(fx.task("t1a").state, TaskState::Pending);
    fx.force("t2", TaskState::Review);
    // The slot is free, but t1a still waits for t2; t1b runs.
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t2", "t1b"]);
    fx.merge("t2", &"c2".repeat(20));
    fx.launch_all();
    fx.merge("t1b", &"c3".repeat(20));
    assert!(tasks_of(&fx.log, "PrepareWorktree").contains(&"t1a".to_string()));
}

#[test]
fn cancel_of_a_working_task_kills_then_removes_after_the_exit() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    let effects = edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));
    assert!(
        ops_in(&effects, "RemoveWorktree").is_empty(),
        "the session is still live"
    );
    let effects = fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: true,
            pid: 7,
        },
    );
    assert_eq!(ops_in(&effects, "RemoveWorktree").len(), 1, "{effects:#?}");
}

/// Review minor 2: a pre-warmed task that gains a dependency no longer holds one of the
/// `max_writers` pre-warm places.
#[test]
fn a_prewarmed_task_that_gains_a_dependency_releases_its_prewarm_place() {
    let plan = plan_with(
        &profile_with("max_writers = 1"),
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(false);
    fx.complete_prepares();
    assert!(fx.task("t1").prewarmed);
    let effects = edit(
        &mut fx,
        vec![
            PlanEdit::AddTask {
                task: plan_task("t0", "[\"crates/z/**\"]"),
            },
            PlanEdit::AddDep {
                task_id: "t1".into(),
                dep: "t0".into(),
            },
        ],
    );
    assert_eq!(fx.task("t1").state, TaskState::Pending);
    assert_eq!(tasks_of(&effects, "PrepareWorktree"), vec!["t0"]);
}

/// Review minor 3: the clean-up of a cancelled task waits for its op in flight, and
/// never removes twice.
#[test]
fn a_cancel_waits_for_the_worktree_op_in_flight() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(false);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1"]);
    let effects = edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    assert!(
        ops_in(&effects, "RemoveWorktree").is_empty(),
        "{effects:#?}"
    );
    let effects = fx.complete_prepares();
    assert_eq!(ops_in(&effects, "RemoveWorktree").len(), 1, "{effects:#?}");
    // Nor is a second removal issued while the first runs.
    for _ in 0..2 {
        let effects = fx.tick();
        assert!(
            ops_in(&effects, "RemoveWorktree").is_empty(),
            "{effects:#?}"
        );
    }
}
