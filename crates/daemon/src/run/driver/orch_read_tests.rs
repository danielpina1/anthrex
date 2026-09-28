//! Task M9.11: `run_status`, decision 16's long-poll, through a real daemon socket
//! with no agent (`orch_read_rig.rs`).

use std::time::{Duration, Instant};

use serde_json::json;

use super::read_rig::{ANSWER, Rig, WORKER};
use crate::run::driver::*;
use crate::run::engine::AgentSignal;
use proto::RunReply;
use proto::RunRequest;

/// Without `since` the digest answers at once, in any run state, and the read
/// receipt drops the wake notes the answer held (decisions 16 and 39).
#[tokio::test(flavor = "multi_thread")]
async fn run_status_returns_at_once_without_since() {
    let rig = Rig::new(|run, _| {
        let record = run.orch.orchestrator.as_mut().unwrap();
        record.notes = vec!["[anthrex] t1 blocked (question): which endpoint?".into()];
        record.note_seqs = vec![1];
        record.last_note_seq = 1;
    })
    .await;
    let rev = rig.digest_rev();
    for args in [json!({}), json!({"wait_secs": 50}), json!({"since": rev})] {
        let started = Instant::now();
        let (ok, digest) = rig.orch("run_status", args.clone()).await;
        assert!(ok, "{args}: {digest}");
        assert!(started.elapsed() < Duration::from_secs(5), "{args}");
        assert_eq!(digest["revision"], json!(rev), "{digest}");
        assert_eq!(digest["run"]["id"], json!(rig.run_id));
        assert_eq!(digest["counts"]["blocked"], json!(1), "{digest}");
    }
    // A `since` that is not the current revision answers at once too.
    let started = Instant::now();
    let (ok, _) = rig
        .orch("run_status", json!({"since": rev + 7, "wait_secs": 50}))
        .await;
    assert!(ok && started.elapsed() < Duration::from_secs(5));

    let deadline = Instant::now() + ANSWER;
    while rig.run(|run| !run.orch.orchestrator.as_ref().unwrap().notes.is_empty()) {
        assert!(
            Instant::now() < deadline,
            "the read receipt never dropped the note"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // A terminal run still answers (reads answer in every state).
    crate::lock(&rig.runs.state)
        .runs
        .get_mut(&rig.run_id)
        .unwrap()
        .state = proto::RunState::Failed;
    let (ok, digest) = rig.orch("run_status", json!({})).await;
    assert!(ok, "{digest}");
    assert_eq!(digest["run"]["state"], "failed");
}

/// Decision 16: a wait ends within 5 s of a state change, with a higher revision.
#[tokio::test(flavor = "multi_thread")]
async fn run_status_waits_until_the_digest_changes() {
    let rig = Arc::new(Rig::new(|_, _| {}).await);
    let rev = rig.digest_rev();
    let waiting = rig.clone();
    let wait = tokio::spawn(async move {
        let (ok, digest) = waiting
            .orch("run_status", json!({"since": rev, "wait_secs": 20}))
            .await;
        (ok, digest, Instant::now())
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!wait.is_finished(), "the call waits while nothing changes");
    rig.cancel_t1().await;
    let changed = Instant::now();
    let (ok, digest, answered) = tokio::time::timeout(Duration::from_secs(25), wait)
        .await
        .expect("the wait ends")
        .unwrap();
    assert!(ok, "{digest}");
    assert!(
        answered.duration_since(changed) < Duration::from_secs(5),
        "answered {:?} after the change",
        answered.duration_since(changed)
    );
    assert!(digest["revision"].as_u64().unwrap() > rev, "{digest}");
    assert_eq!(digest["counts"]["cancelled"], json!(1), "{digest}");
}

/// Decision 16: with nothing changing, the wait takes `wait_secs` and answers the same
/// revision.
#[tokio::test(flavor = "multi_thread")]
async fn run_status_times_out_with_the_same_revision() {
    let rig = Rig::new(|_, _| {}).await;
    let rev = rig.digest_rev();
    let started = Instant::now();
    let (ok, digest) = rig
        .orch("run_status", json!({"since": rev, "wait_secs": 2}))
        .await;
    let took = started.elapsed();
    assert!(ok, "{digest}");
    assert!(
        took >= Duration::from_secs(2) && took < Duration::from_secs(4),
        "{took:?}"
    );
    assert_eq!(digest["revision"], json!(rev), "{digest}");
}

/// Decision 16: a worker's tool call moves a counter, not the digest, so a wait is not
/// ended by it.
#[tokio::test(flavor = "multi_thread")]
async fn a_counter_change_does_not_end_the_wait() {
    let rig = Arc::new(Rig::new(|_, _| {}).await);
    let rev = rig.digest_rev();
    let waiting = rig.clone();
    let started = Instant::now();
    let wait = tokio::spawn(async move {
        waiting
            .orch("run_status", json!({"since": rev, "wait_secs": 3}))
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    rig.runs.send(EventKind::Signal {
        window_id: WORKER,
        signal: AgentSignal::ToolUse {
            name: "Read".into(),
        },
    });
    let deadline = Instant::now() + ANSWER;
    while rig.run(|run| run.tasks[0].rounds[0].tool_calls) == 0 {
        assert!(Instant::now() < deadline, "the tool call was never counted");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (ok, digest) = tokio::time::timeout(ANSWER, wait).await.unwrap().unwrap();
    let took = started.elapsed();
    assert!(ok, "{digest}");
    assert!(took >= Duration::from_secs(3), "{took:?}");
    assert_eq!(digest["revision"], json!(rev), "{digest}");
    assert_eq!(digest["spend"]["tool_calls"], json!(1), "{digest}");
}

/// Decision 16: a run that becomes terminal while a wait is open ends the wait at once,
/// even when nothing moved its digest revision.
#[tokio::test(flavor = "multi_thread")]
async fn terminal_run_ends_the_wait() {
    let rig = Arc::new(Rig::new(|_, _| {}).await);
    let rev = rig.digest_rev();
    let waiting = rig.clone();
    let wait = tokio::spawn(async move {
        let answer = waiting
            .orch("run_status", json!({"since": rev, "wait_secs": 30}))
            .await;
        (answer, Instant::now())
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!wait.is_finished());
    // Past the reducer, so only the terminal state can end the wait.
    let snap = {
        let mut state = crate::lock(&rig.runs.state);
        state.runs.get_mut(&rig.run_id).unwrap().state = proto::RunState::Discarded;
        snapshot(&state, unix_now())
    };
    rig.runs.publish(snap);
    let ended = Instant::now();
    let ((ok, digest), answered) = tokio::time::timeout(ANSWER, wait).await.unwrap().unwrap();
    assert!(ok, "{digest}");
    assert!(answered.duration_since(ended) < Duration::from_secs(2));
    assert_eq!(digest["run"]["state"], "discarded", "{digest}");
    assert_eq!(digest["revision"], json!(rev));
}

/// A waiting `run_status` whose receiver lags behind the snapshot pushes subscribes
/// again and keeps waiting (AGENTS.md: after `Lagged` a receiver resumes from the
/// oldest message); it still ends on the next change. One thread, so the waiting task
/// cannot read while the pushes are sent: the lag is certain.
#[tokio::test(flavor = "current_thread")]
async fn a_lagged_wait_subscribes_again_and_keeps_waiting() {
    let rig = Arc::new(Rig::new(|_, _| {}).await);
    let rev = rig.digest_rev();
    let waiting = rig.clone();
    let wait = tokio::spawn(async move {
        waiting
            .orch("run_status", json!({"since": rev, "wait_secs": 20}))
            .await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!wait.is_finished());
    // Far more unchanged pushes than the channel holds, sent at once.
    let snap = Arc::new(snapshot(&crate::lock(&rig.runs.state), unix_now()));
    for _ in 0..1_000 {
        let _ = rig.runs.pushes.send(snap.clone());
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!wait.is_finished(), "a lag ended the wait");
    rig.cancel_t1().await;
    let (ok, digest) = tokio::time::timeout(ANSWER, wait).await.unwrap().unwrap();
    assert!(ok && digest["revision"].as_u64().unwrap() > rev, "{digest}");
}

/// Review finding 4: the wait's deadline is set once and spans every resubscription.
/// A sender thread keeps the waiting receiver lagging for the whole wait (and past
/// it); the call still answers at `wait_secs`, with the same revision.
#[tokio::test(flavor = "multi_thread")]
async fn the_deadline_spans_lagged_resubscriptions() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let rig = Arc::new(Rig::new(|_, _| {}).await);
    let rev = rig.digest_rev();
    let snap = Arc::new(snapshot(&crate::lock(&rig.runs.state), unix_now()));
    let stop = Arc::new(AtomicBool::new(false));
    let (flooding, sender, pushes) = (stop.clone(), snap.clone(), rig.runs.pushes.clone());
    let flood = std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(12);
        while !flooding.load(Ordering::Relaxed) && Instant::now() < until {
            for _ in 0..512 {
                let _ = pushes.send(sender.clone());
            }
            std::thread::yield_now();
        }
    });
    let started = Instant::now();
    let answer = tokio::time::timeout(
        Duration::from_secs(15),
        rig.orch("run_status", json!({"since": rev, "wait_secs": 2})),
    )
    .await;
    let took = started.elapsed();
    stop.store(true, Ordering::Relaxed);
    flood.join().unwrap();
    let (ok, digest) = answer.expect("the wait ends");
    assert!(ok, "{digest}");
    assert!(
        took >= Duration::from_secs(2) && took < Duration::from_secs(5),
        "{took:?}"
    );
    assert_eq!(digest["revision"], json!(rev));
}

/// No engine lock is held while `run_status` waits: `run status` (a `List`) answers.
#[tokio::test(flavor = "multi_thread")]
async fn long_poll_holds_no_engine_lock() {
    let rig = Arc::new(Rig::new(|_, _| {}).await);
    let rev = rig.digest_rev();
    let waiting = rig.clone();
    let wait = tokio::spawn(async move {
        waiting
            .orch("run_status", json!({"since": rev, "wait_secs": 10}))
            .await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!wait.is_finished());
    let started = Instant::now();
    let reply = rig.request(RunRequest::List).await;
    assert!(matches!(reply, RunReply::Snapshot(_)), "{reply:?}");
    assert!(started.elapsed() < Duration::from_secs(2));
    // And the engine takes events meanwhile: an edit is applied, which ends the wait.
    rig.cancel_t1().await;
    let (ok, digest) = tokio::time::timeout(ANSWER, wait).await.unwrap().unwrap();
    assert!(ok && digest["revision"].as_u64().unwrap() > rev, "{digest}");
}
