//! `RunService` (M8a.22): the engine's driver. It owns the pure reducer's state, feeds it
//! events (client requests, op results, the manager's session feed, a 1-second tick) and
//! executes the effects each step returns, in decision 43's order: `Persist`, then each
//! op's intent line, then the op; when an op returns, its `done` line, then its `OpDone`.
//! Does I/O (design decision 2).
//!
//! **Locks.** The engine lock (`crate::lock(&self.state)`) is held only around `step`, a
//! snapshot, and cloning what the effects of that step need; it is released before any
//! `.await`, `spawn_blocking`, git call or manager call. `book` (the loop's own
//! bookkeeping) is held only for a lookup or an insert, never across either. Every
//! blocking thing — `run.json`, the journal, git, the report — runs on
//! `spawn_blocking` or behind `GitQueue::write` (AGENTS.md rules 2 and 10).
//!
//! **Files.** `driver/effects.rs` executes effects, `driver/ops.rs` runs each op, with
//! `driver/merge.rs` (the merge candidate) and `driver/cleanup.rs` (decision 20's
//! salvage, accept and discard); `driver/observe.rs` translates the session feed,
//! `driver/requests.rs` answers client requests (reading git first for start, finish and
//! resume), and `driver/restore.rs` restores the runs of an earlier daemon (decision 44).

mod cleanup;
mod effects;
mod merge;
mod observe;
mod ops;
mod requests;
mod restore;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use proto::RunsSnapshot;
use tokio::sync::{RwLock, broadcast, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use super::engine::{
    AgentSignal, EngineState, Event, EventKind, INTERRUPT_GRACE_SECS, ReplyId, step,
};
use super::git::GitQueue;
use super::snapshot::snapshot;
use crate::headless::argv::CliCaps;
use crate::manager::{GitRoots, ManagerConfig, WindowManager, WindowSignal};

pub use observe::{ACTIVITY_EVERY, translate};

/// Decision 52: a retired window stays listed, `Exited`, this long.
pub const RETIRE_AFTER: Duration = Duration::from_secs(30);
/// Decision 52: a retired session still running this long after its stdin closed is
/// killed; decision 32's interrupt grace, in the driver's units.
pub const INTERRUPT_GRACE: Duration = Duration::from_secs(INTERRUPT_GRACE_SECS);
/// The done check's per-command git bound (decision 32), capped by the run's own.
pub const DONE_CHECK_GIT_TIMEOUT: Duration = Duration::from_secs(10);
/// Decision 43: a counter-only change is persisted at most this often.
pub const COUNTER_PERSIST_EVERY: Duration = Duration::from_secs(5);
/// M8a.16: a run's report is rewritten at most this often.
pub const REPORT_EVERY: Duration = Duration::from_millis(500);

/// What the service needs from the daemon.
pub struct RunContext {
    pub data_dir: PathBuf,
    pub worktrees_root: PathBuf,
    pub exe: PathBuf,
    pub socket_path: PathBuf,
    pub orchestrator: config::Orchestrator,
    pub git_roots: Arc<dyn GitRoots>,
    pub git: OsString,
    /// The manager's `cli_caps` (decision 53's project-settings check reads the same
    /// caps the sessions are launched with, test overrides included).
    pub cli_caps: CliCaps,
}

impl RunContext {
    /// The context of a daemon whose manager was built from `manager`.
    pub fn new(
        data_dir: PathBuf,
        manager: &ManagerConfig,
        orchestrator: config::Orchestrator,
        git_roots: Arc<dyn GitRoots>,
    ) -> Self {
        RunContext {
            data_dir,
            worktrees_root: manager.worktrees_root.clone(),
            exe: manager.exe.clone(),
            socket_path: manager.socket_path.clone(),
            orchestrator,
            git_roots,
            git: OsString::from("git"),
            cli_caps: manager.cli_caps,
        }
    }
}

enum Msg {
    Event(EventKind),
    Stop(oneshot::Sender<()>),
}

/// A window the engine retired (decision 52): its group is killed at `kill_at` if it
/// still runs, and the window removed at `remove_at`.
struct Retiring {
    kill_at: Instant,
    remove_at: Instant,
}

/// The event loop's bookkeeping.
struct Book {
    retiring: HashMap<u32, Retiring>,
    /// The process of each window the engine killed: its exit is `killed_by_engine`.
    killed: HashMap<u32, u32>,
    /// The roots registered with `GitRoots`, so a watch or unwatch is never doubled.
    watched: HashSet<PathBuf>,
    /// Runs with a counter-only change not yet persisted (decision 43).
    dirty: BTreeSet<String>,
    last_counter_save: Instant,
    reports_written: HashMap<String, Instant>,
    reports_due: BTreeSet<String>,
    publish_due: bool,
    /// Decision 48: the intents of each kind appended so far.
    intents: HashMap<&'static str, u32>,
}

/// Where an op's work goes: captured under the engine lock at the step that emitted it.
#[derive(Clone)]
pub(crate) struct OpCtx {
    pub run_id: String,
    pub project: PathBuf,
    pub data_dir: PathBuf,
    pub git_timeout: Duration,
    pub check_timeout: Duration,
}

/// Runs the engine. See the module doc.
pub struct RunService {
    manager: Arc<WindowManager>,
    ctx: RunContext,
    state: Mutex<EngineState>,
    tx: mpsc::UnboundedSender<Msg>,
    rx: Mutex<Option<mpsc::UnboundedReceiver<Msg>>>,
    signals: Mutex<Option<broadcast::Receiver<WindowSignal>>>,
    replies: Mutex<HashMap<ReplyId, oneshot::Sender<Result<String, String>>>>,
    next_reply: AtomicU64,
    snapshot_tx: watch::Sender<RunsSnapshot>,
    pushes: broadcast::Sender<Arc<RunsSnapshot>>,
    queue: Arc<GitQueue>,
    book: Mutex<Book>,
    /// Serializes appends to (and compactions of) every run's journal.
    journal: Arc<Mutex<()>>,
    /// Per run: every op holds it shared; `Accept` and `Discard` take it exclusively, so
    /// they run after every op of their run emitted before them (M8a.8's concern).
    op_order: Mutex<HashMap<String, Arc<RwLock<()>>>>,
    stopped: AtomicBool,
    /// How long `stop` waits for the loop to acknowledge (30 s; a test shortens it).
    stop_wait_ms: AtomicU64,
    /// Held by the event loop for its whole life, so `stop`'s fallback can wait until
    /// the loop is gone (ruling T22-minors, m8).
    loop_gate: Arc<tokio::sync::Mutex<()>>,
    loop_abort: Mutex<Option<tokio::task::AbortHandle>>,
    abort_after: Option<(String, u32)>,
}

/// Unix seconds, the reducer's clock.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Decision 48's `ANTHREX_TEST_ABORT_AFTER_INTENT=<op kind>[:<n>]`, debug builds only.
fn abort_after() -> Option<(String, u32)> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let value = std::env::var("ANTHREX_TEST_ABORT_AFTER_INTENT").ok()?;
    let (kind, n) = match value.split_once(':') {
        Some((kind, n)) => (kind.to_string(), n.parse().ok()?),
        None => (value, 1),
    };
    Some((kind, n))
}

