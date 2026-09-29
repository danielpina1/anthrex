//! Task M9.13: decision 39's paste, pure, and its writes outside every lock, against a
//! real manager and a real shell window (never an agent).

use std::time::{Duration, Instant};

use proto::{Runtime, WindowSpec};

use super::*;
use crate::launch::LaunchGate;
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::orch::contract::{WAKE_CUT_MARKER, WAKE_MAX_BYTES};

const START: &[u8] = b"\x1b[200~";
const END: &[u8] = b"\x1b[201~";

/// The body of a paste, its brackets checked.
fn body(paste: &[u8]) -> &[u8] {
    assert!(paste.starts_with(START), "{paste:?}");
    assert!(paste.ends_with(END), "{paste:?}");
    &paste[START.len()..paste.len() - END.len()]
}

#[test]
fn encode_paste_wraps_and_normalises() {
    let paste = encode_paste("[anthrex] one\r\ntwo\nthree\rfour");
    assert_eq!(body(&paste), b"[anthrex] one\rtwo\rthree\rfour");
    // Paste markers inside the text are removed, however they are nested, so the text
    // can neither end the paste early nor open a second one.
    let paste = encode_paste("a\x1b[201~b\x1b[200~c\x1b[20\x1b[201~1~d");
    assert_eq!(body(&paste), b"abcd");
    let inner = body(&paste);
    assert!(!inner.windows(START.len()).any(|w| w == START));
    assert!(!inner.windows(END.len()).any(|w| w == END));
    // Nothing else: one paste, no Enter (the `\r` goes after the delay).
    assert_eq!(encode_paste(""), [START, END].concat());
}

#[test]
fn wake_is_clamped() {
    // `WAKE_MAX_BYTES` is the contract's (`run/orch/contract.rs`); a longer text, which
    // `wake_text` never makes, is still cut to it, on a character boundary.
    let long = "é".repeat(WAKE_MAX_BYTES);
    let paste = encode_paste(&long);
    let inner = body(&paste);
    assert!(inner.len() <= WAKE_MAX_BYTES, "{}", inner.len());
    let text = std::str::from_utf8(inner).expect("cut on a character boundary");
    assert!(text.contains(WAKE_CUT_MARKER.trim()), "{text}");
    // A text at the limit is left whole.
    let exact = "x".repeat(WAKE_MAX_BYTES);
    assert_eq!(body(&encode_paste(&exact)), exact.as_bytes());
}

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: std::path::PathBuf) {}
    fn unregister(&self, _: &std::path::Path) {}
}

/// Decision 39 and AGENTS.md rules 2 and 3: the paste and its `\r` go through the
/// window's writer thread, and the 200 ms before the `\r` is a `tokio` sleep holding no
/// lock: the manager answers `list`, and the engine's lock is free, while it is pending.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wake_writes_happen_outside_every_lock() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let mut config = ManagerConfig::for_tests(socket, "/bin/sh".into());
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.worktrees_root = dir.path().join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = events.recv().await {
            pump.handle_event(id, event);
        }
    });
    let runs = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let spec = WindowSpec {
        name: Some("wake-lock".into()),
        runtime: Runtime::Shell,
        cwd: dir.path().to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let info = manager
        .create(spec, dir.path().to_path_buf(), None, 80, 24)
        .await
        .expect("a shell window");

    let delivering = manager.clone();
    let started = Instant::now();
    let delivery = tokio::spawn(async move {
        deliver_wake(&delivering, info.id, "[anthrex] run r1 changed").await
    });
    tokio::time::sleep(Duration::from_millis(60)).await;
    // Inside the delay: both locks answer at once.
    let asked = Instant::now();
    let listed = tokio::task::spawn_blocking({
        let manager = manager.clone();
        move || manager.list()
    })
    .await
    .unwrap();
    assert!(listed.iter().any(|w| w.id == info.id));
    drop(crate::lock(&runs.state));
    assert!(
        asked.elapsed() < Duration::from_millis(100),
        "{:?}",
        asked.elapsed()
    );
    assert!(!delivery.is_finished(), "the delay was not pending");
    delivery.await.unwrap().expect("delivered");
    assert!(started.elapsed() >= SUBMIT_DELAY, "{:?}", started.elapsed());

    // The window this test started, and nothing else.
    let _ = manager.kill(info.id);
}

/// Decision 39, "a read clears too", in the driver (M9.16, found end to end): a digest
/// answer that held every note of the wake-up still waiting drops that wake-up, so an
/// orchestrator that read the change by polling is not pasted at once its turn ends.
/// A wake-up holding a newer note stays.
#[test]
fn a_digest_read_drops_the_wake_up_it_covered() {
    let wakes = Wakes::default();
    let pending = |notes_seq| Pending {
        window_id: 1,
        text: "[anthrex] Run r1 changed: the user approved the plan.".into(),
        digest_revision: 4,
        notes_seq,
        quiet: Duration::from_secs(1),
    };
    crate::lock(&wakes.pending).insert("r1".into(), pending(3));
    crate::lock(&wakes.pending).insert("r2".into(), pending(3));
    wakes.read("r1", 2);
    assert!(
        crate::lock(&wakes.pending).contains_key("r1"),
        "a newer note"
    );
    wakes.read("r1", 3);
    assert!(!crate::lock(&wakes.pending).contains_key("r1"));
    assert!(
        crate::lock(&wakes.pending).contains_key("r2"),
        "another run's"
    );
}
