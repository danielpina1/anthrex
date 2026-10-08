//! Milestone 9.8 decision 29 in the reducer: rung 2 raises a task's effort along its
//! row's model list, then takes the row's fallback at its default effort
//! (`role_step::escalate`), and records the step from the role table (ruling F16).

use proto::models::{ModelRef, ModelTable, Role, RoleChoice};
use proto::{AgentRole, Effort, Route, Runtime, Strength, TaskState};

use super::control::retry;
use super::fixture::*;
use super::gates::{CHECK_MODE, only_op, working_with};
use crate::headless::FailureKind;
use crate::run::engine::{Effect, EventKind, OpResult, TurnOutcome};
use crate::run::model_roles::{ModelEfforts, RunModels};
use crate::run::routing::ROLES_POLICY;

const ROOMY: &str = "[task.budget]\ntool_calls = 1000\nminutes = 1000";
const SOL: &str = "codex:gpt-6-sol";
const OPUS: &str = "claude:claude-opus-5-5";

fn m(s: &str) -> ModelRef {
    ModelRef::parse(s).unwrap()
}

fn at(model: &str, effort: &str) -> Route {
    RunModels::route_of(&m(model), (!effort.is_empty()).then_some(effort))
}

/// A working M task `t1` on the `implementer.medium` row `gpt-6-sol` at `medium`,
/// falling back to Opus, with the catalog's list `low`, `medium`, `high` frozen for
/// Sol (decision 21); its window.
fn working() -> (Fixture, u32) {
    let mut config = config::Orchestrator::default();
    let row = RoleChoice {
        model: m(SOL),
        effort: Some("medium".into()),
        fallback: Some(m(OPUS)),
    };
    config.roles = ModelTable {
        rows: [(Role::ImplementerMedium, row)].into(),
        brainstorm: None,
    };
    let mut fx = Fixture::with_config(&plan_with(PROFILE, &[task("t1", "M", "a", ROOMY)]), config);
    fx.ready(true);
    let models = fx
        .run_mut()
        .limits
        .models
        .as_mut()
        .expect("frozen at start");
    let efforts = ModelEfforts {
        efforts: vec!["low".into(), "medium".into(), "high".into()],
        default: Some("medium".into()),
    };
    models.efforts.insert(m(SOL), efforts);
    let window = fx.launch_all()[0].1;
    (fx, window)
}

/// The session in `window` stalls past its interrupt's grace (rung 2), exits, and its
/// fresh successor starts; the successor's window.
fn stall(fx: &mut Fixture, window: u32) -> u32 {
    let quiet = fx.task("t1").rounds.last().expect("a session").last_event;
    fx.send(quiet + 601, EventKind::Tick);
    let effects = fx.send(quiet + 632, EventKind::Tick);
    assert!(
        effects.contains(&Effect::KillWindow { window_id: window }),
        "{effects:#?}"
    );
    assert_eq!(fx.task("t1").rung, 2);
    let effects = super::turns::killed_exit(fx, window);
    let (op, _) = only_op(&effects, "DiffSoFar");
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    only_op(&effects, "CreateWindow");
    fx.complete_windows()[0].1
}

#[test]
fn rung_two_raises_the_effort_then_takes_the_fallback() {
    let (mut fx, window) = working();
    assert_eq!(fx.task("t1").rounds[0].route, at(SOL, "medium"));
    let window = stall(&mut fx, window);
    let t1 = fx.task("t1");
    assert_eq!((t1.session, &t1.rounds[1].route), (2, &at(SOL, "high")));
    stall(&mut fx, window);
    let t1 = fx.task("t1");
    assert_eq!((t1.session, &t1.rounds[2].route), (3, &at(OPUS, "")));
    // Ruling F16: each escalation records the role table's steps from the route it left.
    let escalations: Vec<_> = (t1.routing_decisions.iter())
        .filter(|d| d.role == AgentRole::Worker && d.trigger == "escalation")
        .collect();
    assert_eq!(escalations.len(), 2, "{escalations:#?}");
    let d = escalations[1];
    assert_eq!(
        (d.policy_version.as_str(), &d.chosen),
        (ROLES_POLICY, &at(OPUS, ""))
    );
    let pool: Vec<_> = d.candidates.iter().map(|c| c.route.clone()).collect();
    assert_eq!(pool[..2], [at(OPUS, ""), at(SOL, "high")], "{pool:#?}");
}

/// M9.8.8 fix round 1 (controller ruling): the built-in table frozen with no catalog
/// in memory (a first run, discovery lazy, decision 20) takes the built-in effort
/// lists, so a stalled `implementer.medium` task climbs Sonnet's effort.
#[test]
fn a_default_table_with_no_catalog_climbs_effort() {
    let plan = plan_with(PROFILE, &[task("t1", "M", "a", ROOMY)]);
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let mut models = RunModels::resolve(&ModelTable::default(), None);
    assert!(
        models.validate(&[]).is_empty(),
        "nothing to validate against"
    );
    fx.run_mut().limits.models = Some(models);
    let window = fx.launch_all()[0].1;
    let sonnet = "claude:claude-sonnet-5";
    assert_eq!(fx.task("t1").rounds[0].route, at(sonnet, "medium"));
    stall(&mut fx, window);
    assert_eq!(fx.task("t1").rounds[1].route, at(sonnet, "high"));
}

