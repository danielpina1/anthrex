//! A root's first probe must not wait for its watcher, and nothing that changed while
//! the watcher was still being built may be lost.
//!
//! Arming a real watcher is not bounded by anything this crate controls. On macOS,
//! `notify`'s `watch()` starts an FSEvents stream, and the first time a freshly built
//! executable does that it has been measured taking 1.5 to 7.8 seconds, for every root
//! in the process at once (`git probe` itself stayed around 25 ms throughout). Before
//! this was fixed the registration probe only started after the watcher was armed, so
//! for those seconds every root stayed blank; that was the intermittent failure in
//! `crates/daemon/tests/server_restore_git.rs`.
//!
//! These tests inject the watcher builder and hold it on a gate, which is the only
//! deterministic way to make "arming is slow" happen. They run on the real clock and
//! the injected probe returns at once, so every deadline below is pure slack, never a
//! budget the code is entitled to use.

use std::path::Path;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use daemon::git::watch::WatchGuard;
use daemon::git::{ArmFn, GitRegistry, ProbeFn};
use proto::{GitState, Head};
use tempfile::tempdir;
use tokio::sync::mpsc;

/// Slack for a publication that needs nothing but an instant fake probe and a
/// `spawn_blocking` hop. Far below `POLL_SECS`, so the safety poll can never be what
/// satisfies a wait.
const SLACK: Duration = Duration::from_secs(10);

/// A safety poll far beyond any wait here, so the only probes are the registration
/// probe and the ones the code under test asks for itself.
const POLL_SECS: u64 = 3600;

fn settings() -> config::Git {
    config::Git {
        enabled: true,
        poll_secs: POLL_SECS,
        ..config::Git::default()
    }
}

fn state(dirty: u32) -> GitState {
    GitState {
        head: Head::Branch("main".to_string()),
        upstream: None,
        ahead: 0,
        behind: 0,
        dirty,
        untracked: 0,
        conflicts: 0,
        operation: None,
        stale: false,
    }
}

/// A probe that answers whatever `answer` currently holds, so a test can change what
/// the worktree "looks like" while the watcher is still unarmed.
fn probe_answering(answer: &Arc<Mutex<GitState>>) -> ProbeFn {
    let answer = Arc::clone(answer);
    Arc::new(move |_: &Path| Some(answer.lock().unwrap().clone()))
}

/// A watcher builder that blocks until the returned sender sends (or is dropped), then
/// reports success. It never delivers an event, so any probe after the first one can
/// only have come from the registry asking for it.
fn gated_arm() -> (ArmFn, std_mpsc::Sender<()>) {
    let (open, gate) = std_mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let arm: ArmFn = Arc::new(move |_root, _ignore, _events| {
        let _ = gate.lock().unwrap().recv();
        Ok(WatchGuard::new(()))
    });
    (arm, open)
}

async fn next_state(
    rx: &mut mpsc::UnboundedReceiver<(std::path::PathBuf, Option<GitState>)>,
) -> Option<GitState> {
    tokio::time::timeout(SLACK, rx.recv())
        .await
        .expect("no publication")
        .expect("the registry dropped the publish channel")
        .1
}

/// Design decision 13: a registered root is probed *immediately*. A watcher that takes
/// seconds to arm must not hold that probe back.
#[tokio::test]
async fn the_first_probe_does_not_wait_for_the_watcher() {
    let dir = tempdir().unwrap();
    let answer = Arc::new(Mutex::new(state(0)));
    let (arm, _still_arming) = gated_arm();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let registry = GitRegistry::with_seams(settings(), tx, probe_answering(&answer), arm);

    registry.register(dir.path().to_path_buf());

    assert_eq!(
        next_state(&mut rx).await,
        Some(state(0)),
        "the registration probe waited for the watcher to arm"
    );
}

/// The other half of the same fix. A change landing after the first probe has read the
/// worktree but before the watcher exists produces no event, so without a probe of its
/// own once the watcher is armed it would stay unpublished until the safety poll —
/// thirty seconds by default, an hour here.
#[tokio::test]
async fn a_change_made_before_the_watcher_armed_is_published_once_it_arms() {
    let dir = tempdir().unwrap();
    let answer = Arc::new(Mutex::new(state(0)));
    let (arm, open) = gated_arm();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let registry = GitRegistry::with_seams(settings(), tx, probe_answering(&answer), arm);

    registry.register(dir.path().to_path_buf());
    assert_eq!(next_state(&mut rx).await, Some(state(0)));

    // The worktree changes while the watcher is still being built...
    *answer.lock().unwrap() = state(1);
    // ...and then the watcher arms.
    open.send(()).unwrap();

    assert_eq!(
        next_state(&mut rx).await,
        Some(state(1)),
        "a change from before the watcher armed was never probed"
    );
}
