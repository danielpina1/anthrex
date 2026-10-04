//! Milestone 9.3's final fix wave (W1), on `chain_goal_tests.rs`' rig (a real socket,
//! a temporary checkout, the fake GitHub, and a real PTY window whose `claude` is a
//! stand-in that sleeps): an idle chain's window that exited before its chain was seen
//! ended is not adopted; the adoption is lost (review B, M1). A delivered run iterated after its window was closed starts a
//! fresh session with its handoff (review A, M3); and an adopted window still reaches
//! its chain after an evicted chain iterates (task 6b fix round 4, on a real window);
//! `start_goal` from an unchained orchestrator is refused in its words (review B, M5);
//! a continued goal over the cap is refused (review B, M6).
//! No agent runs; the tests kill only the windows the rig and its runs made.

use std::time::{Duration, Instant};

use proto::{AgentRole, RunReply, RunRequest, Status};
use serde_json::{Value, json};

use super::super::chain_goal::tests::{ANSWER, CHAIN, ChainRig, PREV, started};
use crate::run::model::Run;
use crate::window::WindowEvent;

/// [`PREV`]'s summary, which a fresh session's first prompt names.
const SUMMARY: &str = "added login";

/// Waits, within [`ANSWER`], until `done` holds of run `run_id`.
async fn until(rig: &ChainRig, run_id: &str, what: &str, done: impl Fn(&Run) -> bool) {
    let deadline = Instant::now() + ANSWER;
    loop {
        if crate::lock(&rig.s.state)
            .runs
            .get(run_id)
            .is_some_and(&done)
        {
            return;
        }
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// B-M1: the chain's window exits after the continue's lookup chose to adopt it, and
/// before the adoption runs (the window is still listed, `Exited`). The adoption is
/// lost: the new run keeps no window, and its first prompt is the handoff of a fresh
/// session, never the exited window's restart with the previous run's prompt.
#[tokio::test(flavor = "multi_thread")]
async fn an_exited_window_is_not_adopted() {
    let rig = ChainRig::new(|prev| {
        prev.orch.orchestrator.as_mut().unwrap().summary = Some(SUMMARY.into());
    })
    .await;
    let run_id = started(&rig.continued(PREV, &rig.checkout.work.clone()).await);
    let _ = rig.manager.kill(rig.window);
    // The rig runs no window event pump: its exit is delivered as the daemon's pump
    // delivers it.
    let exit = WindowEvent::Exited {
        code: None,
        signal: Some("SIGHUP".into()),
    };
    rig.manager.handle_event(rig.window, exit);
    let list = rig.manager.list();
    let exited = list.iter().find(|w| w.id == rig.window).map(|w| w.status);
    assert_eq!(exited, Some(Status::Exited));
    let name = format!("{}/orchestrator", &run_id[run_id.len() - 4..]);
    rig.s.adopt_window(&run_id, rig.window, name, || {}).await;
    until(
        &rig,
        &run_id,
        "the adoption was never reported lost",
        |run| {
            let o = run.orch.orchestrator.as_ref().unwrap();
            o.window_id.is_none()
                && o.first_prompt
                    .contains("This session continues o-3f9a. Your previous run 3f9a was accepted.")
        },
    )
    .await;
    rig.stop().await;
}

/// A-M3: [`PREV`] delivered (a `pr` run complete, D17), its idle window closed by the
/// user (I3), then iterated. Its restart finds no window, so its orchestrator launches
/// a fresh session whose first prompt is the run's own, its summary so far and its
/// chain's history; round 2's request waits for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_delivered_run_whose_window_was_closed_iterates_in_a_fresh_session() {
    let rig = ChainRig::new(|prev| {
        prev.state = proto::RunState::Complete;
        prev.delivery.mode = proto::DeliveryMode::Pr;
        prev.orch.orchestrator.as_mut().unwrap().summary = Some(SUMMARY.into());
    })
    .await;
    rig.manager
        .remove(rig.window)
        .expect("the rig's own window");
    let req = RunRequest::Iterate {
        run: PREV.into(),
        goal: "also add a logout button".into(),
        design: None,
    };
    let reply = tokio::time::timeout(ANSWER, rig.s.request(req))
        .await
        .expect("answered");
    assert!(matches!(reply, RunReply::Done { .. }), "{reply:?}");
    let fresh = |run: &Run| {
        let o = run.orch.orchestrator.as_ref().unwrap();
        o.window_id != Some(rig.window)
            && (o.launch_op.is_some() || o.window_id.is_some() || o.start_error.is_some())
            && o.first_prompt.contains(
                "This session continues o-3f9a on run 3f9a, whose earlier session is gone.",
            )
            && o.first_prompt.contains(SUMMARY)
    };
    until(&rig, PREV, "no fresh session was launched", fresh).await;
    // The fresh session's window, once made, is this test's to kill.
    let deadline = Instant::now() + ANSWER;
    let made = loop {
        let run = crate::lock(&rig.s.state).runs[PREV].clone();
        let o = run.orch.orchestrator.clone().unwrap();
        if o.window_id.is_some() || o.start_error.is_some() || Instant::now() >= deadline {
            break o.window_id;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    if let Some(window) = made {
        let _ = rig.manager.kill(window);
    }
    assert!(
        crate::lock(&rig.s.state).runs[PREV]
            .orch
            .request_wake
            .is_some(),
        "round 2's request waits for the fresh session"
    );
    assert!(made.is_some(), "the fresh session's window");
    rig.stop().await;
}

/// Task 6b fix round 4 (re-review I2), on a real window since the final fix wave's
/// A-M3 (its old rig named a window that did not exist, which an iterate now replaces
/// with a fresh session): an adopted window's MCP target names its chain's first run.
/// Once the chain's second run is delivered, its chain evicted from the table (the
/// project's newer idle chain) and the run iterated, the chain comes back with every
/// run that carries it, so a call naming the first run reaches the second, and
/// `get_context` and `edit_plan` work.
#[tokio::test(flavor = "multi_thread")]
async fn an_adopted_window_reaches_its_chain_after_an_evicted_chain_iterates() {
    let rig = ChainRig::new(|_| {}).await;
    let next = started(&rig.continued(PREV, &rig.checkout.work.clone()).await);
    let deadline = Instant::now() + ANSWER;
    while rig.manager.run_window_live(rig.window).map(|r| r.run_id) != Some(next.clone()) {
        assert!(Instant::now() < deadline, "the window was never adopted");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    {
        let mut state = crate::lock(&rig.s.state);
        let run = state.runs.get_mut(&next).unwrap();
        run.state = proto::RunState::Complete;
        run.delivery.mode = proto::DeliveryMode::Pr;
        run.cancelled = false;
        // Evicted: the project's newer idle chain took its place.
        state.chains.remove(CHAIN);
    }
    let req = RunRequest::Iterate {
        run: next.clone(),
        goal: "add a logout button".into(),
        design: None,
    };
    let reply = tokio::time::timeout(ANSWER, rig.s.request(req))
        .await
        .expect("answered");
    assert!(matches!(reply, RunReply::Done { .. }), "{reply:?}");
    let chain = crate::lock(&rig.s.state).chains[CHAIN].clone();
    assert_eq!(chain.runs, [PREV, next.as_str()]);
    assert_eq!(chain.current(), next);
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
    let call = |tool: &'static str, args: Value| {
        let opts = opts.clone();
        async move {
            let (ok, text) = tokio::time::timeout(ANSWER, mcp::forward(&opts, tool, args))
                .await
                .expect("answered");
            (
                ok,
                serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text)),
            )
        }
    };
    let (ok, digest) = call("run_status", json!({})).await;
    assert!(ok, "{digest}");
    assert_eq!(digest["run"]["id"], json!(next), "{digest}");
    let (ok, context) = call("get_context", json!({})).await;
    assert!(ok, "{context}");
    let (ok, answer) = call("edit_plan", json!({})).await;
    assert!(ok, "{answer}");
    rig.stop().await;
}

/// B-M5: an orchestrator whose run has no chain (one restored from before 9.3, A-M4,
/// accepted) calls `start_goal`: the refusal is worded for the orchestrator, naming
/// no CLI flag.
#[tokio::test(flavor = "multi_thread")]
async fn start_goal_from_an_unchained_orchestrator_is_refused_in_its_words() {
    let rig = ChainRig::new(|prev| prev.chain = None).await;
    assert!(crate::lock(&rig.s.state).chains.is_empty());
    let opts = mcp::McpOptions {
        role: AgentRole::Orchestrator,
        run_id: PREV.into(),
        task_id: None,
        scout_id: None,
        epic: None,
        window_id: rig.window,
        socket: rig.socket.clone(),
        chain: None,
        lane: None,
    };
    let (ok, text) = tokio::time::timeout(
        ANSWER,
        mcp::forward(&opts, "start_goal", json!({"goal": "Add a logout button"})),
    )
    .await
    .expect("answered");
    assert!(!ok, "{text}");
    let refusal = "this orchestrator has no chain to continue; the user starts the next goal \
                   with a new orchestrator";
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(answer, json!({ "error": refusal }));
    assert!(rig.new_runs().is_empty());
    rig.stop().await;
}

/// B-M6: a continued goal over `GOAL_MAX_CHARS` is refused with the daemon's text, as
/// an iterate's request is, and starts nothing; one at the cap is not refused for its
/// length.
#[tokio::test(flavor = "multi_thread")]
async fn a_continued_goal_over_the_cap_is_refused() {
    let rig = ChainRig::new(|_| {}).await;
    let request = |goal: String| RunRequest::StartGoal {
        goal,
        dir: rig.checkout.work.clone(),
        yes: false,
        trust_project: false,
        unconfined_checks: true,
        orchestrator: None,
        delivery: Some(proto::DeliveryMode::Local),
        continue_from: Some(PREV.into()),
        design: None,
    };
    let over = "é".repeat(proto::GOAL_MAX_CHARS + 1);
    let reply = tokio::time::timeout(ANSWER, rig.s.request(request(over)))
        .await
        .expect("answered");
    let text = "the goal is longer than its 16,384-character limit";
    assert_eq!(
        reply,
        RunReply::refused(proto::run_wire::request::START_GOAL, text.to_string())
    );
    assert!(rig.new_runs().is_empty());
    let at = "é".repeat(proto::GOAL_MAX_CHARS);
    let reply = tokio::time::timeout(ANSWER, rig.s.request(request(at)))
        .await
        .expect("answered");
    assert!(matches!(reply, RunReply::Started { .. }), "{reply:?}");
    rig.stop().await;
}
