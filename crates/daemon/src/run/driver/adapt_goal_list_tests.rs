//! Task M9.5.10b (rulings RH-5, RL-3), milestone 9.8: a planned start's orchestrator
//! through `build_plan`. An explicit choice beats the `orchestrator` row, which beats
//! the old `[orchestrator.agent]` keys (a row names the model whole). Never a real
//! agent: Codex is a stand-in no build launches, and Claude is that stand-in.

use super::*;

#[tokio::test]
async fn a_planned_start_takes_the_orchestrator_row_below_an_explicit_choice() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    plain_repo(&root);
    let data = tmp.path().join("data");
    let config = parsed(
        "[orchestrator.agent]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"\n\n[models.orchestrator]\nmodel = \"codex:gpt-6.1-sol\"\neffort = \"high\"\n",
    );
    let both = service(&data, config, INSTALLED_STAND_IN);

    // The row beats `[orchestrator.agent]`.
    let run = build(&both, &root, None).await.unwrap();
    let o = run.orch.orchestrator.as_ref().expect("an orchestrator");
    assert_eq!(
        (
            o.route.runtime,
            o.route.model.as_str(),
            o.route.effort.as_str()
        ),
        (Runtime::Codex, "gpt-6.1-sol", "high")
    );
    assert_eq!(o.routing.source, "role_table");

    // An explicit choice beats the row. Fix round 1 (I2): a runtime-only choice off the
    // row's runtime is that runtime's built-in orchestrator, never the CLI's default.
    let run = build(&both, &root, Some(Runtime::Claude)).await.unwrap();
    let o = run.orch.orchestrator.as_ref().expect("an orchestrator");
    assert_eq!(
        (o.route.runtime, o.route.model.as_str()),
        (Runtime::Claude, "claude-opus-5-5")
    );
    assert_eq!(o.routing.source, "explicit_choice");
}
