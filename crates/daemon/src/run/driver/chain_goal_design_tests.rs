//! Milestone 9.6 task M9.6.15 (decision 30, DF §8.2): a continued goal's design flow.
//! The request's `design` reaches the build (no longer dropped by `requests.rs`'s
//! pattern); with none, a continued goal, never triaged, takes the configured default
//! like any planned code goal, so `start_goal`'s goal is `full` by default; and the
//! reply names the state it really starts in. The chain rig of `chain_goal_tests.rs`:
//! a real daemon socket and a stand-in `claude` that sleeps; no agent runs.

use std::path::Path;

use proto::{DeliveryMode, DesignMode, RunReply, RunRequest, RunState};

use super::Next;
use super::tests::{ChainRig, PREV};
use crate::live_config::LiveSettings;
use crate::run::driver::RunContext;

const ANSWER: std::time::Duration = std::time::Duration::from_secs(30);

/// The rig with `[orchestrator.design] default = "full"`.
async fn full_by_default() -> ChainRig {
    rig_with(DesignMode::Full).await
}

/// The rig with `[orchestrator.design] default` `default` and no documents folder (so
/// the start reads no symlink and nothing is committed).
async fn rig_with(default: DesignMode) -> ChainRig {
    ChainRig::with_context(
        // Off macOS a run's checks run only unconfined (as the sibling chain tests do).
        |prev| prev.limits.unconfined_checks = true,
        |ctx: &mut RunContext| {
            let mut config = ctx.settings.current().orchestrator.clone();
            config.design.default = default;
            config.design.docs_dir = String::new();
            ctx.settings = LiveSettings::defaults_of(config);
        },
    )
    .await
}

/// A goal continuing [`PREV`] from `dir` with `design`, as `run start --continue`
/// sends it.
async fn continued(rig: &ChainRig, dir: &Path, design: Option<DesignMode>) -> RunReply {
    let req = RunRequest::StartGoal {
        goal: "Add a logout button".into(),
        dir: dir.to_path_buf(),
        yes: false,
        trust_project: false,
        unconfined_checks: true,
        orchestrator: None,
        delivery: Some(DeliveryMode::Local),
        continue_from: Some(PREV.into()),
        design,
    };
    tokio::time::timeout(ANSWER, rig.s.request(req))
        .await
        .expect("answered")
}

fn started_in(reply: &RunReply) -> (String, RunState) {
    match reply {
        RunReply::Started { run_id, state, .. } => (run_id.clone(), *state),
        other => panic!("not started: {other:?}"),
    }
}

/// Decision 30: with no `design`, a continued goal uses the flow (the configured
/// default), and the reply says it starts brainstorming, not planning.
#[tokio::test(flavor = "multi_thread")]
async fn a_continued_goal_uses_the_design_flow_and_answers_its_real_state() {
    let rig = full_by_default().await;
    let reply = continued(&rig, &rig.checkout.work.clone(), None).await;
    let (run_id, state) = started_in(&reply);
    assert_eq!(state, RunState::Brainstorming);
    let run = crate::lock(&rig.s.state).runs[&run_id].clone();
    assert_eq!(run.design_mode, DesignMode::Full);
    assert!(run.orch.design.is_some());
    rig.stop().await;
}

/// The request's own `design` reaches the build, over the configured default: `full`
/// brainstorms, and `off` plans as 9.3 did.
#[tokio::test(flavor = "multi_thread")]
async fn a_continued_goals_design_request_is_kept() {
    for (default, asked, state) in [
        (DesignMode::Off, DesignMode::Full, RunState::Brainstorming),
        (DesignMode::Full, DesignMode::Off, RunState::Planning),
    ] {
        let rig = rig_with(default).await;
        let reply = continued(&rig, &rig.checkout.work.clone(), Some(asked)).await;
        let (run_id, started) = started_in(&reply);
        assert_eq!(started, state);
        let run = crate::lock(&rig.s.state).runs[&run_id].clone();
        assert_eq!(run.design_mode, asked);
        assert_eq!(run.orch.design.is_some(), asked == DesignMode::Full);
        rig.stop().await;
    }
}

/// Decision 30 and DF §8.2: the orchestrator's `start_goal` (its inherited options)
/// starts a `full` goal by default, and never with `--yes`.
#[tokio::test(flavor = "multi_thread")]
async fn start_goal_defaults_to_the_design_flow() {
    let rig = full_by_default().await;
    let prev = crate::lock(&rig.s.state).runs[PREV].clone();
    let mut next = Next::inherited(&prev, "Add a logout button".into());
    assert_eq!(next.design, None);
    next.dir = rig.checkout.work.clone();
    let run = rig.s.continued_run(PREV, next).await.expect("built");
    assert_eq!(run.design_mode, DesignMode::Full);
    assert_eq!(run.state, RunState::Brainstorming);
    assert!(!run.orch.yes, "it stops at every gate");
    rig.stop().await;
}
