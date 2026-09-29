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
