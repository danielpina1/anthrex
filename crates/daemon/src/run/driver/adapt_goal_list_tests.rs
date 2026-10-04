//! Task M9.5.10b (rulings RH-5, RL-3): a planned start's orchestrator through
//! `build_plan`. An explicit choice beats `[orchestrator.routes.orchestrator]`, the
//! list beats `[orchestrator.agent]`, and a candidate whose runtime the start's probe
//! found not installed is skipped. Never a real agent: Codex is a stand-in no build
//! launches, and Claude is that stand-in or a path that does not exist.

use super::*;

/// The default roster plus `gpt-6.1-sol`, and an `orchestrator` list of `candidates`;
/// `[orchestrator.agent]` names Claude's opus.
fn listed(candidates: Vec<config::Candidate>) -> config::Orchestrator {
    let mut config = config::Orchestrator::default();
    config.models.push(proto::ModelEntry {
        runtime: Runtime::Codex,
        model: "gpt-6.1-sol".into(),
        strength: proto::Strength::Frontier,
        note: String::new(),
    });
    config.agent.agent.runtime = Some(Runtime::Claude);
    config.agent.agent.model = "claude-opus-5-5".into();
    config.tuning.routes.orchestrator = config::RouteList {
        candidates,
        pick: config::Pick::First,
    };
    config
}

fn cand(runtime: Runtime, model: &str) -> config::Candidate {
    config::Candidate {
        runtime,
        model: model.into(),
        effort: None,
    }
}

#[tokio::test]
async fn a_planned_start_takes_the_orchestrator_list_below_an_explicit_choice() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    plain_repo(&root);
    let data = tmp.path().join("data");
    let config = listed(vec![cand(Runtime::Codex, "gpt-6.1-sol")]);
    let both = service(&data, config.clone(), INSTALLED_STAND_IN);

    // The list beats `[orchestrator.agent]`.
    let run = build(&both, &root, None).await.unwrap();
    let o = run.orch.orchestrator.as_ref().expect("an orchestrator");
    assert_eq!(
        (o.route.runtime, o.route.model.as_str()),
        (Runtime::Codex, "gpt-6.1-sol")
    );
    assert_eq!(o.routing.source, "configured_list");

    // An explicit choice beats the list.
    let run = build(&both, &root, Some(Runtime::Claude)).await.unwrap();
    let o = run.orch.orchestrator.as_ref().expect("an orchestrator");
    assert_eq!(o.route.runtime, Runtime::Claude);
    assert_eq!(o.routing.source, "explicit_choice");

    // A candidate not installed is skipped: `[orchestrator.agent]`'s Claude opus.
    let claude_list = listed(vec![
        cand(Runtime::Claude, "claude-sonnet-5"),
        cand(Runtime::Codex, "gpt-6.1-sol"),
    ]);
    let codex_only = service(&data, claude_list, NO_CLAUDE);
    let run = build(&codex_only, &root, None).await.unwrap();
    let o = run.orch.orchestrator.as_ref().expect("an orchestrator");
    assert_eq!(o.route.model, "gpt-6.1-sol");
    let first = &o.routing.candidates[0];
    assert_eq!(first.route.model, "claude-sonnet-5");
    assert_eq!(first.skipped_reason.as_deref(), Some("not installed"));
}
