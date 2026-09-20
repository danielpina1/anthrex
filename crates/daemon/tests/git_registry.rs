//! The git registry end to end: its tokio task per root, its publication rules and
//! its real `notify` watcher (M4.5.5).
//!
//! Design decision 30 binds this suite too: nothing here sleeps either, bar one test.
//! These run under `#[tokio::test(start_paused = true)]`, where tokio's clock is
//! virtual — every timeout below is a *virtual* duration that elapses instantly, so
//! `expect_no_publication(.., LONG)` really does cross four poll deadlines without
//! the suite taking four poll intervals to run.
//!
//! `a_real_write_triggers_a_probe` is the single exception the brief allows: it uses
//! the real watcher against a real repository and waits on wall-clock time, with a
//! five second deadline.
//!
//! The pure scheduler, publisher and path filter are tested in
//! `crates/daemon/tests/git_schedule.rs`.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use daemon::git::{GitRegistry, ProbeFn, enabled_from_env};
use proto::{GitState, Head};
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

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

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// A virtual deadline for "this should arrive". Well under [`POLL_INTERVAL`], so
/// waiting this long never triggers another probe of its own.
const SOON: Duration = Duration::from_secs(5);
/// A virtual deadline for "nothing more should arrive". Deliberately longer than four
/// poll intervals, so a registry that kept polling would be caught.
const LONG: Duration = Duration::from_secs(120);

type Publication = (PathBuf, Option<GitState>);

