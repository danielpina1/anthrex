//! Milestone 9.3 task 6b, fix round 1: an adoption that cannot land (m1) and an adopt
//! that reads no history (m5), on `chain_goal_tests.rs`' rig: a real socket, a
//! temporary checkout, and a real PTY window whose `claude` is a stand-in that sleeps.
//! No agent runs; the tests remove only the window the rig made.

use std::time::{Duration, Instant};

use proto::{AgentRole, RunRef};

use super::super::chain_goal::HISTORY_READS;
use super::super::chain_goal::tests::{ANSWER, CHAIN, ChainRig, PREV, context, started};
use crate::run::driver::RunService;
use crate::run::engine::HISTORY_FILE;
use crate::server::GitWiring;

/// [`PREV`]'s summary, which a fresh session's handoff names.
const SUMMARY: &str = "added login";

/// Waits, within [`ANSWER`], until `done` holds of run `run_id`'s orchestrator record.
async fn until(
    s: &RunService,
    run_id: &str,
    what: &str,
    done: impl Fn(&crate::run::model::Run) -> bool,
) {
    let deadline = Instant::now() + ANSWER;
    loop {
        if crate::lock(&s.state).runs.get(run_id).is_some_and(&done) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The adoption lost: no window, not live, and the handoff of the new run as its first
/// prompt (the predecessor's outcome and summary, never its copied first prompt).
fn lost(run: &crate::run::model::Run) -> bool {
    let Some(o) = run.orch.orchestrator.as_ref() else {
        return false;
    };
    o.window_id.is_none()
        && !o.live
        && o.first_prompt
            .contains(&format!("You are the orchestrator of run {} ", run.id))
        && o.first_prompt
            .contains("This session continues o-3f9a. Your previous run 3f9a was accepted.")
        && o.first_prompt.contains(SUMMARY)
}

/// The rig, its previous run carrying [`SUMMARY`], and a goal continued from it,
/// adopted: the new run's id.
async fn adopted() -> (ChainRig, String) {
    let rig = ChainRig::new(|prev| {
        prev.orch.orchestrator.as_mut().unwrap().summary = Some(SUMMARY.into());
    })
    .await;
    let run_id = started(&rig.continued(PREV, &rig.checkout.work.clone()).await);
    let expected = RunRef {
        run_id: run_id.clone(),
        task_id: None,
        role: AgentRole::Orchestrator,
        session: 1,
        lane: None,
    };
    let deadline = Instant::now() + ANSWER;
    while rig.manager.run_window_live(rig.window) != Some(expected.clone()) {
        assert!(Instant::now() < deadline, "the window was never adopted");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (rig, run_id)
}

/// m1: the adopted window is gone when the driver rebinds it (`no window with id`),
/// so the driver sends `OrchEvent::AdoptLost` with the new run's handoff, and the run
/// keeps no window a `run resume` could only fail to restart.
#[tokio::test(flavor = "multi_thread")]
async fn an_adoption_whose_window_is_gone_leaves_a_fresh_launch() {
    let (rig, run_id) = adopted().await;
    rig.manager
        .remove(rig.window)
        .expect("the rig's own window");
    let name = format!("{}/orchestrator", &run_id[run_id.len() - 4..]);
    rig.s.adopt_window(&run_id, rig.window, name, || {}).await;
    until(
        &rig.s,
        &run_id,
        "the adoption was never reported lost",
        lost,
    )
    .await;
    let path = crate::lock(&rig.s.state).runs[PREV]
        .repo_dir
        .join(HISTORY_FILE);
    assert!(
        crate::lock(&HISTORY_READS).contains(&path),
        "its handoff read the history"
    );
    let chain = crate::lock(&rig.s.state).chains[CHAIN].clone();
    assert_eq!(chain.window_id, 0, "no window to follow until it launches");
    rig.stop().await;
}

/// m1 at a restart: the daemon stopped between the step that adopted the window and
/// the adoption, so the manager's record of the window still names the previous run.
/// The restored run is treated as an adoption lost.
#[tokio::test(flavor = "multi_thread")]
async fn a_restart_before_the_adoption_landed_leaves_a_fresh_launch() {
    let (rig, run_id) = adopted().await;
    // The manager's record as it was before the adoption, and the runs on disk.
    let previous = RunRef {
        run_id: PREV.into(),
        task_id: None,
        role: AgentRole::Orchestrator,
        session: 1,
        lane: None,
    };
    rig.manager.rebind_run_window(rig.window, previous).unwrap();
    {
        let state = crate::lock(&rig.s.state);
        for run in state.runs.values() {
            crate::run::journal::save_run(run).unwrap();
        }
    }
    let wiring = GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let restarted = RunService::new(
        rig.manager.clone(),
        context(&rig.checkout, &rig.manager, &wiring),
    );
    tokio::time::timeout(ANSWER, restarted.restore())
        .await
        .expect("the restore returns");
    let shutdown = tokio_util::sync::CancellationToken::new();
    restarted.spawn(shutdown.clone());
    until(
        &restarted,
        &run_id,
        "the restart did not report the adoption lost",
        lost,
    )
    .await;
    shutdown.cancel();
    restarted.stop().await;
    rig.stop().await;
}

/// m5: an adopt reads no history; only a fresh launch needs the handoff. Every read
/// is recorded by its path in tests (`chain_goal::HISTORY_READS`); this rig's history
/// path is its own temporary directory's.
#[tokio::test(flavor = "multi_thread")]
async fn an_adopt_reads_no_history() {
    let (rig, _) = adopted().await;
    let path = crate::lock(&rig.s.state).runs[PREV]
        .repo_dir
        .join(HISTORY_FILE);
    assert!(
        !crate::lock(&HISTORY_READS).contains(&path),
        "the history was read for an adopt"
    );
    rig.stop().await;
}
