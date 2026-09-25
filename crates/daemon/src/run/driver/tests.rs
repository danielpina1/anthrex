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

fn saved_goal(dir: &Path) -> String {
    let text = std::fs::read_to_string(dir.join(crate::run::journal::RUN_FILE)).unwrap();
    let run: crate::run::model::Run = serde_json::from_str(&text).unwrap();
    run.goal
}

/// Ruling T22-N2 (the m8 residual): a `run.json` write asked for earlier, run after a
/// later one (a stuck loop's blocking write finishing late), does not rename the older
/// state over the newer.
#[tokio::test]
async fn an_older_run_json_write_never_lands_over_a_newer_one() {
    use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
    let s = service();
    let data = tempfile::tempdir().unwrap();
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.data_dir = data.path().to_path_buf();
    let (mut older, mut newer) = (run.clone(), run);
    older.goal = "older".into();
    newer.goal = "newer".into();
    let write_older = s.writes.writer(older);
    let write_newer = s.writes.writer(newer);
    write_newer().unwrap();
    write_older().unwrap();
    assert_eq!(saved_goal(data.path()), "newer");
}

/// Ruling T22-N2: the loop's own `stop_now` is stuck saving run `a` when `stop()` gives
/// up waiting. The fallback still writes every run's last `run.json`, `b`'s included,
/// rather than taking the loop's `stopped` for saves it never finished.
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_whose_loop_is_stuck_saving_still_saves_every_run() {
    use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
    let s = service();
    s.stop_wait_ms.store(200, Ordering::SeqCst);
    let data = tempfile::tempdir().unwrap();
    let base = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    let handle = s.spawn(CancellationToken::new());
    for id in ["a", "b"] {
        let mut run = base.clone();
        run.id = id.to_string();
        run.data_dir = data.path().join(id);
        s.send(EventKind::Start {
            reply: 0,
            run: Box::new(run),
        });
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !["a", "b"].iter().all(|id| {
        data.path()
            .join(id)
            .join(crate::run::journal::RUN_FILE)
            .exists()
    }) {
        assert!(Instant::now() < deadline, "the runs were never saved");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    for run in crate::lock(&s.state).runs.values_mut() {
        run.goal = "at stop".into();
    }
    // `a`'s writes wait on a lock the test holds, on a thread of its own.
    let (locked_tx, locked) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel::<()>();
    let slot = s.writes.slot("a");
    let holder = std::thread::spawn(move || {
        let _held = crate::lock(&slot);
        locked_tx.send(()).unwrap();
        let _ = release_rx.recv();
    });
    locked.recv().unwrap();
    let stopper = {
        let s = s.clone();
        tokio::spawn(async move { s.stop().await })
    };
    // The fallback has begun once it has taken the loop's abort handle.
    let deadline = Instant::now() + Duration::from_secs(10);
    while crate::lock(&s.loop_abort).is_some() {
        assert!(Instant::now() < deadline, "stop() never fell back");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    release.send(()).unwrap();
    holder.join().unwrap();
    tokio::time::timeout(Duration::from_secs(10), stopper)
        .await
        .expect("stop returns")
        .unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
    assert_eq!(saved_goal(&data.path().join("a")), "at stop");
    assert_eq!(
        saved_goal(&data.path().join("b")),
        "at stop",
        "stop() returned with b's last run.json never written"
    );
}

/// A run whose next change panics the reducer: its revision is at `u64::MAX`, so the
/// bump `finish` (or the restore) gives a changed run overflows (debug builds check
/// arithmetic, as every test build does). Final review B-I2's stand-in for any bug in
/// `step`.
fn poisoned_run(id: &str, data: &Path) -> crate::run::model::Run {
    use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.id = id.to_string();
    run.data_dir = data.join("runs").join(id);
    run.state = proto::RunState::Running;
    run.revision = u64::MAX;
    run
}

/// Final review B-I2: a panic in `step` leaves the engine's state as it was before the
/// event, fails that event's request, and keeps the event loop alive for the next one.
#[tokio::test(flavor = "multi_thread")]
async fn a_panicking_step_keeps_the_state_and_the_loop() {
    let s = service();
    let data = tempfile::tempdir().unwrap();
    let run = poisoned_run("bad", data.path());
    crate::lock(&s.state)
        .runs
        .insert(run.id.clone(), run.clone());
    let handle = s.spawn(CancellationToken::new());
    let answer = tokio::time::timeout(
        Duration::from_secs(10),
        s.ask(|reply| EventKind::Cancel {
            reply,
            run_id: "bad".into(),
        }),
    )
    .await
    .expect("a request whose step panicked is answered");
    let error = answer.expect_err("the request failed");
    assert!(error.contains("run engine failed"), "{error}");
    assert_eq!(crate::lock(&s.state).runs.get("bad"), Some(&run));
    assert_eq!(s.current().runs.len(), 1);
    // The loop still steps: an unrelated request gets the engine's own answer.
    let answer = tokio::time::timeout(
        Duration::from_secs(10),
        s.ask(|reply| EventKind::Cancel {
            reply,
            run_id: "nope".into(),
        }),
    )
    .await
    .expect("the loop still answers");
    let error = answer.expect_err("an unknown run is refused");
    assert!(!error.contains("shutting down"), "{error}");
    assert!(!handle.is_finished());
    s.stop().await;
}

/// Final review B-I2: a run whose restore panics is left out, the others are restored,
/// and the daemon starts.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_whose_restore_panics_does_not_stop_the_others() {
    let data = tempfile::tempdir().unwrap();
    let config = ManagerConfig::new("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let s = RunService::for_manager(&manager, data.path().to_path_buf(), Arc::new(NoRoots));
    let bad = poisoned_run("bad", data.path());
    let mut good = poisoned_run("good", data.path());
    good.revision = 3;
    for run in [&bad, &good] {
        crate::run::journal::save_run(run).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(60), s.restore())
        .await
        .expect("the restore returns");
    let state = crate::lock(&s.state);
    assert_eq!(state.runs.keys().collect::<Vec<_>>(), vec!["good"]);
    assert_eq!(state.runs["good"].state, proto::RunState::Paused);
}
