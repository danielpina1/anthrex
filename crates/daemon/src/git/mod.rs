//! Per-worktree git state: detection, probing, filesystem watching, publication.
//!
//! [`parse`] is the pure `porcelain=v2 -z` parser (M4.5.3), [`probe`] is the hardened
//! `git` invocation that feeds it (M4.5.4), [`schedule`] is the pure state machine that
//! decides *when* to run one, and [`watch`] is the `notify` watcher and its path filter.
//! [`GitRegistry`] is what ties them together: one tokio task per registered worktree
//! root, each owning its own watcher, its own scheduler and its own probe.
//!
//! Three properties are structural here rather than incidental, because each is a bug
//! that would only show up in production.
//!
//! * **A probe never runs on the async runtime.** The only call site is inside
//!   [`tokio::task::spawn_blocking`]; the probe spawns a child process and drains its
//!   stdout with a five second deadline (AGENTS.md hard rule 2).
//! * **Two probes for one root never overlap.** One task owns a root, it holds at most
//!   one [`tokio::task::JoinHandle`], and [`schedule::Scheduler`] will not hand out a
//!   second probe until [`schedule::Scheduler::probe_finished`] is called.
//! * **Unregistering while a probe is in flight publishes nothing afterwards.**
//!   [`GitRegistry::unregister`] sets the root's cancelled flag *under the same mutex*
//!   that every publication takes, so a publication either lands entirely before the
//!   unregister (and is then removed with the root) or sees the flag and is dropped.
//! * **A watcher is never destroyed on a runtime thread.** `notify`'s watchers join a
//!   thread in their `Drop`, and the watcher lives inside this task's future, so an
//!   `abort()` would otherwise run that blocking destructor on a worker. It is held as
//!   a [`watch::WatchGuard`], which disposes of it on a blocking thread instead.

pub mod parse;
pub mod probe;
pub mod schedule;
pub mod watch;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proto::GitState;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::{JoinError, JoinHandle};
use tokio::time::Instant;

use crate::git::schedule::{Plan, Publisher, Scheduler};

/// How a root is probed. Injected so the registry's tests can drive its timing and its
/// publication rules without a repository, a `git` binary or a five second timeout.
pub type ProbeFn = Arc<dyn Fn(&Path) -> Option<GitState> + Send + Sync>;

/// How a root's watcher is built and armed: `(root, git.ignore, event sink)`. Blocking,
/// so it only ever runs on [`tokio::task::spawn_blocking`]. Injected for the same reason
/// as [`ProbeFn`]: arming a real watcher can take seconds (see [`run_root`]), and a test
/// can only make that happen on demand by standing in for it.
pub type ArmFn = Arc<
    dyn Fn(&Path, &[String], UnboundedSender<()>) -> notify::Result<watch::WatchGuard>
        + Send
        + Sync,
>;

/// Where publications go: the daemon forwards each one as a `DaemonMsg::Git`.
type Publish = UnboundedSender<(PathBuf, Option<GitState>)>;

/// Design decision 21 and the `ANTHREX_GIT` row of the brief's environment table:
/// `off` and `0` disable the subsystem entirely. Anything else — including an empty
/// value and an unset variable — leaves it on.
pub fn enabled_from_env() -> bool {
    !matches!(std::env::var("ANTHREX_GIT").as_deref(), Ok("off") | Ok("0"))
}

/// The `[git]` settings the daemon actually runs on: the file's table, with `enabled`
/// resolved against `ANTHREX_GIT`.
///
/// The environment can only turn git *off*. `ANTHREX_GIT=off` is the escape hatch
/// git-surface spec 3.7 gives, and the smoke script and CI depend on it winning; a
/// config file able to switch git back on against it would not be an escape hatch. The
/// file's own power is the other direction — `git.enabled = false` with the variable
/// unset turns git off.
pub fn settings_from_env(settings: config::Git) -> config::Git {
    settings_with(enabled_from_env(), settings)
}

/// The pure half of [`settings_from_env`], separated so the rule can be tested without
/// writing the process environment — `crates/daemon/tests/git_env.rs` must stay a
/// single test in a binary of its own (see its module docs).
pub fn settings_with(env_enabled: bool, settings: config::Git) -> config::Git {
    config::Git {
        enabled: env_enabled && settings.enabled,
        ..settings
    }
}