impl RunService {
    /// A service for `manager` with the default `[orchestrator]` and its data under
    /// `data_dir`: what a daemon built without `lifecycle::run` (a test's) needs.
    pub fn for_manager(
        manager: &Arc<WindowManager>,
        data_dir: PathBuf,
        git_roots: Arc<dyn GitRoots>,
    ) -> Arc<Self> {
        let ctx = RunContext::new(
            data_dir,
            manager.config(),
            config::Orchestrator::default(),
            git_roots,
        );
        Self::new(manager.clone(), ctx)
    }

    pub fn new(manager: Arc<WindowManager>, ctx: RunContext) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        let signals = manager.signals();
        let (snapshot_tx, _) = watch::channel(RunsSnapshot {
            revision: 0,
            runs: Vec::new(),
        });
        let (pushes, _) = broadcast::channel(256);
        Arc::new(RunService {
            manager,
            ctx,
            state: Mutex::new(EngineState::default()),
            tx,
            rx: Mutex::new(Some(rx)),
            signals: Mutex::new(Some(signals)),
            replies: Mutex::new(HashMap::new()),
            next_reply: AtomicU64::new(1),
            snapshot_tx,
            pushes,
            queue: Arc::new(GitQueue::new()),
            book: Mutex::new(Book {
                retiring: HashMap::new(),
                killed: HashMap::new(),
                watched: HashSet::new(),
                dirty: BTreeSet::new(),
                last_counter_save: Instant::now(),
                reports_written: HashMap::new(),
                reports_due: BTreeSet::new(),
                publish_due: false,
                intents: HashMap::new(),
            }),
            journal: Arc::new(Mutex::new(())),
            op_order: Mutex::new(HashMap::new()),
            stopped: AtomicBool::new(false),
            stop_wait_ms: AtomicU64::new(30_000),
            loop_gate: Arc::new(tokio::sync::Mutex::new(())),
            loop_abort: Mutex::new(None),
            abort_after: abort_after(),
        })
    }

    /// Starts the event loop, the session-feed forwarder and the 1-second ticker.
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
        let signals = crate::lock(&self.signals).take();
        if let Some(signals) = signals {
            tokio::spawn(self.clone().forward(signals, shutdown.clone()));
        }
        let ticker = self.clone();
        let tick_token = shutdown.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = tick_token.cancelled() => break,
                    _ = interval.tick() => ticker.send(EventKind::Tick),
                }
            }
        });
        let rx = crate::lock(&self.rx).take();
        let service = self.clone();
        let gate = self.loop_gate.clone().try_lock_owned().ok();
        let handle = tokio::spawn(async move {
            let _gate = gate;
            let Some(mut rx) = rx else { return };
            while let Some(msg) = rx.recv().await {
                match msg {
                    Msg::Event(kind) => service.handle(kind).await,
                    Msg::Stop(ack) => {
                        service.stop_now().await;
                        let _ = ack.send(());
                        return;
                    }
                }
            }
        });
        *crate::lock(&self.loop_abort) = Some(handle.abort_handle());
        handle
    }

    /// Decision 46: after `stop` the service ignores every event, and the last
    /// `run.json` of each run is the one written here.
    pub async fn stop(&self) {
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }
        let (ack, done) = oneshot::channel();
        if self.tx.send(Msg::Stop(ack)).is_ok()
            && tokio::time::timeout(
                Duration::from_millis(self.stop_wait_ms.load(Ordering::SeqCst)),
                done,
            )
            .await
            .is_ok_and(|r| r.is_ok())
        {
            return;
        }
        // No acknowledgement: the loop is gone, never ran, or is stuck. It is aborted and
        // awaited first, so no `run.json` is written here beside a live loop (ruling
        // T22-minors, m8). A blocking write the loop had already handed to a thread
        // cannot be cancelled; it runs to its end.
        let abort = crate::lock(&self.loop_abort).take();
        if let Some(abort) = abort {
            abort.abort();
        }
        drop(self.loop_gate.lock().await);
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }
        self.stop_now().await;
    }

    async fn stop_now(&self) {
        let runs = {
            let mut state = crate::lock(&self.state);
            let (next, _) = step(
                std::mem::take(&mut *state),
                Event {
                    now: unix_now(),
                    kind: EventKind::Stop,
                },
            );
            *state = next;
            state.runs.values().cloned().collect::<Vec<_>>()
        };
        self.stopped.store(true, Ordering::SeqCst);
        for run in runs {
            effects::save(run).await;
        }
        let waiting: Vec<_> = crate::lock(&self.replies).drain().collect();
        for (_, reply) in waiting {
            let _ = reply.send(Err("the daemon is shutting down".to_string()));
        }
    }

    pub fn snapshots(&self) -> watch::Receiver<RunsSnapshot> {
        self.snapshot_tx.subscribe()
    }

    /// Every snapshot the service publishes from now on, one per structural change
    /// (decision 47), for `RunRequest::Subscribe`.
    pub fn pushes(&self) -> broadcast::Receiver<Arc<RunsSnapshot>> {
        self.pushes.subscribe()
    }

    /// The runs as they are now (decision 47's snapshot).
    pub fn current(&self) -> RunsSnapshot {
        let state = crate::lock(&self.state);
        snapshot(&state, unix_now())
    }

    fn send(&self, kind: EventKind) {
        let _ = self.tx.send(Msg::Event(kind));
    }

    /// Sends the event `make` builds with a fresh reply id, and waits for the engine's
    /// `Effect::Reply`.
    async fn ask(&self, make: impl FnOnce(ReplyId) -> EventKind) -> Result<String, String> {
        let id = self.next_reply.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        crate::lock(&self.replies).insert(id, tx);
        if self.stopped.load(Ordering::SeqCst) || self.tx.send(Msg::Event(make(id))).is_err() {
            crate::lock(&self.replies).remove(&id);
            return Err("the daemon is shutting down".to_string());
        }
        rx.await
            .unwrap_or_else(|_| Err("the daemon is shutting down".to_string()))
    }

    /// One event: a step under the engine lock, then its effects with the lock released.
    async fn handle(self: &Arc<Self>, kind: EventKind) {
        let now = unix_now();
        let tick = matches!(kind, EventKind::Tick);
        let kind = self.mark_killed(kind);
        let prepared = {
            let mut state = crate::lock(&self.state);
            let (next, fx) = step(std::mem::take(&mut *state), Event { now, kind });
            *state = next;
            effects::prepare(&state, fx, now)
        };
        self.execute(prepared, now).await;
        if tick {
            self.on_tick(now).await;
        }
    }

    /// Fills in `killed_by_engine` for the exit of a process the engine killed.
    fn mark_killed(&self, kind: EventKind) -> EventKind {
        match kind {
            EventKind::Signal {
                window_id,
                signal:
                    AgentSignal::ProcessExited {
                        code,
                        killed_by_engine,
                        pid,
                    },
            } => {
                // The record goes with the exit it marks (ruling T22-minors, m6).
                let killed = {
                    let mut book = crate::lock(&self.book);
                    let ours = book.killed.get(&window_id) == Some(&pid);
                    if ours {
                        book.killed.remove(&window_id);
                    }
                    killed_by_engine || ours
                };
                EventKind::Signal {
                    window_id,
                    signal: AgentSignal::ProcessExited {
                        code,
                        killed_by_engine: killed,
                        pid,
                    },
                }
            }
            other => other,
        }
    }

    /// The manager's session feed, translated (`observe.rs`) and sent in the order it
    /// arrives.
    async fn forward(
        self: Arc<Self>,
        mut signals: broadcast::Receiver<WindowSignal>,
        shutdown: CancellationToken,
    ) {
        let mut last_activity = HashMap::new();
        loop {
            let received = tokio::select! {
                _ = shutdown.cancelled() => return,
                received = signals.recv() => received,
            };
            match received {
                Ok(signal) => {
                    if let Some(agent) = translate(&signal, &mut last_activity, Instant::now()) {
                        self.send(EventKind::Signal {
                            window_id: signal.window_id,
                            signal: agent,
                        });
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "the run engine lagged on the session feed");
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    /// The tick's own work: retire deadlines, the coalesced snapshot, counter-only
    /// persists, due reports, and journal compaction (never between a `run.json` write
    /// and its intent lines: the tick runs between steps).
    async fn on_tick(self: &Arc<Self>, now: u64) {
        self.retire_deadlines();
        let publish = std::mem::take(&mut crate::lock(&self.book).publish_due);
        if publish {
            let snap = self.current();
            self.publish(snap);
        }
        let dirty = {
            let mut book = crate::lock(&self.book);
            if book.last_counter_save.elapsed() >= COUNTER_PERSIST_EVERY {
                book.last_counter_save = Instant::now();
                std::mem::take(&mut book.dirty)
            } else {
                BTreeSet::new()
            }
        };
        for run_id in dirty {
            if let Some(run) = self.run_clone(&run_id) {
                effects::save(run).await;
            }
        }
        self.write_due_reports(now).await;
        self.compact_journals().await;
    }

    fn run_clone(&self, run_id: &str) -> Option<super::model::Run> {
        crate::lock(&self.state).runs.get(run_id).cloned()
    }

    /// Kills retired sessions still running at their deadline and removes retired
    /// windows at theirs (decision 52).
    fn retire_deadlines(&self) {
        let now = Instant::now();
        let (kill, remove): (Vec<u32>, Vec<u32>) = {
            let mut book = crate::lock(&self.book);
            let kill = book
                .retiring
                .iter_mut()
                .filter(|(_, r)| r.kill_at <= now)
                .map(|(id, r)| {
                    r.kill_at = r.remove_at;
                    *id
                })
                .collect();
            let remove: Vec<u32> = book
                .retiring
                .iter()
                .filter(|(_, r)| r.remove_at <= now)
                .map(|(id, _)| *id)
                .collect();
            for id in &remove {
                book.retiring.remove(id);
            }
            (kill, remove)
        };
        for id in kill {
            if self.manager.child_pid(id).ok().flatten().is_some() {
                self.kill_window(id);
            }
        }
        for id in remove {
            self.forget_window(id);
            let _ = self.manager.remove(id);
        }
    }

    /// Drops every record of window `id` (ruling T22-minors, m6): it is being removed.
    fn forget_window(&self, id: u32) {
        let mut book = crate::lock(&self.book);
        book.retiring.remove(&id);
        book.killed.remove(&id);
    }

    /// Kills window `id`'s session, recording its process so the exit is the engine's.
    fn kill_window(&self, id: u32) -> Option<u32> {
        let pid = self.manager.headless_pid(id);
        if let Some(pid) = pid {
            crate::lock(&self.book).killed.insert(id, pid);
        }
        if let Err(error) = self.manager.headless_kill(id) {
            tracing::debug!(id, %error, "headless kill");
        }
        pid
    }

    fn publish(&self, snap: RunsSnapshot) {
        let _ = self.pushes.send(Arc::new(snap.clone()));
        self.snapshot_tx.send_replace(snap);
    }

    fn watch_root(&self, root: &Path) {
        let new = crate::lock(&self.book).watched.insert(root.to_path_buf());
        if new {
            self.ctx.git_roots.register(root.to_path_buf());
        }
    }

    fn unwatch_root(&self, root: &Path) {
        let watched = crate::lock(&self.book).watched.remove(root);
        if watched {
            self.ctx.git_roots.unregister(root);
        }
    }

    /// The per-run lock that orders `Accept` and `Discard` after the run's earlier ops.
    fn op_lock(&self, run_id: &str) -> Arc<RwLock<()>> {
        crate::lock(&self.op_order)
            .entry(run_id.to_string())
            .or_default()
            .clone()
    }
}

#[cfg(test)]
mod tests {
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
}
