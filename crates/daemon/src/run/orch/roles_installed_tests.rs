//! M9.17 fix round 3: a sub-planner's and a run scout's record and route over the
//! runtimes the run's start found installed (`run.orch.installed`). A candidate on a
//! runtime that is not installed is recorded `not installed` (decision 43: a factual
//! reason), and a run scout of a Codex-only user is routed to Codex.

use std::collections::BTreeMap;
use std::path::PathBuf;

use proto::{Route, Runtime, Strength};

use super::*;
use crate::run::orch::test_support::{orchestrator, run_of, scout};
use crate::run::orch::{EpicRecord, PlannerPhase, RunScoutState};
use crate::scout::spec::{ScoutContext, ScoutRouting};

fn installed(claude: bool) -> BTreeMap<String, bool> {
    [("claude".to_string(), claude), ("codex".to_string(), true)].into()
}

/// `scout::spec::run_scout_route` on the scout service's live keys.
fn run_scout_route(ctx: &ScoutContext, installed: &BTreeMap<String, bool>) -> Route {
    crate::scout::spec::run_scout_route(&ctx.roster, &ScoutRouting::of(ctx), installed)
}

fn ctx() -> ScoutContext {
    let orchestrator = config::Orchestrator::default();
    ScoutContext {
        roster: config::default_roster(),
        default_runtime: Runtime::Claude,
        scouts: orchestrator.scouts.clone(),
        claude: orchestrator.claude.clone(),
        caps: crate::headless::argv::CLI_CAPS,
        data_dir: PathBuf::from("/nonexistent/anthrex-test/data"),
    }
}

/// A run with a Codex orchestrator, the built-in roster, `installed`, one epic whose
/// sub-planner is on `planner`, and run scout `s1`.
fn run_on(installed: BTreeMap<String, bool>, planner: Route) -> Run {
    let mut run = run_of(1);
    run.roster = config::default_roster();
    let mut o = orchestrator();
    o.route.runtime = Runtime::Codex;
    o.route.model = String::new();
    run.orch.orchestrator = Some(o);
    run.orch.installed = installed;
    let mut epic = EpicRecord::new("api", PlannerPhase::Planning);
    epic.route = planner;
    run.orch.epics.push(epic);
    run.orch
        .run_scouts
        .push(scout("s1", RunScoutState::Running, &["crates/api/**"]));
    run
}

fn reasons(d: &RoleRoutingDecision) -> Vec<(Runtime, String, Option<String>)> {
    d.candidates
        .iter()
        .map(|c| {
            (
                c.route.runtime,
                c.route.model.clone(),
                c.skipped_reason.clone(),
            )
        })
        .collect()
}

fn codex_route(strength: Strength) -> Route {
    Route {
        runtime: Runtime::Codex,
        model: String::new(),
        strength,
        effort: proto::Effort::High,
    }
}

#[test]
fn a_codex_only_planner_records_claude_as_not_installed() {
    let run = run_on(installed(false), codex_route(Strength::Standard));
    let d = planner_record(&run, 0, 1, 2_000);
    assert_eq!(d.chosen.runtime, Runtime::Codex);
    for (runtime, model, reason) in reasons(&d) {
        if runtime == Runtime::Claude {
            assert_eq!(reason.as_deref(), Some(NOT_INSTALLED), "{model}");
        }
    }
    assert!(reasons(&d).iter().any(|(r, _, _)| *r == Runtime::Claude));
    // With Claude installed, the peer's frontier model was the one taken, and nothing
    // is called not installed.
    let opus = Route {
        runtime: Runtime::Claude,
        model: "claude-opus-5-5".into(),
        strength: Strength::Frontier,
        effort: proto::Effort::High,
    };
    let run = run_on(installed(true), opus);
    let d = planner_record(&run, 0, 1, 2_000);
    assert!(
        reasons(&d)
            .iter()
            .all(|(_, _, r)| r.as_deref() != Some(NOT_INSTALLED)),
        "{:?}",
        reasons(&d)
    );
}

