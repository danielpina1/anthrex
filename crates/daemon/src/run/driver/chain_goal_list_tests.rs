//! Task M9.5.10b (rulings RH-5, RL-3): a continued chain keeps its orchestrator's route
//! over `[orchestrator.routes.orchestrator]`. The chain rig of `chain_goal_tests.rs`:
//! a real daemon socket and a stand-in `claude` that sleeps; no agent runs.

use super::Next;
use super::tests::{ChainRig, PREV};
use crate::live_config::LiveSettings;

#[tokio::test(flavor = "multi_thread")]
async fn a_continued_chain_keeps_its_route_over_the_orchestrator_list() {
    // The list names an installed runtime's other model, so taking it would show.
    let rig = ChainRig::with_context(
        // Off macOS a run's checks run only unconfined (as the sibling chain tests do).
        |prev| prev.limits.unconfined_checks = true,
        |ctx| {
            let mut config = ctx.settings.current().orchestrator.clone();
            config.tuning.routes.orchestrator = config::RouteList {
                candidates: vec![config::Candidate {
                    runtime: proto::Runtime::Claude,
                    model: "claude-sonnet-5".into(),
                    effort: None,
                }],
                pick: config::Pick::First,
            };
            ctx.settings = LiveSettings::defaults_of(config);
        },
    )
    .await;
    let prev = crate::lock(&rig.s.state).runs[PREV].clone();
    let chain_route = prev
        .orch
        .orchestrator
        .as_ref()
        .expect("the chain's")
        .route
        .clone();
    // The continued run as built, before the engine adopts the chain's window (an
    // adoption copies the session; a chain that ended launches on this route).
    let mut next = Next::inherited(&prev, "Add a logout button".into());
    next.dir = rig.checkout.work.clone();
    let run = rig.s.continued_run(PREV, next).await.expect("built");
    assert_eq!(run.limits.route_lists.orchestrator.candidates.len(), 1);
    let o = run.orch.orchestrator.as_ref().expect("an orchestrator");
    assert_eq!(
        (o.route.runtime, o.route.model.as_str()),
        (chain_route.runtime, chain_route.model.as_str())
    );
    assert_eq!(o.routing.source, "continued_chain");
    rig.stop().await;
}