/// The injected probe: records every call, blocks on an optional gate, and returns the
/// next scripted result (repeating the last one once the script runs out, so that the
/// extra probes a poll fires between assertions cannot change what gets published).
struct Harness {
    total: AtomicUsize,
    live: AtomicUsize,
    max_live: AtomicUsize,
    results: Mutex<VecDeque<Option<GitState>>>,
    last: Mutex<Option<GitState>>,
    starts: mpsc::UnboundedSender<PathBuf>,
    gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl Harness {
    fn next_result(&self) -> Option<GitState> {
        match self.results.lock().unwrap().pop_front() {
            Some(result) => {
                *self.last.lock().unwrap() = result.clone();
                result
            }
            None => self.last.lock().unwrap().clone(),
        }
    }
}

struct Fake {
    probe: ProbeFn,
    harness: Arc<Harness>,
    starts: mpsc::UnboundedReceiver<PathBuf>,
}

fn fake_probe(results: Vec<Option<GitState>>, gate: Option<std::sync::mpsc::Receiver<()>>) -> Fake {
    let (starts_tx, starts) = mpsc::unbounded_channel();
    let harness = Arc::new(Harness {
        total: AtomicUsize::new(0),
        live: AtomicUsize::new(0),
        max_live: AtomicUsize::new(0),
        results: Mutex::new(results.into_iter().collect()),
        last: Mutex::new(None),
        starts: starts_tx,
        gate: Mutex::new(gate),
    });
    let inner = Arc::clone(&harness);
    let probe: ProbeFn = Arc::new(move |root: &Path| {
        inner.total.fetch_add(1, Ordering::SeqCst);
        let live = inner.live.fetch_add(1, Ordering::SeqCst) + 1;
        inner.max_live.fetch_max(live, Ordering::SeqCst);
        let _ = inner.starts.send(root.to_path_buf());
        if let Some(gate) = inner.gate.lock().unwrap().as_ref() {
            let _ = gate.recv();
        }
        let result = inner.next_result();
        inner.live.fetch_sub(1, Ordering::SeqCst);
        result
    });
    Fake {
        probe,
        harness,
        starts,
    }
}

fn scripted(results: Vec<Option<GitState>>) -> Fake {
    fake_probe(results, None)
}

async fn next_publication(rx: &mut mpsc::UnboundedReceiver<Publication>) -> Publication {
    tokio::time::timeout(SOON, rx.recv())
        .await
        .expect("expected a publication")
        .expect("the registry dropped the publish channel")
}

async fn expect_no_publication(rx: &mut mpsc::UnboundedReceiver<Publication>, within: Duration) {
    if let Ok(unexpected) = tokio::time::timeout(within, rx.recv()).await {
        panic!("expected no further publication, got {unexpected:?}");
    }
}

// ---------------------------------------------------------------------------
// The registry
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn registering_a_root_probes_it_immediately() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let Fake {
        probe,
        harness,
        mut starts,
    } = scripted(vec![Some(state(0))]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let registry = GitRegistry::with_probe(true, tx, probe);

    registry.register(root.clone());

    let probed = tokio::time::timeout(SOON, starts.recv())
        .await
        .expect("registration must probe without waiting for the poll")
        .unwrap();
    assert_eq!(probed, root);
    assert_eq!(next_publication(&mut rx).await, (root, Some(state(0))));
    assert_eq!(harness.max_live.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn a_disabled_registry_never_probes() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let Fake {
        probe,
        harness,
        starts: _starts,
    } = scripted(vec![Some(state(0))]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let registry = GitRegistry::with_probe(false, tx, probe);

    registry.register(root);

    expect_no_publication(&mut rx, LONG).await;
    assert_eq!(harness.total.load(Ordering::SeqCst), 0);
    assert!(registry.snapshot().is_empty());
}

#[tokio::test(start_paused = true)]
async fn state_changes_are_published_once() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let Fake {
        probe,
        harness,
        starts: _starts,
    } = scripted(vec![
        Some(state(1)),
        Some(state(1)),
        Some(state(1)),
        Some(state(2)),
    ]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let registry = GitRegistry::with_probe(true, tx, probe);

    registry.register(root.clone());

    // The repeated identical results are not published a second time, so the next
    // publication after the first is the changed one, not another copy of it.
    assert_eq!(
        next_publication(&mut rx).await,
        (root.clone(), Some(state(1)))
    );
    let second = tokio::time::timeout(LONG, rx.recv())
        .await
        .expect("the changed state must be published")
        .unwrap();
    assert_eq!(second, (root.clone(), Some(state(2))));
    expect_no_publication(&mut rx, LONG).await;

    assert!(
        harness.total.load(Ordering::SeqCst) >= 4,
        "the poll must have probed repeatedly"
    );
    assert_eq!(
        harness.max_live.load(Ordering::SeqCst),
        1,
        "probes overlapped"
    );
    assert_eq!(registry.snapshot(), vec![(root, Some(state(2)))]);
}

#[tokio::test(start_paused = true)]
async fn a_transient_failure_republishes_the_last_state_as_stale_through_the_registry() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let Fake {
        probe,
        harness: _harness,
        starts: _starts,
    } = scripted(vec![Some(state(1)), None, Some(state(1))]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let registry = GitRegistry::with_probe(true, tx, probe);

    registry.register(root.clone());

    let stale = GitState {
        stale: true,
        ..state(1)
    };
    for expected in [Some(state(1)), Some(stale), Some(state(1))] {
        let got = tokio::time::timeout(LONG, rx.recv())
            .await
            .expect("expected a publication")
            .unwrap();
        assert_eq!(got, (root.clone(), expected));
    }
    expect_no_publication(&mut rx, LONG).await;
}

#[tokio::test(start_paused = true)]
async fn a_root_that_never_succeeded_publishes_none_once_through_the_registry() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let Fake {
        probe,
        harness,
        starts: _starts,
    } = scripted(vec![None]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let registry = GitRegistry::with_probe(true, tx, probe);

    registry.register(root.clone());

    assert_eq!(next_publication(&mut rx).await, (root, None));
    expect_no_publication(&mut rx, LONG).await;
    assert!(
        harness.total.load(Ordering::SeqCst) >= 2,
        "the poll kept probing; it just stopped saying so"
    );
}

#[tokio::test(start_paused = true)]
async fn unregistering_stops_the_probes() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let Fake {
        probe,
        harness,
        starts: _starts,
    } = scripted(vec![Some(state(1))]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let registry = GitRegistry::with_probe(true, tx, probe);

    registry.register(root.clone());
    assert_eq!(
        next_publication(&mut rx).await,
        (root.clone(), Some(state(1)))
    );

    // First establish that the poll really is firing in this test's virtual time —
    // otherwise "it stopped probing" would be true of a registry that never started.
    expect_no_publication(&mut rx, LONG).await;
    let while_registered = harness.total.load(Ordering::SeqCst);
    assert!(
        while_registered >= 4,
        "the poll must probe while the root is registered, got {while_registered}"
    );

    registry.unregister(&root);
    assert!(registry.snapshot().is_empty());

    // Neither the poll nor anything else probes again: the count is read after a
    // window in which four more polls would have fired, and must not move across a
    // second such window.
    expect_no_publication(&mut rx, LONG).await;
    let after_unregister = harness.total.load(Ordering::SeqCst);
    expect_no_publication(&mut rx, LONG).await;
    assert_eq!(
        harness.total.load(Ordering::SeqCst),
        after_unregister,
        "unregistering must stop the poll"
    );
}

#[tokio::test(start_paused = true)]
async fn unregistering_during_a_probe_publishes_nothing_afterwards() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let (release, gate) = std::sync::mpsc::channel();
    let Fake {
        probe,
        harness,
        mut starts,
    } = fake_probe(vec![Some(state(7))], Some(gate));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let registry = GitRegistry::with_probe(true, tx, probe);

    registry.register(root.clone());
    tokio::time::timeout(SOON, starts.recv())
        .await
        .expect("the registration probe must start")
        .unwrap();

    // The probe is now blocked inside `spawn_blocking`. Unregistering must not panic,
    // and its result must never reach the channel.
    registry.unregister(&root);
    release.send(()).unwrap();

    expect_no_publication(&mut rx, LONG).await;
    assert_eq!(harness.max_live.load(Ordering::SeqCst), 1);
}

/// M4.5.6 review finding: `server.rs` used to derive "is this the last window on this
/// root" from a `manager.list()` snapshot, which races a concurrent `register` on the
/// same root across two independent locks. The fix moves reference counting into the
/// registry itself, so `register` and `unregister` commute regardless of arrival order.
/// This test pins that contract directly against the registry, independent of the
/// server: two `register` calls must be undone by two `unregister` calls, and the root
/// must keep probing after only one of them.
#[tokio::test(start_paused = true)]
async fn a_root_survives_until_every_registration_is_released() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let Fake {
        probe,
        harness,
        starts: _starts,
    } = scripted(vec![Some(state(0))]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let registry = GitRegistry::with_probe(true, tx, probe);

    registry.register(root.clone());
    registry.register(root.clone());
    assert_eq!(
        next_publication(&mut rx).await,
        (root.clone(), Some(state(0)))
    );
    assert_eq!(
        harness.total.load(Ordering::SeqCst),
        1,
        "a second registration of an already-live root must not start a second task"
    );

    registry.unregister(&root);
    // Still referenced once: the poll keeps firing, and the snapshot still knows the
    // root, across a window that would have caught a task that tore down early.
    expect_no_publication(&mut rx, LONG).await;
    assert!(
        harness.total.load(Ordering::SeqCst) >= 4,
        "the root must still be polled after releasing only one of two registrations"
    );
    assert_eq!(registry.snapshot(), vec![(root.clone(), Some(state(0)))]);

    registry.unregister(&root);
    assert!(
        registry.snapshot().is_empty(),
        "the second unregister must be the one that actually tears the root down"
    );
    let after_last_unregister = harness.total.load(Ordering::SeqCst);
    expect_no_publication(&mut rx, LONG).await;
    assert_eq!(
        harness.total.load(Ordering::SeqCst),
        after_last_unregister,
        "no probe may run once every registration has been released"
    );
}

#[test]
fn enabled_from_env_reads_anthrex_git() {
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // SAFETY: this is the only test in this binary that touches ANTHREX_GIT, and
    // ENV_LOCK serialises it against itself.
    unsafe {
        std::env::remove_var("ANTHREX_GIT");
    }
    assert!(enabled_from_env(), "unset leaves git on");

    for (value, expected) in [("off", false), ("0", false), ("", true), ("on", true)] {
        unsafe {
            std::env::set_var("ANTHREX_GIT", value);
        }
        assert_eq!(enabled_from_env(), expected, "ANTHREX_GIT={value:?}");
    }

    unsafe {
        std::env::remove_var("ANTHREX_GIT");
    }
}

// ---------------------------------------------------------------------------
// The one test allowed to wait on wall-clock time
// ---------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed with {status}");
}

fn repo_with_one_committed_file() -> TempDir {
    let dir = tempdir().unwrap();
    git(dir.path(), &["init"]);
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    git(dir.path(), &["add", "a.txt"]);
    git(dir.path(), &["commit", "-m", "init"]);
    dir
}

/// The real `notify` watcher, a real repository and the real probe. This is the only
/// test in the suite that waits on wall-clock time, and it waits with a deadline.
#[tokio::test]
async fn a_real_write_triggers_a_probe() {
    if std::process::Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .status()
        .is_err()
    {
        return;
    }
    let repo = repo_with_one_committed_file();
    let root = repo.path().to_path_buf();
    let (tx, mut rx) = mpsc::unbounded_channel::<Publication>();
    let registry = GitRegistry::new(true, tx);
    registry.register(root.clone());

    // The registration probe sees a clean tree.
    let first = tokio::time::timeout(SOON, rx.recv())
        .await
        .expect("the registration probe must publish")
        .unwrap();
    assert_eq!(first.0, root);
    assert_eq!(first.1.as_ref().map(|s| s.dirty), Some(0));

    std::fs::write(root.join("a.txt"), "two\n").unwrap();

    let deadline = std::time::Instant::now() + secs(5);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let publication = tokio::time::timeout(left, rx.recv())
            .await
            .expect("a real write must trigger a probe within five seconds")
            .unwrap();
        if publication.1.as_ref().is_some_and(|s| s.dirty == 1) {
            break;
        }
    }
}