#[test]
fn a_codex_only_run_scout_is_routed_to_codex() {
    let ctx = ctx();
    let got = run_scout_route(&ctx, &installed(false));
    assert_eq!((got.runtime, got.model.as_str()), (Runtime::Codex, ""));
    let run = run_on(installed(false), codex_route(Strength::Standard));
    let d = scout_record(&run, "s1", 2_000);
    assert_eq!(d.chosen, got);
    let claude: Vec<_> = reasons(&d)
        .into_iter()
        .filter(|(r, _, _)| *r == Runtime::Claude)
        .collect();
    assert!(!claude.is_empty());
    assert!(
        claude
            .iter()
            .all(|(_, _, r)| r.as_deref() == Some(NOT_INSTALLED)),
        "{claude:?}"
    );
    // A strength only Claude has: the scout stays on Codex's default model.
    let mut frontier = ctx.clone();
    frontier.scouts.strength = Strength::Frontier;
    frontier.default_runtime = Runtime::Codex;
    assert_eq!(
        run_scout_route(&frontier, &installed(false)).runtime,
        Runtime::Codex
    );
    // `[orchestrator.scouts] runtime` pins it, installed or not.
    let mut pinned = ctx.clone();
    pinned.scouts.runtime = Some(Runtime::Claude);
    assert_eq!(
        run_scout_route(&pinned, &installed(false)).runtime,
        Runtime::Claude
    );
}

#[test]
fn a_run_scout_with_both_runtimes_is_routed_as_before() {
    let ctx = ctx();
    let before = crate::scout::spec::scout_route(&ctx);
    assert_eq!(run_scout_route(&ctx, &installed(true)), before);
    // Nothing recorded (a run from before the start check): as before.
    assert_eq!(run_scout_route(&ctx, &BTreeMap::new()), before);
    let run = run_on(installed(true), codex_route(Strength::Standard));
    let d = scout_record(&run, "s1", 2_000);
    assert_eq!(d.chosen, before);
    assert!(
        reasons(&d)
            .iter()
            .all(|(_, _, r)| r.as_deref() != Some(NOT_INSTALLED))
    );
}

/// Whole-branch review, item 1: a run's scouts keep the route keys its start froze and
/// checked; a config change after the start does not reroute them.
#[test]
fn a_config_change_after_the_start_does_not_reroute_a_runs_scouts() {
    let mut config = config::Orchestrator::default();
    config.scouts.runtime = Some(Runtime::Codex);
    let mut run = run_on(installed(true), codex_route(Strength::Standard));
    run.limits.orch = crate::run::orch::OrchLimits::from_config(&config);
    // The daemon's config now says Claude (`ctx()`); the routing takes the run alone.
    let d = scout_record(&run, "s1", 2_000);
    assert_eq!(d.chosen.runtime, Runtime::Codex);
    assert_eq!(
        crate::run::orch::launch::scout_route_of(&run).runtime,
        Runtime::Codex
    );
}

/// Whole-branch fix round 2, item 4: a run recorded before its scouts' keys were frozen
/// (`OrchLimits.scouts` absent) routes them on its own frozen `default_runtime` and the
/// default scout keys, never on the daemon's live config, so what its start checked
/// (and `reach` counts) is what launches.
#[test]
fn a_run_recorded_before_the_frozen_keys_never_reads_the_live_config() {
    let mut run = run_on(installed(true), codex_route(Strength::Standard));
    run.limits.default_runtime = Runtime::Claude;
    let mut json = serde_json::to_value(&run.limits.orch).unwrap();
    json.as_object_mut().unwrap().remove("scouts");
    run.limits.orch = serde_json::from_value(json).unwrap();
    assert_eq!(run.limits.orch.scouts, None);
    // Nothing here reads the daemon's config (the routing takes the run alone), so a
    // live config routing every scout to Codex cannot reach it.
    let route = crate::run::orch::launch::scout_route_of(&run);
    assert_eq!(route.runtime, Runtime::Claude);
    assert_eq!(route.strength, config::Scouts::default().strength);
    assert_eq!(scout_record(&run, "s1", 2_000).chosen, route);
    assert_eq!(crate::run::orch::launch::frozen_scout_route(&run), route);
}
