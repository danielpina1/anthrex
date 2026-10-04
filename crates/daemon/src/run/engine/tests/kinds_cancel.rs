//! M9.9 review fixes, second round: `run cancel` and the `finish` edit halt the run's
//! sub-planners and run scouts (queued ones never start), and `run retry` of a research
//! or review task takes that kind's own fresh start, never a worker's.

use proto::{BlockReason, PlanEdit, RunState, TaskState, TokenUsage};
use serde_json::json;

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::kinds::{research, research_window, review, running, window_task};
use super::merge::pending;
use super::orch::{ORCH, orch_tool};
use super::planners::{PLANNER, epic, planner_ended, planning_mail};
use crate::run::engine::{Effect, EventKind, OpKind, OpResult, OrchEvent, ScoutEnd};
use crate::run::orch::{PlannerPhase, RunScoutState};

const CANCELLED: &str = "the run was cancelled";

pub(super) fn spawn_scout(fx: &mut Fixture, id: &str) {
    let args = json!({"id": id, "question": format!("What is {id}?"), "area": ["crates/api/**"]});
    let effects = orch_tool(fx, ORCH, "spawn_scout", args);
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
}

fn scout_state(fx: &Fixture, id: &str) -> RunScoutState {
    let full = format!("{}-{id}", fx.run().short());
    fx.run()
        .orch
        .run_scouts
        .iter()
        .find(|s| s.id == full)
        .unwrap()
        .state
        .clone()
}

/// A running run with `mail`'s sub-planner live, scout `api` running and scout `db`
/// queued behind it (two reader slots).
fn busy() -> Fixture {
    let mut fx = planning_mail(true);
    fx.run_mut().limits.max_readers = 2;
    spawn_scout(&mut fx, "api");
    spawn_scout(&mut fx, "db");
    let (op, _) = fx.op("StartScout");
    fx.done(op, OpResult::ScoutStarted { window_id: 77 });
    assert_eq!(scout_state(&fx, "api"), RunScoutState::Running);
    assert_eq!(scout_state(&fx, "db"), RunScoutState::Queued);
    assert_eq!(epic(&fx, "mail").phase, PlannerPhase::Planning);
    fx
}

pub(super) fn cancel(fx: &mut Fixture) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    })
}

/// Every op still pending answered as the executor would, and every killed window's
/// exit delivered, until nothing is left; then the ref guard.
pub(super) fn settle_all(fx: &mut Fixture) {
    for _ in 0..8 {
        let ops: Vec<_> = fx
            .run()
            .pending_ops
            .values()
            .map(|p| (p.op, p.kind.clone()))
            .collect();
        for (op, kind) in ops {
            let result = match kind {
                OpKind::PrepareWorktree { .. } => OpResult::Worktree { head: BASE.into() },
                OpKind::RemoveWorktree { .. } => OpResult::Removed {
                    salvage_ref: None,
                    cleared_locks: Vec::new(),
                },
                OpKind::VerifyRefs { .. } => OpResult::RefsOk,
                _ => OpResult::Failed {
                    message: "gone".into(),
                },
            };
            fx.done(op, result);
        }
        let killed: Vec<u32> = fx
            .log
            .iter()
            .filter_map(|e| match e {
                Effect::KillWindow { window_id } => Some(*window_id),
                _ => None,
            })
            .collect();
        for window in killed {
            fx.signal(
                window,
                crate::run::engine::AgentSignal::ProcessExited {
                    code: None,
                    killed_by_engine: true,
                    pid: 0,
                },
            );
        }
        if fx.run().state == RunState::Complete {
            return;
        }
    }
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
}

fn scout_ended(fx: &mut Fixture, id: &str) {
    let full = format!("{}-{id}", fx.run().short());
    fx.next(EventKind::Orch(OrchEvent::ScoutEnded {
        run_id: RUN_ID.into(),
        scout_id: full,
        outcome: ScoutEnd::Failed {
            reason: "halted".into(),
        },
        usage: TokenUsage::default(),
    }));
}

