//! Milestone 9.8 decision 29 in the reducer: rung 2 raises a task's effort along its
//! row's model list, then takes the row's fallback at its default effort
//! (`role_step::escalate`), and records the step from the role table (ruling F16).

use proto::models::{ModelRef, ModelTable, Role, RoleChoice};
use proto::{AgentRole, Route};

use super::fixture::*;
use super::gates::only_op;
use crate::run::engine::{Effect, EventKind, OpResult};
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
