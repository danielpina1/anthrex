//! Milestone 9.16 task M9.16.7: the readings `forward` feeds the engine. Each test builds
//! a [`Quota`] with its own sink, runtime lookup, sessions folder (always a `tempfile`
//! directory, never `~/.codex`) and clock; a rollout read's task handle is awaited, so
//! no test waits on a fixed sleep.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::Runtime;
use proto::brakes::{Permille, QuotaWindow};
use tokio::sync::mpsc;

use super::{QUOTA_READ_WAIT, Quota, ROLLOUT_READ_EVERY};
use crate::headless::quota::QuotaSample;
use crate::headless::{SessionEvent, TurnOutcome};
use crate::manager::{WindowSignal, WindowSignalKind};
use crate::run::driver::translate;
use crate::run::engine::{AgentSignal, EventKind};

const CODEX: u32 = 7;
const CLAUDE: u32 = 3;
const THREAD: &str = "019a0000-0000-7000-8000-00000000abcd";
/// 2026-10-15 12:00:00 UTC.
const START: u64 = 1_792_065_600;
/// A bound no test should come near: the read's own wait is far shorter.
const GUARD: Duration = Duration::from_secs(20);

fn feed(sessions: Option<PathBuf>, wait: Duration) -> (Quota, mpsc::UnboundedReceiver<EventKind>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let runtimes = HashMap::from([(CODEX, Runtime::Codex), (CLAUDE, Runtime::Claude)]);
    let quota = Quota::new(
        Arc::new(move |kind| {
            let _ = tx.send(kind);
        }),
        Arc::new(move |window| runtimes.get(&window).copied()),
        sessions,
    )
    .with_read_wait(wait);
    (quota, rx)
}

fn signal(window_id: u32, event: SessionEvent) -> WindowSignal {
    WindowSignal {
        window_id,
        pid: Some(4242),
        kind: WindowSignalKind::Session(event),
    }
}

fn init(window: u32) -> WindowSignal {
    signal(
        window,
        SessionEvent::Init {
            session_id: THREAD.into(),
            model: None,
            mcp_ok: None,
        },
    )
}

fn turn_ended(window: u32) -> WindowSignal {
    signal(
        window,
        SessionEvent::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: None,
            denials: Vec::new(),
        },
    )
}

/// `<sessions>/2026/10/15/rollout-…-<THREAD>.jsonl`, its path.
fn rollout_path(sessions: &Path) -> PathBuf {
    let dir = sessions.join("2026").join("10").join("15");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("rollout-2026-10-15T13-00-00-{THREAD}.jsonl"))
}

/// A rollout whose last `token_count` puts the 7-day window at `percent`.
fn write_rollout(sessions: &Path, percent: f64) -> PathBuf {
    let path = rollout_path(sessions);
    let line = serde_json::json!({
        "timestamp": "2026-10-15T12:00:05.000Z",
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": null,
            "rate_limits": {
                "primary": {"used_percent": percent, "window_minutes": 10080, "resets_at": 1_792_600_000u64},
                "secondary": null,
            },
        },
    });
    std::fs::write(&path, format!("{{\"type\":\"session_meta\"}}\n{line}\n")).unwrap();
    path
}

fn seven_day(permille: u16) -> QuotaSample {
    QuotaSample {
        windows: vec![(
            QuotaWindow::SevenDay,
            Permille(permille),
            Some(1_792_600_000),
        )],
        rejected: None,
    }
}

async fn finished(handle: Option<tokio::task::JoinHandle<()>>) {
    let handle = handle.expect("a rollout read was started");
    tokio::time::timeout(GUARD, handle)
        .await
        .expect("the read's task is bounded")
        .expect("the read's task did not panic");
}

#[tokio::test]
async fn claude_quota_event_reaches_the_engine() {
    let (quota, mut rx) = feed(None, QUOTA_READ_WAIT);
    let sample = seven_day(530);
    let reading = signal(CLAUDE, SessionEvent::Quota(sample.clone()));
    assert!(quota.observe_at(&reading, Instant::now(), START).is_none());
    assert_eq!(
        rx.try_recv().ok(),
        Some(EventKind::Quota {
            runtime: Runtime::Claude,
            sample,
        })
    );
    assert!(rx.try_recv().is_err(), "one event");
    // Activity throttling is unchanged: the reading is still activity, once a second.
    let (mut last, now) = (HashMap::new(), Instant::now());
    assert_eq!(
        translate(&reading, &mut last, now),
        Some(AgentSignal::Activity)
    );
    assert_eq!(translate(&reading, &mut last, now), None);
}