#[test]
fn cancel_halts_planners_and_scouts() {
    let mut fx = busy();
    let effects = cancel(&mut fx);
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    let api = format!("{}-api", fx.run().short());
    assert!(
        effects.contains(&Effect::StopPlanner {
            window_id: PLANNER,
            reason: CANCELLED.into(),
        }),
        "{effects:#?}"
    );
    assert!(
        effects.contains(&Effect::StopScout {
            scout_id: api,
            reason: CANCELLED.into(),
        }),
        "{effects:#?}"
    );
    // The queued scout never starts.
    assert!(ops_in(&effects, "StartScout").is_empty(), "{effects:#?}");
    let failed = |s: RunScoutState| {
        s == RunScoutState::Failed {
            reason: CANCELLED.into(),
        }
    };
    assert!(failed(scout_state(&fx, "api")));
    assert!(failed(scout_state(&fx, "db")));
    assert_eq!(
        epic(&fx, "mail").phase,
        PlannerPhase::Failed {
            reason: CANCELLED.into()
        }
    );
    // The sessions end; the run completes.
    planner_ended(
        &mut fx,
        ("mail", 1),
        ScoutEnd::Failed {
            reason: "halted".into(),
        },
    );
    scout_ended(&mut fx, "api");
    settle_all(&mut fx);
    assert!(pending(&fx, "StartScout", None).is_empty());
}

#[test]
fn the_finish_edit_halts_planners_and_scouts() {
    let mut fx = busy();
    let effects = edit(&mut fx, vec![PlanEdit::Finish]);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::StopPlanner { window_id, .. } if *window_id == PLANNER)),
        "{effects:#?}"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::StopScout { .. })),
        "{effects:#?}"
    );
    assert!(!epic(&fx, "mail").phase.is_live());
    assert!(!matches!(
        scout_state(&fx, "db"),
        RunScoutState::Queued | RunScoutState::Running
    ));
}

#[test]
fn retrying_a_research_task_starts_a_fresh_research_session() {
    let mut fx = running("", &[research("r1", "")]);
    fx.tick();
    let window = research_window(&mut fx, "r1");
    fx.turn_completed(window);
    fx.turn_completed(window);
    assert_eq!(fx.task("r1").state, TaskState::Blocked);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Retry {
        reply,
        run_id: RUN_ID.into(),
        task_id: "r1".into(),
    });
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    assert!(ops_in(&fx.log, "DiffSoFar").is_empty());
    assert!(ops_in(&fx.log, "PrepareWorktree").is_empty());
    let research_windows = fx
        .ops("CreateWindow")
        .iter()
        .filter(|(_, k)| window_task(k) == "r1")
        .count();
    assert_eq!(research_windows, 2);
    let task = fx.task("r1");
    assert_eq!(task.state, TaskState::Working);
    assert_eq!(task.block, None);
    assert_eq!(task.rounds.last().unwrap().role, proto::AgentRole::Scout);
}

#[test]
fn retrying_a_review_task_reviews_it_again() {
    let mut fx = running("", &[review("v1", "S")]);
    fx.tick();
    let (op, _) = fx.op("ResolveTarget");
    fx.done(
        op,
        OpResult::Failed {
            message: "no such ref".into(),
        },
    );
    let task = fx.task("v1");
    assert_eq!(task.state, TaskState::Blocked);
    assert_eq!(
        task.block.as_ref().map(|b| b.reason),
        Some(BlockReason::Environment)
    );
    let reply = fx.reply();
    let effects = fx.next(EventKind::Retry {
        reply,
        run_id: RUN_ID.into(),
        task_id: "v1".into(),
    });
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    assert!(ops_in(&fx.log, "DiffSoFar").is_empty());
    assert!(ops_in(&fx.log, "PrepareWorktree").is_empty());
    assert_eq!(fx.ops("ResolveTarget").len(), 2);
    assert_eq!(fx.task("v1").state, TaskState::Review);
}

#[test]
fn a_launch_in_flight_at_cancel_is_stopped_when_it_starts() {
    let mut fx = planning_mail(true);
    fx.run_mut().limits.max_readers = 2;
    spawn_scout(&mut fx, "api");
    let (scout_op, _) = fx.op("StartScout");
    let api = format!("{}-api", fx.run().short());
    // A scout whose window arrives after the cancel.
    cancel(&mut fx);
    let effects = fx.done(scout_op, OpResult::ScoutStarted { window_id: 78 });
    assert!(
        effects.contains(&Effect::StopScout {
            scout_id: api,
            reason: CANCELLED.into(),
        }),
        "{effects:#?}"
    );
    // A planner whose window arrives after the cancel.
    let mut fx = super::orch::launched(true);
    super::orch::edit_plan(
        &mut fx,
        json!({"edits": [super::orch::add("t1", "auth")], "submit": true}),
    );
    super::planners::spawn(&mut fx, "mail");
    let (op, _) = fx.op("StartPlanner");
    cancel(&mut fx);
    let effects = fx.done(op, OpResult::PlannerStarted { window_id: PLANNER });
    assert!(
        effects.contains(&Effect::StopPlanner {
            window_id: PLANNER,
            reason: CANCELLED.into(),
        }),
        "{effects:#?}"
    );
}
