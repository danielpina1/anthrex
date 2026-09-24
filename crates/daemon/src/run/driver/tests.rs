use super::*;
use crate::run::engine::Effect;
use std::path::Path;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

fn service() -> Arc<RunService> {
    let config = ManagerConfig::new("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    RunService::for_manager(&manager, "/tmp/ax-unused-data".into(), Arc::new(NoRoots))
}

fn exit(window_id: u32, pid: u32) -> EventKind {
    EventKind::Signal {
        window_id,
        signal: AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: false,
            pid,
        },
    }
}

/// Ruling T22-minors, m6: a killed window's record goes with its exit, or with the
/// window, so `Book.killed` never grows over a daemon's life.
#[tokio::test]
async fn the_killed_record_goes_with_its_exit_or_its_window() {
    let s = service();
    crate::lock(&s.book).killed.insert(3, 41);
    crate::lock(&s.book).killed.insert(4, 42);
    let marked = s.mark_killed(exit(3, 41));
    assert!(matches!(
        marked,
        EventKind::Signal {
            signal: AgentSignal::ProcessExited {
                killed_by_engine: true,
                ..
            },
            ..
        }
    ));
    assert!(!crate::lock(&s.book).killed.contains_key(&3));
    s.execute(
        vec![effects::Ready::Effect(Effect::RemoveWindow {
            window_id: 4,
        })],
        0,
    )
    .await;
    assert!(crate::lock(&s.book).killed.is_empty());
}

/// Ruling T22-minors, m8: when the loop does not acknowledge `Stop` in time,
/// `stop()` aborts it and waits until it is gone before writing any `run.json`, so
/// the fallback never writes beside a live loop. The loop is held here inside an
/// intent append (the journal lock is the test's).
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_that_times_out_waits_for_the_loop_to_go() {
    use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
    let s = service();
    s.stop_wait_ms.store(200, Ordering::SeqCst);
    let data = tempfile::tempdir().unwrap();
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.data_dir = data.path().to_path_buf();
    // The journal lock, held on a thread of its own until `release` is sent.
    let (locked_tx, locked) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel::<()>();
    let journal = s.journal.clone();
    let holder = std::thread::spawn(move || {
        let _held = crate::lock(&journal);
        locked_tx.send(()).unwrap();
        let _ = release_rx.recv();
    });
    locked.recv().unwrap();
    let handle = s.spawn(CancellationToken::new());
    s.send(EventKind::Start {
        reply: 0,
        run: Box::new(run),
    });
    // The loop reaches the intent append, and waits there on the held lock.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !data.path().join(crate::run::journal::RUN_FILE).exists() {
        assert!(Instant::now() < deadline, "the run was never saved");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::timeout(Duration::from_secs(10), s.stop())
        .await
        .expect("stop returns");
    let loop_gone = tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .is_ok();
    release.send(()).unwrap();
    holder.join().unwrap();
    assert!(
        loop_gone,
        "stop() wrote run.json while its loop was still running"
    );
}
