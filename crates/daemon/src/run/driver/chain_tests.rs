//! Milestone 9.3 task 6a: which run a chained orchestrator's call reaches (decision
//! 20), the idle orchestrator's tool limits (decision 21) and the idle window's check
//! (decision 19), through a real daemon socket with no agent (`orch_read_rig.rs`).

use std::time::{Duration, Instant};

use proto::{AgentRole, RunReply, RunRequest, RunState};
use serde_json::{Value, json};

use super::read_rig::{ANSWER, ORCH, Rig};
use crate::run::chain::rebuild;
use crate::run::driver::*;

/// The rig's run's chain (its id ends in `3f9a`).
const CHAIN: &str = "o-3f9a";
/// The chain's next run.
const NEXT: &str = "next-goal-4c1d";

/// The rig's run, `first`, accepted, and [`NEXT`], a copy of it running as its
/// chain's current run, with the same orchestrator window.
fn two_runs(rig: &Rig) {
    let mut state = crate::lock(&rig.runs.state);
    let first = state.runs.get_mut(&rig.run_id).unwrap();
    first.chain = Some(CHAIN.into());
    let mut next = first.clone();
    first.state = RunState::Accepted;
    next.id = NEXT.into();
    next.created_at += 1;
    next.orch.digest_fp = crate::run::orch::digest::fingerprint(&next);
    state.runs.insert(NEXT.into(), next);
    state.chains = rebuild(&state.runs);
    assert_eq!(state.chains[CHAIN].runs, [rig.run_id.as_str(), NEXT]);
}

/// The rig's run, accepted, as its chain's last run: the chain is idle, its window
/// still open.
fn idle(rig: &Rig) {
    let mut state = crate::lock(&rig.runs.state);
    let run = state.runs.get_mut(&rig.run_id).unwrap();
    run.chain = Some(CHAIN.into());
    run.state = RunState::Accepted;
    state.chains = rebuild(&state.runs);
    state.chains.get_mut(CHAIN).unwrap().ended = false;
}

/// The orchestrator's `tool` from a window of `chain`, naming `run_id`.
async fn chained(
    rig: &Rig,
    (run_id, chain): (&str, &str),
    tool: &str,
    args: Value,
) -> (bool, Value) {
    let mut opts = rig.opts(AgentRole::Orchestrator, ORCH, None);
    opts.run_id = run_id.into();
    opts.chain = Some(chain.into());
    rig.call(opts, tool, args).await
}

