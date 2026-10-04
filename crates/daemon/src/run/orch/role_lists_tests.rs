//! Task M9.5.10b: decision 9a for the run's roles. Run scouts spread over their list in
//! start order, the role records keep the list, its pick and the rotation, and the
//! orchestrator's precedence is an explicit choice, then the list, then
//! `[orchestrator.agent]` (rulings RH-5, RL-3).

use config::{Candidate, Pick, RouteList, RouteLists};
use proto::{Effort, ModelEntry, OrchestratorChoice, Runtime, Strength};

use super::*;
use crate::run::model::RouteListsFrozen;
use crate::run::orch::installed::resolve_installed;
use crate::run::orch::launch::{frozen_scout_route, planner_route, scout_route_of};
use crate::run::orch::roles::{
    EARLIER_TAKEN, NOT_INSTALLED, RoleSnapshot, orchestrator_record, planner_record, scout_record,
};
use crate::run::orch::test_support::{orchestrator as record_of, run_of, scout};
use crate::run::orch::{EpicRecord, PlannerPhase, RunScoutState};

const SOL: &str = "gpt-6.1-sol";
const LUNA: &str = "gpt-6-luna";
const HAIKU: &str = "claude-haiku-4-5";

fn roster() -> Vec<ModelEntry> {
    let mut roster = config::default_roster();
    for (model, strength) in [(SOL, Strength::Frontier), (LUNA, Strength::Fast)] {
        roster.push(ModelEntry {
            runtime: Runtime::Codex,
            model: model.into(),
            strength,
            note: String::new(),
        });
    }
    roster
}

fn cand(runtime: Runtime, model: &str, effort: Option<Effort>) -> Candidate {
    Candidate {
        runtime,
        model: model.into(),
        effort,
    }
}

fn route(runtime: Runtime, model: &str, strength: Strength, effort: Effort) -> Route {
    Route {
        runtime,
        model: model.into(),
        strength,
        effort,
    }
}

/// `scout` = spread [codex/gpt-6-luna, claude/claude-haiku-4-5 low]; `planner` and
/// `orchestrator` = [codex/gpt-6.1-sol].
fn lists() -> RouteListsFrozen {
    let sol = || RouteList {
        candidates: vec![cand(Runtime::Codex, SOL, None)],
        pick: Pick::First,
    };
    let lists = RouteLists {
        scout: RouteList {
            candidates: vec![
                cand(Runtime::Codex, LUNA, None),
                cand(Runtime::Claude, HAIKU, Some(Effort::Low)),
            ],
            pick: Pick::Spread,
        },
        planner: sol(),
        orchestrator: sol(),
        ..Default::default()
    };
    RouteListsFrozen::freeze(&lists, &roster())
}

/// A run with an orchestrator, the lists, and run scouts s1, s2 and s3.
fn listed_run() -> Run {
    let mut run = run_of(1);
    run.roster = roster();
    run.limits.route_lists = lists();
    run.orch.orchestrator = Some(record_of());
    for id in ["s1", "s2", "s3"] {
        let s = scout(id, RunScoutState::Running, &["crates/api/**"]);
        run.orch.run_scouts.push(s);
    }
    run
}

#[test]
fn run_scouts_spread_over_their_list_in_start_order() {
    let run = listed_run();
    let effort = run
        .limits
        .orch
        .scouts
        .as_ref()
        .map_or(Effort::Medium, |s| s.effort);
    let luna = route(Runtime::Codex, LUNA, Strength::Fast, effort);
    let haiku = route(Runtime::Claude, HAIKU, Strength::Fast, Effort::Low);
    let routes: Vec<Route> = ["s1", "s2", "s3"]
        .into_iter()
        .map(|id| scout_route_of(&run, id))
        .collect();
    assert_eq!(routes, [luna.clone(), haiku.clone(), luna]);
    // Codex not installed: every scout on Claude.
    let mut codexless = run.clone();
    codexless.orch.installed = [("codex".to_string(), false)].into();
    assert_eq!(scout_route_of(&codexless, "s1"), haiku);
    // With no list, today's route.
    let mut plain = run.clone();
    plain.limits.route_lists = RouteListsFrozen::default();
    assert_eq!(scout_route_of(&plain, "s2"), frozen_scout_route(&plain));
}

