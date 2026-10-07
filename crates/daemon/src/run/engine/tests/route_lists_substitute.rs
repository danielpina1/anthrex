//! Task M9.5.10a fix round 2 (ruling T10a-3) in the reducer: a route that failed in
//! this task for an environment reason is substituted by the next list candidate that
//! has not failed, whatever its strength, through `run retry` and through rung 2; with
//! every route failed, the original is retried and the run log says so.

use config::{Candidate, Pick, RouteList, RouteLists};
use proto::{Effort, ModelEntry, Route, Runtime, Strength, TaskState};

use super::control::retry;
use super::fixture::*;
use super::gates::CHECK_MODE;
use crate::headless::FailureKind;
use crate::run::engine::TurnOutcome;
use crate::run::refit::Tuned;

const GPT6_SOL: &str = "gpt-6-sol";

/// A working `t1` (S, `crates/a/**`) whose `s` list is [opus, codex/gpt-6-sol]; its
/// window. `only_listed` keeps only the two listed models in the roster.
fn working(only_listed: bool) -> (Fixture, u32) {
    let mut config = config::Orchestrator::default();
    config.models.push(ModelEntry {
        runtime: Runtime::Codex,
        model: GPT6_SOL.into(),
        strength: Strength::Standard,
        note: String::new(),
    });
    let cand = |runtime, model: &str| Candidate {
        runtime,
        model: model.into(),
        effort: None,
    };
    let lists = RouteLists {
        s: RouteList {
            candidates: vec![
                cand(Runtime::Claude, "claude-opus-5-5"),
                cand(Runtime::Codex, GPT6_SOL),
            ],
            pick: Pick::First,
        },
        ..Default::default()
    };
    let plan = plan_with(PROFILE, &[task("t1", "S", "a", CHECK_MODE)]);
    let mut fx = Fixture::with_config(&plan, config);
    fx.tuning = Tuned {
        lists,
        ..Tuned::default()
    };
    if only_listed {
        fx.start_with(true, |run| {
            run.roster
                .retain(|e| ["claude-opus-5-5", GPT6_SOL].contains(&e.model.as_str()))
        });
        let (op, _) = fx.op("CreateRunBranch");
        fx.done(
            op,
            crate::run::engine::OpResult::Worktree { head: BASE.into() },
        );
    } else {
        fx.ready(true);
    }
    let window = fx.launch_all()[0].1;
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(fx.task("t1").route.model, "claude-opus-5-5");
    (fx, window)
}

fn gpt6() -> Route {
    Route {
        runtime: Runtime::Codex,
        model: GPT6_SOL.into(),
        strength: Strength::Standard,
        effort: Effort::LOW,
    }
}

/// The substitute's runtime, model and strength (its effort is the list's business).
fn assert_same_model(got: &Route, want: &Route) {
    let key = |r: &Route| (r.runtime, r.model.clone(), r.strength);
    assert_eq!(key(got), key(want), "{got:?}");
}

fn client_error(fx: &mut Fixture, window: u32) {
    let failed = TurnOutcome::Failed {
        error: "credit balance is too low".into(),
        kind: FailureKind::ClientError,
    };
    fx.turn_ended(window, failed);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
}

#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn run_retry_substitutes_a_failed_route_with_a_weaker_candidate() {
    let (mut fx, window) = working(false);
    client_error(&mut fx, window);
    retry(&mut fx, "t1");
    assert_same_model(&fx.task("t1").route, &gpt6());
}

#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn rung_2_substitutes_a_failed_route_with_a_weaker_candidate() {
    let (mut fx, _) = working(false);
    let t1 = fx.task_mut("t1");
    let k = t1.rounds.len() - 1;
    t1.rounds[k].environment_failed = true;
    let mut effects = Vec::new();
    let now = fx.now;
    super::super::ladder::rung2(fx.run_mut(), 0, "a test".into(), now, &mut effects);
    assert_same_model(&fx.task("t1").route, &gpt6());
}

#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn with_every_route_failed_a_retry_takes_the_original_and_logs_it() {
    let (mut fx, window) = working(true);
    client_error(&mut fx, window);
    // gpt-6-sol failed in this task too, in an earlier session.
    let k = fx.task("t1").rounds.len() - 1;
    let mut earlier = fx.task("t1").rounds[k].clone();
    earlier.route = gpt6();
    fx.task_mut("t1").rounds.push(earlier);
    let original = fx.task("t1").route.clone();
    retry(&mut fx, "t1");
    assert_eq!(fx.task("t1").route, original);
    let line = "every route for task t1 failed in this task; retrying claude/claude-opus-5-5";
    assert!(
        fx.run().log.iter().any(|e| e.text == line),
        "{:#?}",
        fx.run().log
    );
}

/// A working `t1` on opus with no list and the default roster (codex holds only its
/// Standard default); its window.
fn working_unlisted() -> (Fixture, u32) {
    let plan = plan_with(PROFILE, &[task("t1", "S", "a", CHECK_MODE)]);
    let mut fx = Fixture::with_config(&plan, config::Orchestrator::default());
    fx.start_with(true, |run| run.tasks[0].route = opus());
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(
        op,
        crate::run::engine::OpResult::Worktree { head: BASE.into() },
    );
    let window = fx.launch_all()[0].1;
    assert_eq!(fx.task("t1").route, opus());
    (fx, window)
}

fn opus() -> Route {
    Route {
        runtime: Runtime::Claude,
        model: "claude-opus-5-5".into(),
        strength: Strength::Frontier,
        effort: Effort::MEDIUM,
    }
}

/// Ruling T10a-5: Codex's Standard default, the strongest route left, on the peer runtime.
fn codex_default() -> Route {
    Route {
        runtime: Runtime::Codex,
        model: String::new(),
        strength: Strength::Standard,
        effort: Effort::HIGH,
    }
}

#[test]
fn with_no_list_run_retry_substitutes_a_failed_route_from_the_roster() {
    let (mut fx, window) = working_unlisted();
    client_error(&mut fx, window);
    retry(&mut fx, "t1");
    assert_eq!(fx.task("t1").route, codex_default());
    assert!(
        !fx.run()
            .log
            .iter()
            .any(|e| e.text.starts_with("every route"))
    );
}

#[test]
fn with_no_list_rung_2_substitutes_a_failed_route_from_the_roster() {
    let (mut fx, _) = working_unlisted();
    let t1 = fx.task_mut("t1");
    let k = t1.rounds.len() - 1;
    t1.rounds[k].environment_failed = true;
    let mut effects = Vec::new();
    let now = fx.now;
    super::super::ladder::rung2(fx.run_mut(), 0, "a test".into(), now, &mut effects);
    assert_eq!(fx.task("t1").route, codex_default());
}

#[test]
fn with_every_roster_route_failed_a_retry_takes_the_original_and_logs_it() {
    let (mut fx, window) = working_unlisted();
    client_error(&mut fx, window);
    let k = fx.task("t1").rounds.len() - 1;
    for e in fx.run().roster.clone() {
        let mut earlier = fx.task("t1").rounds[k].clone();
        earlier.route = Route {
            runtime: e.runtime,
            model: e.model.clone(),
            strength: e.strength,
            effort: Effort::LOW,
        };
        fx.task_mut("t1").rounds.push(earlier);
    }
    retry(&mut fx, "t1");
    assert_eq!(fx.task("t1").route, opus());
    let line = "every route for task t1 failed in this task; retrying claude/claude-opus-5-5";
    assert!(fx.run().log.iter().any(|e| e.text == line));
}