#[tokio::test]
async fn codex_turn_end_reads_the_rollout_once_a_minute() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions");
    write_rollout(&sessions, 99.0);
    let (quota, mut rx) = feed(Some(sessions.clone()), QUOTA_READ_WAIT);
    let t0 = Instant::now();
    assert!(quota.observe_at(&init(CODEX), t0, START).is_none());
    finished(quota.observe_at(&turn_ended(CODEX), t0, START)).await;
    let want = |permille| EventKind::Quota {
        runtime: Runtime::Codex,
        sample: seven_day(permille),
    };
    assert_eq!(rx.try_recv().ok(), Some(want(990)));
    // Ten seconds later: no read.
    let t10 = t0 + Duration::from_secs(10);
    assert!(
        quota
            .observe_at(&turn_ended(CODEX), t10, START + 10)
            .is_none()
    );
    assert!(rx.try_recv().is_err());
    // A minute after the first: read again, a fresher reading.
    write_rollout(&sessions, 40.0);
    let t60 = t0 + ROLLOUT_READ_EVERY;
    finished(quota.observe_at(&turn_ended(CODEX), t60, START + 60)).await;
    assert_eq!(rx.try_recv().ok(), Some(want(400)));
    assert!(rx.try_recv().is_err());
    // A Claude window's turn end never reads a rollout.
    let t120 = t0 + 2 * ROLLOUT_READ_EVERY;
    assert!(quota.observe_at(&init(CLAUDE), t120, START + 120).is_none());
    assert!(
        quota
            .observe_at(&turn_ended(CLAUDE), t120, START + 120)
            .is_none()
    );
}