#[test]
fn role_routing_history_keeps_the_list_and_rotation() {
    let mut run = listed_run();
    // A run scout: the whole list, the chosen candidate, the pick and the rotation.
    let d = scout_record(&run, "s2", 100);
    let reasons: Vec<Option<&str>> = (d.candidates.iter())
        .map(|c| c.skipped_reason.as_deref())
        .collect();
    assert_eq!(reasons, [Some(EARLIER_TAKEN), None]);
    assert_eq!(
        (d.source.as_str(), d.policy_version.as_str()),
        (LIST_SOURCE, LIST_POLICY)
    );
    assert_eq!(
        (d.selected_index, d.pick_policy.as_deref(), d.rotation),
        (1, Some("spread"), Some(1))
    );
    // A sub-planner on the `first` list: no rotation.
    let planner = planner_route(&run).expect("an orchestrator");
    assert_eq!(planner.model, SOL);
    let mut epic = EpicRecord::new("api", PlannerPhase::Planning);
    epic.route = planner;
    run.orch.epics.push(epic);
    let d = planner_record(&run, 0, 1, 100);
    assert_eq!((d.source.as_str(), d.selected_index), (LIST_SOURCE, 0));
    assert_eq!(
        (d.pick_policy.as_deref(), d.rotation),
        (Some("first"), None)
    );
    // The orchestrator, resolved over the list.
    let agent = config::AgentConfig::default();
    let resolved = resolve(None, &run, &agent, &|_| None).expect("resolves");
    let mut o = record_of();
    o.route = resolved.route;
    o.routing = RoleSnapshot {
        source: resolved.source,
        candidates: resolved.candidates,
    };
    run.orch.orchestrator = Some(o);
    let d = orchestrator_record(&run, "start", 100).expect("an orchestrator");
    assert_eq!(
        (d.source.as_str(), d.policy_version.as_str()),
        (LIST_SOURCE, LIST_POLICY)
    );
    assert_eq!(d.pick_policy.as_deref(), Some("first"));
    assert_eq!(d.chosen.model, SOL);

    // A reloaded run records the same: every pick reads the run's own frozen lists.
    let back: Run = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
    assert_eq!(
        scout_record(&back, "s2", 100),
        scout_record(&run, "s2", 100)
    );
    // Not installed: recorded as such, and the next candidate taken.
    run.orch.installed = [("codex".to_string(), false)].into();
    let d = scout_record(&run, "s1", 100);
    assert_eq!(
        d.candidates[0].skipped_reason.as_deref(),
        Some(NOT_INSTALLED)
    );
    assert_eq!(d.chosen.model, HAIKU);
}

/// Decision 6 through the orchestrator's precedence, as `make_planned` resolves it.
fn resolve(
    choice: Option<&OrchestratorChoice>,
    run: &Run,
    agent: &config::AgentConfig,
    missing: crate::run::orch::installed::Missing<'_>,
) -> Result<Resolved, String> {
    let list = &run.limits.route_lists.orchestrator;
    let installed: Installed = [Runtime::Claude, Runtime::Codex]
        .into_iter()
        .map(|r| (r.label().to_string(), missing(r).is_none()))
        .collect();
    let today = || resolve_installed(choice, agent, Runtime::Claude, &run.roster, missing);
    orchestrator(choice, list, agent.effort, &installed, today)
}

#[test]
fn the_orchestrator_precedence() {
    let run = listed_run();
    let sol = route(Runtime::Codex, SOL, Strength::Frontier, Effort::High);
    let nothing_missing = |_: Runtime| None;
    // The list beats `[orchestrator.agent]`, even one that names a runtime and model.
    let agent = config::AgentConfig {
        runtime: Some(Runtime::Claude),
        model: "claude-opus-5-5".into(),
        effort: Effort::High,
    };
    let got = resolve(None, &run, &agent, &nothing_missing).unwrap();
    assert_eq!((got.route, got.source.as_str()), (sol, LIST_SOURCE));
    // An explicit choice beats the list; so does a continued chain's route, which its
    // continuation passes as the choice (`chain_goal.rs`).
    let choice = OrchestratorChoice {
        runtime: Runtime::Claude,
        model: Some("claude-sonnet-5".into()),
    };
    let got = resolve(Some(&choice), &run, &agent, &nothing_missing).unwrap();
    assert_eq!(got.route.model, "claude-sonnet-5");
    assert_eq!(got.source, "explicit_choice");
    // A list whose every candidate is not installed leaves `[orchestrator.agent]`, the
    // skipped candidate leading the snapshot.
    let codex_missing = |r: Runtime| (r == Runtime::Codex).then(|| "/nonexistent/codex".into());
    let got = resolve(None, &run, &agent, &codex_missing).unwrap();
    assert_eq!(
        (got.route.model.as_str(), got.source.as_str()),
        ("claude-opus-5-5", "agent_config")
    );
    assert_eq!(got.candidates[0].route.model, SOL);
    assert_eq!(
        got.candidates[0].skipped_reason.as_deref(),
        Some(NOT_INSTALLED)
    );
    // No list: `[orchestrator.agent]` as before.
    let mut plain = run.clone();
    plain.limits.route_lists = RouteListsFrozen::default();
    let got = resolve(None, &plain, &agent, &nothing_missing).unwrap();
    let before = resolve_installed(
        None,
        &agent,
        Runtime::Claude,
        &plain.roster,
        &nothing_missing,
    );
    assert_eq!(Ok(got), before);
}