/// The last published state of every registered root, and the sink they go to.
///
/// The mutex covers the map *and* the send, so publications for one root reach the
/// channel in the order the map records them, and so [`GitRegistry::unregister`] has
/// something to serialise against (see the module docs).
struct Channel {
    published: Mutex<HashMap<PathBuf, Option<GitState>>>,
    publish: Publish,
}

impl Channel {
    fn emit(&self, cancelled: &AtomicBool, root: &Path, state: Option<GitState>) {
        let mut published = crate::lock(&self.published);
        if cancelled.load(Ordering::SeqCst) {
            return;
        }
        published.insert(root.to_path_buf(), state.clone());
        // An unbounded send never blocks, so holding the mutex across it is not a
        // blocking-under-a-lock violation; it is what keeps the two in step.
        let _ = self.publish.send((root.to_path_buf(), state));
    }

    fn forget(&self, cancelled: &AtomicBool, root: &Path) {
        let mut published = crate::lock(&self.published);
        cancelled.store(true, Ordering::SeqCst);
        published.remove(root);
    }
}

struct Slot {
    handle: JoinHandle<()>,
    cancelled: Arc<AtomicBool>,
    /// How many windows currently reference this root. The task starts on the 0 → 1
    /// transition and tears down on the 1 → 0 transition; every call in between just
    /// moves this count, which is what makes `register` and `unregister` commute
    /// regardless of the order a racing create and remove happen to land in — the
    /// reason this lives here rather than being derived from `WindowManager::list()`
    /// (see the module docs' race note).
    refs: usize,
}

/// Watches and probes a set of worktree roots, publishing each root's git state when
/// it changes.
///
/// The manager holds no git state and takes no git-related lock: it calls
/// [`GitRegistry::register`] and [`GitRegistry::unregister`] as windows come and go
/// (design decision 16, never with the manager lock held) and reads nothing back.
/// Reference counting lives here, under `roots`' own mutex, precisely so that two
/// calls racing from different connections — one window's `create` registering a root
/// just as another window on the same root is removed — always commute to the same
/// end state no matter which lands first, instead of depending on a snapshot of the
/// window table taken by the caller.
pub struct GitRegistry {
    settings: config::Git,
    probe: ProbeFn,
    arm: ArmFn,
    channel: Arc<Channel>,
    roots: Mutex<HashMap<PathBuf, Slot>>,
}

impl GitRegistry {
    /// The production registry: the real `git` binary, the real probe.
    ///
    /// `settings` is `config.toml`'s `[git]` table as [`settings_from_env`] resolved it
    /// — `enabled` decides whether anything runs at all, and `poll_secs`,
    /// `debounce_ms` and `ignore` reach each root's scheduler and watcher.
    pub fn new(settings: config::Git, publish: Publish) -> Self {
        Self::with_probe(
            settings,
            publish,
            Arc::new(|root: &Path| probe::probe(OsStr::new("git"), root, probe::PROBE_TIMEOUT)),
        )
    }

    /// The same registry with the probe injected — the seam design decision 30 asks
    /// for, and the one the scheduling tests hang everything else off.
    pub fn with_probe(settings: config::Git, publish: Publish, probe: ProbeFn) -> Self {
        Self::with_seams(
            settings,
            publish,
            probe,
            Arc::new(|root: &Path, ignore: &[String], events| watch::arm(root, ignore, events)),
        )
    }

    /// [`Self::with_probe`] with the watcher builder injected as well.
    pub fn with_seams(settings: config::Git, publish: Publish, probe: ProbeFn, arm: ArmFn) -> Self {
        Self {
            settings,
            probe,
            arm,
            channel: Arc::new(Channel {
                published: Mutex::new(HashMap::new()),
                publish,
            }),
            roots: Mutex::new(HashMap::new()),
        }
    }