/// `run edit <run> cancel t1`, as `anthrex run edit` sends it.
async fn cancel_t1(rig: &Rig, run_id: &str) {
    let edit = serde_json::from_value(json!({"op": "cancel_task", "task_id": "t1"})).unwrap();
    let reply = rig
        .request(RunRequest::Edit {
            run_id: run_id.into(),
            edits: vec![edit],
            submit: false,
        })
        .await;
    assert!(matches!(reply, RunReply::Done { .. }), "{reply:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chained_call_reaches_the_current_run() {
    let rig = Rig::new(|_, _| {}).await;
    two_runs(&rig);
    let first = rig.run_id.clone();
    let (ok, digest) = chained(&rig, (&first, CHAIN), "run_status", json!({})).await;
    assert!(ok, "{digest}");
    assert_eq!(digest["run"]["id"], json!(NEXT), "{digest}");
    assert_eq!(digest["run"]["state"], json!("running"), "{digest}");
    // The current run itself is reached as it is.
    let (ok, digest) = chained(&rig, (NEXT, CHAIN), "run_status", json!({})).await;
    assert!(ok && digest["run"]["id"] == json!(NEXT), "{digest}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_outside_the_chain_is_refused() {
    let rig = Rig::new(|_, _| {}).await;
    {
        // `NEXT` is the only run of its own chain; the rig's run is in none.
        let mut state = crate::lock(&rig.runs.state);
        let mut next = state.runs[&rig.run_id].clone();
        next.id = NEXT.into();
        next.chain = Some("o-4c1d".into());
        state.runs.insert(NEXT.into(), next);
        state.chains = rebuild(&state.runs);
    }
    let first = rig.run_id.clone();
    let text =
        format!("this window is the orchestrator of o-4c1d; run {first} is not one of its runs");
    for (tool, args) in [
        ("run_status", json!({})),
        ("get_context", json!({})),
        ("edit_plan", json!({"edits": [], "submit": true})),
    ] {
        let (ok, answer) = chained(&rig, (&first, "o-4c1d"), tool, args).await;
        assert!(!ok, "{tool}: {answer}");
        assert_eq!(answer, json!({ "error": text }), "{tool}");
    }
    // D16: a chain not in the table, named with a run that does not carry it.
    let (ok, answer) = chained(&rig, (&first, "o-0000"), "run_status", json!({})).await;
    let text =
        format!("this window is the orchestrator of o-0000; run {first} is not one of its runs");
    assert!(!ok);
    assert_eq!(answer, json!({ "error": text }));
    // Nothing reached the rig's run.
    assert!(rig.run(|run| run.orch.digest_read_at.is_none()));
}

/// D16 (decision 20 amended): a failed run's chain leaves the table, and its window,
/// released as today (KG §3.1), still reads its own run; a write is the engine's to
/// refuse, as before 9.3.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_runs_window_reads_its_own_run() {
    let rig = Rig::new(|run, _| run.chain = Some(CHAIN.into())).await;
    {
        let mut state = crate::lock(&rig.runs.state);
        state.chains = rebuild(&state.runs);
        state.runs.get_mut(&rig.run_id).unwrap().state = RunState::Failed;
    }
    rig.runs.send(EventKind::Tick);
    let deadline = Instant::now() + ANSWER;
    while !crate::lock(&rig.runs.state).chains.is_empty() {
        assert!(Instant::now() < deadline, "the failed run's chain stayed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let first = rig.run_id.clone();
    let (ok, digest) = chained(&rig, (&first, CHAIN), "run_status", json!({})).await;
    assert!(ok, "{digest}");
    assert_eq!(digest["run"]["id"], json!(first), "{digest}");
    assert_eq!(digest["run"]["state"], json!("failed"), "{digest}");
    let args = json!({"edits": [], "submit": true});
    let (ok, answer) = chained(&rig, (&first, CHAIN), "edit_plan", args).await;
    assert!(!ok, "{answer}");
    assert!(
        !answer.to_string().contains("not one of its runs"),
        "{answer}"
    );
}

/// D16: the project's older idle chain, dropped when a newer one went idle (decision
/// 19), keeps a plain window that still reads its own run.
#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_idle_chains_window_reads_its_own_run() {
    use crate::run::chain::{Chain, make_idle};
    let rig = Rig::new(|run, _| {
        run.chain = Some(CHAIN.into());
        run.state = RunState::Accepted;
    })
    .await;
    {
        let mut state = crate::lock(&rig.runs.state);
        state.chains = rebuild(&state.runs);
        let mut newer = state.runs[&rig.run_id].clone();
        newer.id = "newer-goal-4c1d".into();
        newer.chain = Some("o-4c1d".into());
        newer.state = RunState::Discarded;
        let chain = Chain::new("o-4c1d".into(), &newer).unwrap();
        state.runs.insert(newer.id.clone(), newer);
        state.chains.insert(chain.id.clone(), chain);
        make_idle(&mut state.chains, "o-4c1d");
        assert!(
            !state.chains.contains_key(CHAIN),
            "the older idle chain is dropped"
        );
    }
    let first = rig.run_id.clone();
    let (ok, digest) = chained(&rig, (&first, CHAIN), "run_status", json!({})).await;
    assert!(ok, "{digest}");
    assert_eq!(digest["run"]["id"], json!(first), "{digest}");
    let (ok, context) = chained(&rig, (&first, CHAIN), "get_context", json!({})).await;
    assert!(ok, "{context}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_orchestrator_may_read_but_not_edit() {
    let rig = Rig::new(|_, _| {}).await;
    idle(&rig);
    let first = rig.run_id.clone();
    let (ok, context) = chained(&rig, (&first, CHAIN), "get_context", json!({})).await;
    assert!(ok, "{context}");
    let (ok, digest) = chained(&rig, (&first, CHAIN), "run_status", json!({})).await;
    assert!(
        ok && digest["run"]["state"] == json!("accepted"),
        "{digest}"
    );
    let text = "run 3f9a has ended; start a new goal with start_goal when the user gives you one";
    for (tool, args) in [
        ("edit_plan", json!({"edits": [], "submit": true})),
        ("spawn_scout", json!({"question": "where is auth?"})),
        ("task_result", json!({"task_id": "t0"})),
    ] {
        let (ok, answer) = chained(&rig, (&first, CHAIN), tool, args).await;
        assert!(!ok, "{tool}: {answer}");
        assert_eq!(answer, json!({ "error": text }), "{tool}");
    }
}

/// The long-poll test's shape (M9.11): no engine lock is held while a resolved
/// `run_status` waits, so `run status` answers within its separation bound (2 s
/// against the call's 10 s wait); the wait is on the current run, which an edit of
/// that run ends. Fix round 1, m5: the call is known to wait once it has subscribed to
/// the snapshot pushes (`wait_digest`), which the rig's receiver count shows; nothing
/// else in the rig subscribes.
#[tokio::test(flavor = "multi_thread")]
async fn resolution_holds_no_lock_across_the_read() {
    let rig = Arc::new(Rig::new(|_, _| {}).await);
    two_runs(&rig);
    let rev = crate::lock(&rig.runs.state).runs[NEXT].orch.digest_rev;
    let receivers = rig.runs.pushes.receiver_count();
    let waiting = rig.clone();
    let first = rig.run_id.clone();
    let wait = tokio::spawn(async move {
        let args = json!({"since": rev, "wait_secs": 10});
        chained(&waiting, (&first, CHAIN), "run_status", args).await
    });
    let deadline = Instant::now() + ANSWER;
    while rig.runs.pushes.receiver_count() <= receivers {
        assert!(!wait.is_finished(), "the resolved call did not wait");
        assert!(Instant::now() < deadline, "the resolved call never waited");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !wait.is_finished(),
        "the resolved call waits on the running run"
    );
    let started = Instant::now();
    let reply = rig.request(RunRequest::List).await;
    assert!(matches!(reply, RunReply::Snapshot(_)), "{reply:?}");
    assert!(started.elapsed() < Duration::from_secs(2));
    cancel_t1(&rig, NEXT).await;
    let (ok, digest) = tokio::time::timeout(ANSWER, wait).await.unwrap().unwrap();
    assert!(ok, "{digest}");
    assert_eq!(digest["run"]["id"], json!(NEXT));
    assert!(digest["revision"].as_u64().unwrap() > rev, "{digest}");
}

/// Decision 19: an idle chain whose window is gone from the manager's list ends at the
/// next check.
#[tokio::test(flavor = "multi_thread")]
async fn an_idle_chains_closed_window_ends_it() {
    let rig = Rig::new(|_, _| {}).await;
    idle(&rig);
    // Window `ORCH` does not exist.
    rig.runs.check_orchestrators();
    let deadline = Instant::now() + ANSWER;
    while !crate::lock(&rig.runs.state).chains[CHAIN].ended {
        assert!(Instant::now() < deadline, "the chain never ended");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Task 6b (6a review m4): a `run_status` long-poll resolved to the idle chain's last
/// run just before a next goal adopted the window answers for the new run. Here the
/// adopt lands between the resolution and the read (the call reaches `read` already
/// resolved): the answer is the chain's current run's, at once.
#[tokio::test(flavor = "multi_thread")]
async fn a_long_poll_resolved_before_an_adopt_answers_for_the_new_run() {
    let rig = Rig::new(|_, _| {}).await;
    idle(&rig);
    let first = rig.run_id.clone();
    let rev = rig.digest_rev();
    let call = proto::ToolCall {
        run_id: first.clone(),
        task_id: None,
        role: AgentRole::Orchestrator,
        window_id: ORCH,
        tool: "run_status".into(),
        args: json!({"since": rev, "wait_secs": 10}),
        scout_id: None,
        epic: None,
        chain: Some(CHAIN.into()),
    };
    // Resolved while idle: the chain's last run.
    let resolved = rig.runs.resolved(call).unwrap();
    assert_eq!(resolved.run_id, first);
    // The next goal adopts the window.
    {
        let mut state = crate::lock(&rig.runs.state);
        let mut next = state.runs[&first].clone();
        next.id = NEXT.into();
        next.state = RunState::Planning;
        next.created_at += 1;
        next.orch.digest_fp = crate::run::orch::digest::fingerprint(&next);
        state.runs.insert(NEXT.into(), next);
        let chain = state.chains.get_mut(CHAIN).unwrap();
        chain.runs.push(NEXT.into());
        chain.state = crate::run::chain::ChainState::Active;
    }
    let started = Instant::now();
    let reply = tokio::time::timeout(ANSWER, rig.runs.read(resolved, ANSWER))
        .await
        .expect("the read answers");
    let RunReply::ToolResult { ok, text, .. } = reply else {
        panic!("{reply:?}");
    };
    let digest: Value = serde_json::from_str(&text).unwrap();
    assert!(ok, "{digest}");
    assert_eq!(digest["run"]["id"], json!(NEXT), "{digest}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

/// Task 6b (6a re-review, D16): a window adopted by a later run keeps an MCP target
/// naming the chain's first run. Once that chain has left the table (here its newest
/// run failed), the window reads the newest run carrying the chain, not the first; its
/// `start_goal` is refused, the window being a plain one (decision 19).
#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_chains_window_reads_its_newest_run_and_starts_no_goal() {
    let rig = Rig::new(|_, _| {}).await;
    two_runs(&rig);
    {
        let mut state = crate::lock(&rig.runs.state);
        state.runs.get_mut(NEXT).unwrap().state = RunState::Failed;
        state.chains.clear();
    }
    let first = rig.run_id.clone();
    let (ok, digest) = chained(&rig, (&first, CHAIN), "run_status", json!({})).await;
    assert!(ok, "{digest}");
    assert_eq!(digest["run"]["id"], json!(NEXT), "{digest}");
    let (ok, answer) = chained(&rig, (&first, CHAIN), "start_goal", json!({"goal": "more"})).await;
    assert!(!ok, "{answer}");
    let text = "o-3f9a has ended; this window cannot start a goal, and the user starts the next one with a new orchestrator";
    assert_eq!(answer, json!({ "error": text }));
    assert_eq!(crate::lock(&rig.runs.state).runs.len(), 2, "no run started");
}

/// The rig's run, delivered (a `pr` run complete with every PR landed, D17), as its
/// chain's last run: the chain is idle, its window still open.
fn delivered(rig: &Rig) {
    let mut state = crate::lock(&rig.runs.state);
    let run = state.runs.get_mut(&rig.run_id).unwrap();
    run.chain = Some(CHAIN.into());
    run.state = RunState::Complete;
    run.delivery.mode = proto::DeliveryMode::Pr;
    run.cancelled = false;
    if run.rounds.is_empty() {
        let first = crate::run::model::Round::first(run);
        run.rounds.push(first);
    }
    state.chains = rebuild(&state.runs);
    let chain = state.chains.get_mut(CHAIN).unwrap();
    assert_eq!(chain.state, crate::run::chain::ChainState::Idle);
    chain.ended = false;
}

/// Task 6b fix round 2 (ruling "6b concerns", concern 1): a delivered run's idle
/// orchestrator may call `edit_plan` with `summary` alone (decision 38) or `iterate`
/// alone (D17); any other `edit_plan`, and every other tool, is refused with decision
/// 21's text.
#[tokio::test(flavor = "multi_thread")]
async fn a_delivered_runs_idle_orchestrator_may_only_summarise_or_iterate() {
    let rig = Rig::new(|_, _| {}).await;
    delivered(&rig);
    let first = rig.run_id.clone();
    let text = "run 3f9a has ended; start a new goal with start_goal when the user gives you one";
    for args in [
        json!({}),
        json!({"submit": true}),
        json!({"edits": [{"op": "cancel_task", "task_id": "t1"}]}),
        json!({"edits": [], "submit": true}),
        json!({"summary": "done", "edits": [{"op": "cancel_task", "task_id": "t1"}]}),
        json!({"edits": [], "summary": "done", "iterate": "more"}),
    ] {
        let (ok, answer) = chained(&rig, (&first, CHAIN), "edit_plan", args.clone()).await;
        assert!(!ok, "{args}: {answer}");
        assert_eq!(answer, json!({ "error": text }), "{args}");
    }
    let (ok, answer) = chained(
        &rig,
        (&first, CHAIN),
        "task_result",
        json!({"task_id": "t0"}),
    )
    .await;
    assert_eq!((ok, answer), (false, json!({ "error": text })));

    // Task M9.3.7 fix round 1: the summary alone needs no `edits`.
    let summary = json!({"summary": "added login"});
    let (ok, answer) = chained(&rig, (&first, CHAIN), "edit_plan", summary).await;
    assert!(ok, "{answer}");
    {
        let state = crate::lock(&rig.runs.state);
        let run = &state.runs[&first];
        assert_eq!(
            run.rounds.last().and_then(|r| r.summary.as_deref()),
            Some("added login")
        );
        assert_eq!(
            state.chains[CHAIN].state,
            crate::run::chain::ChainState::Idle
        );
    }
    let iterate = json!({"iterate": "add a logout button"});
    let (ok, answer) = chained(&rig, (&first, CHAIN), "edit_plan", iterate).await;
    assert!(ok, "{answer}");
    let state = crate::lock(&rig.runs.state);
    assert_eq!(state.runs[&first].state, RunState::Planning);
    assert_eq!(state.runs[&first].round(), 2);
    assert_eq!(
        state.chains[CHAIN].state,
        crate::run::chain::ChainState::Active
    );
}
