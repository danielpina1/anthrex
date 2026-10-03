//! Milestone 9.3's final fix wave (review B, I1): a continued start has a deadline on
//! everything before the engine's `Start`. On `chain_goal_tests.rs`' rig (a real
//! socket, a temporary checkout, the fake GitHub, a stand-in `claude` that sleeps),
//! with the context's git a stand-in that never answers within the deadline: the
//! `start_goal` tool and the request are refused with the exact text, and no run, ref
//! or wake was made. The deadline is the test seam's cap (`RunContext::continue_cap`),
//! 1 s, under the stand-in's own `git_timeout_secs` (5 s).

use std::path::Path;
use std::time::{Duration, Instant};

use proto::run_wire::request;
use proto::{AgentRole, DeliveryMode, RunReply, RunRequest};
use serde_json::json;

use super::tests::{ANSWER, CHAIN, ChainRig, PREV};
use crate::run::chain::{CONTINUE_START_BOUND, ChainState, START_GOAL_TOOL_BOUND};
use crate::run::driver::delivery::tests::git;
use crate::run::orch::contract_rounds::CONTINUE_TOO_SLOW;

/// The test seam's deadline.
const CAP: Duration = Duration::from_secs(1);

/// A `git` that answers nothing for 8 s (past the rig's 5 s `git_timeout_secs`, which
/// kills it first), in `dir`.
fn stuck_git(dir: &Path) -> std::ffi::OsString {
    let git = dir.join("stuck-git");
    std::fs::write(&git, "#!/bin/sh\nexec sleep 8\n").unwrap();
    std::fs::set_permissions(&git, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    git.into_os_string()
}

/// A rig whose daemon's git is stuck, the continue deadline capped at [`CAP`].
async fn stuck_rig() -> ChainRig {
    ChainRig::with_context(
        |_| {},
        |ctx| {
            // The checkout's temporary directory, which the rig removes.
            let tmp = ctx.data_dir.parent().unwrap().to_path_buf();
            ctx.git = stuck_git(&tmp);
            ctx.continue_cap = Some(CAP);
        },
    )
    .await
}

/// Every ref under `refs/anthrex` of the checkout, as the real git lists them.
fn anthrex_refs(work: &Path) -> String {
    git(work, &["for-each-ref", "refs/anthrex"])
}

/// No run was made, no ref, and the chain and its window are as they were: idle, on
/// `PREV`, nothing to paste.
fn nothing_started(rig: &ChainRig, refs: &str) {
    assert!(rig.new_runs().is_empty());
    assert_eq!(anthrex_refs(&rig.checkout.work), refs);
    let state = crate::lock(&rig.s.state);
    let chain = &state.chains[CHAIN];
    assert_eq!(
        (chain.state, chain.runs.clone()),
        (ChainState::Idle, vec![PREV.to_string()])
    );
    assert_eq!(state.runs[PREV].orch.request_wake, None);
    drop(state);
    assert_eq!(rig.manager.run_window_live(rig.window), None, "not adopted");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stuck_continue_past_the_tool_deadline_starts_nothing() {
    let rig = stuck_rig().await;
    let refs = anthrex_refs(&rig.checkout.work);
    let opts = mcp::McpOptions {
        role: AgentRole::Orchestrator,
        run_id: PREV.into(),
        task_id: None,
        scout_id: None,
        epic: None,
        window_id: rig.window,
        socket: rig.socket.clone(),
        chain: Some(CHAIN.into()),
        lane: None,
    };
    let began = Instant::now();
    let (ok, text) = tokio::time::timeout(
        ANSWER,
        mcp::forward(&opts, "start_goal", json!({"goal": "Add a logout button"})),
    )
    .await
    .expect("answered");
    let took = began.elapsed();
    assert!(!ok, "{text}");
    assert!(text.contains(CONTINUE_TOO_SLOW), "{text}");
    assert!(took < CAP + Duration::from_secs(5), "{took:?}");
    nothing_started(&rig, &refs);

    // The request path (the CLI's `--continue`, the TUI) is refused the same way.
    let req = RunRequest::StartGoal {
        goal: "Add a logout button".into(),
        dir: rig.checkout.work.clone(),
        yes: false,
        trust_project: false,
        unconfined_checks: true,
        orchestrator: None,
        delivery: Some(DeliveryMode::Local),
        continue_from: Some(PREV.into()),
    };
    let reply = tokio::time::timeout(ANSWER, rig.s.request(req))
        .await
        .expect("answered");
    assert_eq!(
        reply,
        RunReply::refused(request::START_GOAL, CONTINUE_TOO_SLOW.to_string())
    );
    nothing_started(&rig, &refs);
    rig.stop().await;
}

/// The two deadlines: the request's has `run start`'s terms; the tool's leaves
/// `anthrex mcp`'s 100 s reply bound 10 s for the step and the round trip.
#[test]
fn the_continue_deadlines_are_run_starts_and_the_tools() {
    assert_eq!(
        CONTINUE_START_BOUND,
        Duration::from_secs(180) + crate::host::PREFLIGHT_BOUND + Duration::from_secs(30)
    );
    assert_eq!(
        START_GOAL_TOOL_BOUND + Duration::from_secs(10),
        mcp::TOOL_REPLY_TIMEOUT
    );
}

/// W1 fix round 2 (item 5): a refusal the CLI words (`no_chain_to_continue`, from the
/// steps after the tool's own lookup) reaches the orchestrator in the tool's words;
/// every other refusal is as it was.
#[test]
fn the_tools_lost_chain_refusal_is_in_its_words() {
    use crate::run::orch::contract_rounds::{NO_CHAIN_FOR_TOOL, no_chain_to_continue};
    let lost = no_chain_to_continue("3f9a");
    assert_eq!(super::tool_refusal(lost, "3f9a"), NO_CHAIN_FOR_TOOL);
    let other = CONTINUE_TOO_SLOW.to_string();
    assert_eq!(super::tool_refusal(other.clone(), "3f9a"), other);
}