    /// Adds one reference to `root`, starting its watcher and probing it immediately
    /// (design decision 13) only if this is the first reference. A second and later
    /// `register` for the same root just counts up — the root keeps running the task
    /// it already had. `ANTHREX_GIT=off` makes every call a no-op, so no watcher is
    /// ever built and no probe ever runs (design decision 21).
    ///
    /// Must be called from inside a tokio runtime: the first reference spawns the
    /// root's task.
    pub fn register(&self, root: PathBuf) {
        if !self.settings.enabled {
            return;
        }
        let mut roots = crate::lock(&self.roots);
        if let Some(slot) = roots.get_mut(&root) {
            slot.refs += 1;
            return;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let handle = tokio::spawn(run_root(
            root.clone(),
            self.settings.clone(),
            Arc::clone(&self.probe),
            Arc::clone(&self.arm),
            Arc::clone(&self.channel),
            Arc::clone(&cancelled),
        ));
        roots.insert(
            root,
            Slot {
                handle,
                cancelled,
                refs: 1,
            },
        );
    }

    /// Removes one reference from `root`. Stops watching and probing it, and forgets
    /// its last published state, only once this was the last reference. Safe while a
    /// probe is in flight: that probe's result is discarded rather than published (see
    /// the module docs). Unregistering a root with no reference to remove — including
    /// one that was never registered, e.g. because `ANTHREX_GIT=off` — is a no-op.
    pub fn unregister(&self, root: &Path) {
        let released = {
            let mut roots = crate::lock(&self.roots);
            match roots.get_mut(root) {
                Some(slot) if slot.refs > 1 => {
                    slot.refs -= 1;
                    None
                }
                Some(_) => roots.remove(root),
                None => None,
            }
        };
        if let Some(slot) = released {
            self.channel.forget(&slot.cancelled, root);
            slot.handle.abort();
        }
    }

    /// Every root that has published something, with what it published — what design
    /// decision 17 sends to a client straight after its `Welcome`.
    ///
    /// A root that has been registered but whose first probe has not returned yet is
    /// absent rather than present as `None`: on the wire `None` means "this is not a
    /// repository", and "not yet known" is not that.
    pub fn snapshot(&self) -> Vec<(PathBuf, Option<GitState>)> {
        crate::lock(&self.channel.published)
            .iter()
            .map(|(root, state)| (root.clone(), state.clone()))
            .collect()
    }
}

/// Design decision 22: what `WindowManager::remove_with_worktree` is allowed to know
/// about this registry.
///
/// The manager has to unregister a root *between* killing the agent and deleting its
/// directory, which only it is in a position to do, so it needs these two calls — and
/// through this trait it gets exactly these two and nothing else: no [`GitState`], no
/// `GitRegistry`, no knowledge that a watcher or a probe exists.
///
/// Both are the inherent methods below, unchanged. In particular the reference counting
/// stays here (design decision 23): the manager calls `unregister` once per removed
/// window and this decides whether anything stops.
impl crate::manager::GitRoots for GitRegistry {
    fn register(&self, root: PathBuf) {
        GitRegistry::register(self, root);
    }