/// Ruling C26: the rollout is a FIFO nobody writes, so the read blocks in `open`; the
/// writer thread opens it once the read has (or `O_RDWR` past its deadline, which never
/// blocks), and holds it until the test is done, so the abandoned read ends (EOF).
#[tokio::test(flavor = "multi_thread")]
async fn codex_rollout_read_times_out_cleanly() {
    use std::os::unix::fs::OpenOptionsExt;
    let wait = Duration::from_millis(200);
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions");
    let fifo = rollout_path(&sessions);
    let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let held = fifo.clone();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let writer = std::thread::spawn(move || {
        let deadline = Instant::now() + wait * 100;
        let file = loop {
            let opened = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&held);
            match opened {
                Ok(file) => break file,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => {
                    let mut rdwr = std::fs::OpenOptions::new();
                    rdwr.read(true).write(true);
                    break rdwr.open(&held).expect("a FIFO opens O_RDWR");
                }
            }
        };
        let _ = released.recv_timeout(wait * 100);
        let _ = std::fs::remove_file(&held);
        drop(file);
    });

    let (quota, mut rx) = feed(Some(sessions), wait);
    let now = Instant::now();
    quota.observe_at(&init(CODEX), now, START);
    let read = quota.observe_at(&turn_ended(CODEX), now, START);
    // `observe` never waits for the read: the next signal is delivered at once.
    let sample = seven_day(530);
    quota.observe_at(
        &signal(CLAUDE, SessionEvent::Quota(sample.clone())),
        now,
        START,
    );
    assert_eq!(
        rx.try_recv().ok(),
        Some(EventKind::Quota {
            runtime: Runtime::Claude,
            sample,
        })
    );
    finished(read).await;
    assert!(
        rx.try_recv().is_err(),
        "a read that timed out sends nothing"
    );
    let _ = release.send(());
    tokio::task::spawn_blocking(move || writer.join().unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn missing_rollout_is_unmeasured() {
    let dir = tempfile::tempdir().unwrap();
    let (quota, mut rx) = feed(Some(dir.path().join("sessions")), QUOTA_READ_WAIT);
    let now = Instant::now();
    quota.observe_at(&init(CODEX), now, START);
    finished(quota.observe_at(&turn_ended(CODEX), now, START)).await;
    assert!(rx.try_recv().is_err());
    // No sessions folder known (no `HOME`, no `CODEX_HOME`): nothing is read at all.
    let (quota, mut rx) = feed(None, QUOTA_READ_WAIT);
    quota.observe_at(&init(CODEX), now, START);
    assert!(quota.observe_at(&turn_ended(CODEX), now, START).is_none());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn context_window_is_learned() {
    let (quota, mut rx) = feed(None, QUOTA_READ_WAIT);
    let learned = signal(
        CLAUDE,
        SessionEvent::ContextWindow {
            model: "claude-opus-5-5[1m]".into(),
            tokens: 1_000_000,
        },
    );
    assert!(quota.observe_at(&learned, Instant::now(), START).is_none());
    assert!(rx.try_recv().is_err(), "nothing for the engine");
    assert_eq!(
        quota.windows(),
        [("claude-opus-5-5[1m]".to_string(), 1_000_000)]
    );
    // Decision 23 (4): matched exactly.
    assert_eq!(
        quota.learned_window("claude-opus-5-5[1m]", "claude-opus-5-5[1m]"),
        Some(1_000_000)
    );
}

/// Decision 23 (4) with the controller's ruling (M9.16.1 finding 2): a transcript line's
/// `message.model` never carries `[1m]`, so the canonical match applies only when the
/// route's own model carries the tag, or when the learned entry is the only one for that
/// canonical id; a plain route never inherits a tagged (1M) window.
#[tokio::test]
async fn the_canonical_match_needs_the_routes_tag_or_a_single_entry() {
    let (quota, _rx) = feed(None, QUOTA_READ_WAIT);
    let learn = |model: &str, tokens: u64| {
        let event = SessionEvent::ContextWindow {
            model: model.into(),
            tokens,
        };
        quota.observe_at(&signal(CLAUDE, event), Instant::now(), START);
    };
    // Only the 1M session's entry: a 1M route takes it; a 200k route does not.
    learn("claude-opus-5-5[1m]", 1_000_000);
    let line = "claude-opus-5-5";
    assert_eq!(
        quota.learned_window(line, "claude-opus-5-5[1m]"),
        Some(1_000_000)
    );
    assert_eq!(quota.learned_window(line, "claude-opus-5-5"), None);
    // A dated 200k entry, the only plain one: the plain route takes it by canonical id.
    learn("claude-opus-5-5-20261001", 200_000);
    assert_eq!(quota.learned_window(line, "claude-opus-5-5"), Some(200_000));
    assert_eq!(
        quota.learned_window(line, "claude-opus-5-5[1m]"),
        Some(1_000_000)
    );
    // Two plain entries for the same canonical id: ambiguous, no match.
    learn("claude-opus-5-5-20261101", 400_000);
    assert_eq!(quota.learned_window(line, "claude-opus-5-5"), None);
    // An exact key always wins.
    learn("claude-opus-5-5", 200_000);
    assert_eq!(quota.learned_window(line, "claude-opus-5-5"), Some(200_000));
    assert_eq!(
        quota.learned_window("claude-haiku-4-5", "claude-haiku-4-5"),
        None
    );
}

/// `forward` makes the one `observe` call per signal, before the signal's own event: a
/// Claude reading reaches the engine's queue, then the signal's activity.
#[tokio::test]
async fn forward_sends_the_reading_before_the_signal() {
    use crate::manager::{GitRoots, ManagerConfig, WindowManager};
    use crate::run::driver::{Msg, RunService};
    struct NoRoots;
    impl GitRoots for NoRoots {
        fn register(&self, _: PathBuf) {}
        fn unregister(&self, _: &Path) {}
    }
    let dir = tempfile::tempdir().unwrap();
    let config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let s = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let mut rx = crate::lock(&s.rx).take().expect("the loop is not running");
    let (tx, feed) = tokio::sync::broadcast::channel(16);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let forwarder = tokio::spawn(s.clone().forward(feed, shutdown.clone()));
    let sample = seven_day(530);
    tx.send(signal(CLAUDE, SessionEvent::Quota(sample.clone())))
        .unwrap();
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + GUARD;
    while seen.len() < 2 {
        let msg = tokio::time::timeout_at(deadline, rx.recv()).await;
        match msg.expect("both events within the guard") {
            Some(Msg::Event(kind)) => seen.push(kind),
            Some(_) => {}
            None => panic!("the queue closed"),
        }
    }
    assert_eq!(
        seen,
        [
            EventKind::Quota {
                runtime: Runtime::Claude,
                sample,
            },
            EventKind::Signal {
                window_id: CLAUDE,
                signal: AgentSignal::Activity,
            },
        ]
    );
    shutdown.cancel();
    forwarder.await.unwrap();
}