// M9.8.13 fix round 1 (review I2): restored from `route_lists_substitute.rs` and
// `route_lists_failed.rs`, deleted with them; they drive `run retry` and rung 2 over a
// route that failed in the task, and the "every route … failed" log line.

/// The default config, its `implementer.small` row (Sonnet at `low`) falling back to
/// Codex's default.
fn small_falls_back_to_codex() -> config::Orchestrator {
    let mut config = config::Orchestrator::default();
    let row = RoleChoice {
        model: m("claude:claude-sonnet-5"),
        effort: Some("low".into()),
        fallback: Some(m("codex:default")),
    };
    (config.roles.rows).insert(Role::ImplementerSmall, row);
    config
}

/// A working S task `t1` started on Opus, its row ([`small_falls_back_to_codex`])
/// falling back to Codex's default; its window.
fn working_on_opus() -> (Fixture, u32) {
    let plan = plan_with(PROFILE, &[task("t1", "S", "a", CHECK_MODE)]);
    let mut fx = Fixture::with_config(&plan, small_falls_back_to_codex());
    fx.start_with(true, |run| run.tasks[0].route = opus());
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
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

/// Decision 29: the row's fallback, Codex's default, at its default effort.
fn codex_default() -> Route {
    Route {
        runtime: Runtime::Codex,
        model: String::new(),
        strength: Strength::Standard,
        effort: Effort::DEFAULT,
    }
}

fn client_error(fx: &mut Fixture, window: u32, error: &str) {
    let failed = TurnOutcome::Failed {
        error: error.into(),
        kind: FailureKind::ClientError,
    };
    fx.turn_ended(window, failed);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    assert!(fx.task("t1").rounds[0].environment_failed);
}

#[test]
fn run_retry_substitutes_a_failed_route_with_the_rows_fallback() {
    let (mut fx, window) = working_on_opus();
    client_error(&mut fx, window, "credit balance is too low");
    retry(&mut fx, "t1");
    assert_eq!(fx.task("t1").route, codex_default());
    let logged = fx
        .run()
        .log
        .iter()
        .any(|e| e.text.starts_with("every route"));
    assert!(!logged, "{:#?}", fx.run().log);
}

#[test]
fn rung_2_substitutes_a_failed_route_with_the_rows_fallback() {
    let (mut fx, _) = working_on_opus();
    let t1 = fx.task_mut("t1");
    let k = t1.rounds.len() - 1;
    t1.rounds[k].environment_failed = true;
    let mut effects = Vec::new();
    let now = fx.now;
    super::super::ladder::rung2(fx.run_mut(), 0, "a test".into(), now, &mut effects);
    assert_eq!(fx.task("t1").route, codex_default());
}

/// Every route the role table reaches from Opus (the row's own model and its fallback)
/// failed in this task too, in earlier sessions: a retry takes the original and the run
/// log says so.
#[test]
fn with_every_route_failed_a_retry_takes_the_original_and_logs_it() {
    let (mut fx, window) = working_on_opus();
    client_error(&mut fx, window, "credit balance is too low");
    let k = fx.task("t1").rounds.len() - 1;
    let sonnet = at("claude:claude-sonnet-5", "low");
    for route in [sonnet, codex_default()] {
        let mut earlier = fx.task("t1").rounds[k].clone();
        earlier.route = route;
        fx.task_mut("t1").rounds.push(earlier);
    }
    retry(&mut fx, "t1");
    assert_eq!(fx.task("t1").route, opus());
    let line = "every route for task t1 failed in this task; retrying claude/claude-opus-5-5";
    let logged = fx.run().log.iter().any(|e| e.text == line);
    assert!(logged, "{:#?}", fx.run().log);
}

/// The action menu's retry preview names the route a retry takes: here a worker whose
/// own route failed with a client error, so no higher effort on that model, but its
/// row's fallback, Codex's default, at its default effort (decision 29).
#[test]
fn the_retry_preview_names_the_route_a_retry_takes() {
    use crate::run::engine::actions::{self, ActionNode};
    let (mut fx, window) = working_with(PROFILE, CHECK_MODE, small_falls_back_to_codex());
    client_error(&mut fx, window, "model not found");
    let preview = actions::available(fx.run(), &ActionNode::Task("t1"))
        .into_iter()
        .find(|a| a.kind == proto::ActionKind::Retry)
        .expect("retry is listed")
        .effect;
    assert_eq!(
        preview,
        "retry t1: a fresh session at rung 2 on codex default (default effort)"
    );
    retry(&mut fx, "t1");
    let route = &fx.task("t1").route;
    assert_eq!((route.runtime, route.model.as_str()), (Runtime::Codex, ""));
}
