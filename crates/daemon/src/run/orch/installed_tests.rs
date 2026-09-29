use proto::{Effort, OrchestratorChoice, Runtime};

use super::*;
use crate::run::orch::roles::NOT_INSTALLED;

fn agent(runtime: Option<Runtime>) -> config::AgentConfig {
    config::AgentConfig {
        runtime,
        model: String::new(),
        effort: Effort::High,
    }
}

/// Only `absent`'s binary is missing.
fn without(absent: &[Runtime]) -> impl Fn(Runtime) -> Option<String> {
    let absent = absent.to_vec();
    move |r: Runtime| {
        absent
            .contains(&r)
            .then(|| format!("/nonexistent/{}", r.label()))
    }
}

#[test]
fn an_installed_runtime_resolves_as_before() {
    let roster = config::default_roster();
    let missing = without(&[Runtime::Codex]);
    let got = resolve_installed(None, &agent(None), Runtime::Claude, &roster, &missing);
    let plain = resolve_orchestrator(None, &agent(None), Runtime::Claude, &roster);
    assert_eq!(got, plain);
}

#[test]
fn a_default_runtime_that_is_not_installed_is_skipped_for_its_peer() {
    let roster = config::default_roster();
    let missing = without(&[Runtime::Claude]);
    let got = resolve_installed(None, &agent(None), Runtime::Claude, &roster, &missing).unwrap();
    assert_eq!(got.route.runtime, Runtime::Codex);
    assert_eq!(got.source, "roster_default");
    let peer = resolve_orchestrator(None, &agent(None), Runtime::Codex, &roster).unwrap();
    assert_eq!(got.route, peer.route);
    // Every Claude entry first, each skipped as not installed; then the peer's own
    // snapshot, unchanged. No candidate is called a failure.
    let claude: Vec<_> = roster
        .iter()
        .filter(|e| e.runtime == Runtime::Claude)
        .collect();
    assert_eq!(got.candidates.len(), claude.len() + peer.candidates.len());
    for (c, e) in got.candidates.iter().zip(&claude) {
        assert_eq!(c.route.runtime, Runtime::Claude);
        assert_eq!(c.route.model, e.model);
        assert_eq!(c.route.effort, Effort::High);
        assert_eq!(c.skipped_reason.as_deref(), Some(NOT_INSTALLED));
    }
    assert_eq!(got.candidates[claude.len()..], peer.candidates[..]);
    assert!(
        got.candidates
            .iter()
            .any(|c| c.route == got.route && c.skipped_reason.is_none())
    );
}

#[test]
fn a_chosen_or_configured_runtime_that_is_not_installed_refuses_the_start() {
    let roster = config::default_roster();
    let missing = without(&[Runtime::Codex]);
    let choice = OrchestratorChoice {
        runtime: Runtime::Codex,
        model: None,
    };
    let refused = |r: Result<Resolved, String>| r.expect_err("the start is refused");
    for text in [
        refused(resolve_installed(
            Some(&choice),
            &agent(None),
            Runtime::Claude,
            &roster,
            &missing,
        )),
        refused(resolve_installed(
            None,
            &agent(Some(Runtime::Codex)),
            Runtime::Claude,
            &roster,
            &missing,
        )),
    ] {
        assert_eq!(
            text,
            "the orchestrator's runtime codex is not installed (/nonexistent/codex is not an \
             executable file); install it, or choose another runtime with --orchestrator"
        );
    }
}

#[test]
fn neither_runtime_installed_refuses_naming_the_default() {
    let roster = config::default_roster();
    let missing = without(&[Runtime::Claude, Runtime::Codex]);
    let text = resolve_installed(None, &agent(None), Runtime::Claude, &roster, &missing)
        .expect_err("refused");
    assert!(
        text.starts_with("the orchestrator's runtime claude is not installed (/nonexistent/claude"),
        "{text}"
    );
}

#[test]
fn not_installed_names_who_the_runtime_and_the_binary() {
    let missing = without(&[Runtime::Codex]);
    assert_eq!(
        not_installed("the sub-planners'", Runtime::Claude, &missing, "x"),
        None
    );
    assert_eq!(
        not_installed(
            "the sub-planners'",
            Runtime::Codex,
            &missing,
            "change [orchestrator.planners] runtime"
        )
        .as_deref(),
        Some(
            "the sub-planners' runtime codex is not installed (/nonexistent/codex is not an \
             executable file); install it, or change [orchestrator.planners] runtime"
        )
    );
}

/// A run whose orchestrator is on `runtime`, with the built-in roster.
fn run_on(runtime: Runtime) -> crate::run::model::Run {
    let mut run = crate::run::orch::test_support::run_of(1);
    run.roster = config::default_roster();
    let mut record = crate::run::orch::test_support::orchestrator();
    record.route.runtime = runtime;
    run.orch.orchestrator = Some(record);
    run
}

/// M9.17 fix round 2: the sub-planners' route (frontier by default) steps to the peer
/// runtime only when the peer is installed or not known; the built-in roster has no
/// Codex frontier entry, so a Codex-only user's planners stay on Codex.
#[test]
fn the_planner_route_steps_to_the_peer_only_when_it_is_installed() {
    use crate::run::orch::launch::planner_route;
    let mut run = run_on(Runtime::Codex);
    // Not known (a run from before the start check): M8b's route, the peer's frontier.
    assert_eq!(planner_route(&run).unwrap().runtime, Runtime::Claude);
    run.orch.installed = [("claude".to_string(), true), ("codex".to_string(), true)].into();
    assert_eq!(planner_route(&run).unwrap().runtime, Runtime::Claude);
    run.orch.installed.insert("claude".into(), false);
    let route = planner_route(&run).unwrap();
    assert_eq!(route.runtime, Runtime::Codex);
    assert_eq!(route.model, "");
    // Configured on Codex, the same.
    run.limits.orch.planners.runtime = Some(Runtime::Codex);
    assert_eq!(planner_route(&run).unwrap().runtime, Runtime::Codex);
    // A Claude orchestrator with both installed keeps its frontier planner.
    let mut run = run_on(Runtime::Claude);
    run.orch.installed = [("claude".to_string(), true), ("codex".to_string(), true)].into();
    let route = planner_route(&run).unwrap();
    assert_eq!(
        (route.runtime, route.model.as_str()),
        (Runtime::Claude, "claude-opus-5-5")
    );
}

/// M9.17 fix round 2, item 6: a skipped runtime with no roster entry is still recorded,
/// as its one default-model candidate, `not installed`.
#[test]
fn a_skipped_runtime_with_no_roster_entry_is_recorded() {
    let roster: Vec<_> = config::default_roster()
        .into_iter()
        .filter(|e| e.runtime == Runtime::Claude)
        .collect();
    let missing = without(&[Runtime::Codex]);
    let got = resolve_installed(None, &agent(None), Runtime::Codex, &roster, &missing).unwrap();
    assert_eq!(got.route.runtime, Runtime::Claude);
    let first = &got.candidates[0];
    assert_eq!(
        (first.route.runtime, first.route.model.as_str()),
        (Runtime::Codex, "")
    );
    assert_eq!(first.skipped_reason.as_deref(), Some(NOT_INSTALLED));
}