    fn unregister(&self, root: &Path) {
        GitRegistry::unregister(self, root);
    }
}

impl Drop for GitRegistry {
    /// Dropping the registry stops every root — otherwise the tasks, and the watcher
    /// threads they own, would outlive the daemon that made them.
    fn drop(&mut self) {
        for (_, slot) in crate::lock(&self.roots).drain() {
            slot.cancelled.store(true, Ordering::SeqCst);
            slot.handle.abort();
        }
    }
}

/// Why the root's task woke up.
enum Wake {
    Probe(Result<Option<GitState>, JoinError>),
    Armed(Result<notify::Result<watch::WatchGuard>, JoinError>),
    Events,
    Deadline,
}

/// One root's whole life: a loop over "what does the scheduler say to do now, and what
/// happens next", with its watcher being armed alongside.
///
/// **The first probe does not wait for the watcher.** Arming one is not bounded by
/// anything here: on macOS it starts an FSEvents stream, and the first time a freshly
/// built executable does that it has been measured taking 1.5 to 7.8 seconds, for every
/// root in the process at once, while `git status` itself took about 25 ms. Probing
/// only once the watcher was armed held design decision 13's "probe immediately on
/// registration" hostage to that, and left every root blank for as long as it lasted.
///
/// **A root is probed again once its watcher is armed.** The first probe can read the
/// worktree before the watcher exists, and a change that lands in between produces no
/// event. Without that extra probe it would stay unpublished until the safety poll. It
/// costs one `git status` per registration, and the [`Publisher`] republishes nothing
/// if the state has not changed.
async fn run_root(
    root: PathBuf,
    settings: config::Git,
    probe: ProbeFn,
    arm: ArmFn,
    channel: Arc<Channel>,
    cancelled: Arc<AtomicBool>,
) {
    let (sender, mut events) = mpsc::unbounded_channel();
    // Keeping the original sender alive means `events.recv()` never resolves to `None`
    // even when the watcher could not be built, so the select! below has one fewer
    // state to be wrong about.
    let _keep_open = sender.clone();

    // `watch::build` walks the tree on the inotify backend, so it is blocking work.
    let watch_root = root.clone();
    let watch_ignore = settings.ignore.clone();
    // If this task is aborted while arming is still running, the handle is dropped and
    // the guard `arm` returns is dropped by tokio on the blocking thread that built it,
    // so it still never reaches a runtime worker.
    let mut arming = Some(tokio::task::spawn_blocking(move || {
        arm(&watch_root, &watch_ignore, sender)
    }));
    // Held, not used: dropping the guard unwatches the root, on a blocking thread
    // rather than on the worker that drops this future (see `watch::WatchGuard`).
    let mut _watcher: Option<watch::WatchGuard> = None;

    let mut scheduler = Scheduler::new(
        Instant::now(),
        Duration::from_secs(settings.poll_secs),
        Duration::from_millis(settings.debounce_ms),
    );
    let mut publisher = Publisher::default();
    let mut in_flight: Option<JoinHandle<Option<GitState>>> = None;

    loop {
        if cancelled.load(Ordering::SeqCst) {
            return;
        }
        let Plan {
            probe: start,
            wake_at,
        } = scheduler.plan(Instant::now());
        if start {
            let probe = Arc::clone(&probe);
            let root = root.clone();
            in_flight = Some(tokio::task::spawn_blocking(move || probe(&root)));
        }

        match wake(in_flight.as_mut(), arming.as_mut(), &mut events, wake_at).await {
            Wake::Probe(result) => {
                in_flight = None;
                scheduler.probe_finished();
                let state = result.unwrap_or_else(|error| {
                    tracing::warn!(?root, %error, "git probe task failed");
                    None
                });
                if let Some(publication) = publisher.decide(state) {
                    channel.emit(&cancelled, &root, publication);
                }
            }
            Wake::Armed(built) => {
                arming = None;
                // Design decision 15: a watcher that could not be built is logged once
                // and the root stays on the safety poll.
                match built {
                    Ok(Ok(watcher)) => {
                        _watcher = Some(watcher);
                        scheduler.request_probe();
                    }
                    Ok(Err(error)) => {
                        tracing::warn!(?root, %error, "git watcher unavailable; polling only");
                    }
                    Err(error) => {
                        tracing::warn!(?root, %error, "git watcher setup failed; polling only");
                    }
                }
            }
            Wake::Events => {
                let now = Instant::now();
                let mut tripped = scheduler.record_event(now);
                // Everything already queued belongs to the same burst; counting it all
                // now is what lets the breaker see a storm as a storm.
                while events.try_recv().is_ok() {
                    tripped |= scheduler.record_event(now);
                }
                if tripped {
                    tracing::warn!(
                        ?root,
                        "git watcher event storm; this root is poll-only for 60s"
                    );
                }
            }
            Wake::Deadline => {}
        }
    }
}

/// Waits for whichever comes first: the outstanding probe returning, the watcher
/// finishing arming, a watcher event, or the scheduler's next deadline.
///
/// This is its own function so that the borrows of `in_flight` and `arming` end before
/// the caller reassigns them.
async fn wake(
    in_flight: Option<&mut JoinHandle<Option<GitState>>>,
    arming: Option<&mut JoinHandle<notify::Result<watch::WatchGuard>>>,
    events: &mut UnboundedReceiver<()>,
    wake_at: Instant,
) -> Wake {
    tokio::select! {
        biased;
        result = joined(in_flight) => Wake::Probe(result),
        built = joined(arming) => Wake::Armed(built),
        Some(()) = events.recv() => Wake::Events,
        () = tokio::time::sleep_until(wake_at) => Wake::Deadline,
    }
}

/// The handle's result, or never when there is no handle. A handle is only ever passed
/// in until it has resolved once: the caller drops it on the result.
async fn joined<T>(handle: Option<&mut JoinHandle<T>>) -> Result<T, JoinError> {
    match handle {
        Some(handle) => handle.await,
        None => std::future::pending().await,
    }
}
