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

pub mod parse;
pub mod probe;
pub mod schedule;
pub mod watch;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use proto::GitState;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::{JoinError, JoinHandle};
use tokio::time::Instant;

use crate::git::schedule::{Plan, Publisher, Scheduler};

/// How a root is probed. Injected so the registry's tests can drive its timing and its
/// publication rules without a repository, a `git` binary or a five second timeout.
pub type ProbeFn = Arc<dyn Fn(&Path) -> Option<GitState> + Send + Sync>;

/// Where publications go: the daemon forwards each one as a `DaemonMsg::Git`.
type Publish = UnboundedSender<(PathBuf, Option<GitState>)>;

/// Design decision 21 and the `ANTHREX_GIT` row of the brief's environment table:
/// `off` and `0` disable the subsystem entirely. Anything else — including an empty
/// value and an unset variable — leaves it on.
pub fn enabled_from_env() -> bool {
    !matches!(std::env::var("ANTHREX_GIT").as_deref(), Ok("off") | Ok("0"))
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
}

/// Watches and probes a set of worktree roots, publishing each root's git state when
/// it changes.
///
/// The manager holds no git state and takes no git-related lock: it calls
/// [`GitRegistry::register`] and [`GitRegistry::unregister`] as windows come and go
/// (design decision 16, never with the manager lock held) and reads nothing back.
pub struct GitRegistry {
    enabled: bool,
    probe: ProbeFn,
    channel: Arc<Channel>,
    roots: Mutex<HashMap<PathBuf, Slot>>,
}

impl GitRegistry {
    /// The production registry: the real `git` binary, the real probe.
    pub fn new(enabled: bool, publish: Publish) -> Self {
        Self::with_probe(
            enabled,
            publish,
            Arc::new(|root: &Path| probe::probe(OsStr::new("git"), root, probe::PROBE_TIMEOUT)),
        )
    }

    /// The same registry with the probe injected — the seam design decision 30 asks
    /// for, and the one the scheduling tests hang everything else off.
    pub fn with_probe(enabled: bool, publish: Publish, probe: ProbeFn) -> Self {
        Self {
            enabled,
            probe,
            channel: Arc::new(Channel {
                published: Mutex::new(HashMap::new()),
                publish,
            }),
            roots: Mutex::new(HashMap::new()),
        }
    }

    /// Starts watching and probing `root`, immediately (design decision 13). Registering
    /// a root that is already registered does nothing; `ANTHREX_GIT=off` makes this a
    /// no-op, so no watcher is built and no probe ever runs (design decision 21).
    ///
    /// Must be called from inside a tokio runtime: it spawns the root's task.
    pub fn register(&self, root: PathBuf) {
        if !self.enabled {
            return;
        }
        let mut roots = crate::lock(&self.roots);
        if roots.contains_key(&root) {
            return;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let handle = tokio::spawn(run_root(
            root.clone(),
            Arc::clone(&self.probe),
            Arc::clone(&self.channel),
            Arc::clone(&cancelled),
        ));
        roots.insert(root, Slot { handle, cancelled });
    }

    /// Stops watching and probing `root` and forgets its last published state. Safe
    /// while a probe is in flight: that probe's result is discarded rather than
    /// published (see the module docs).
    pub fn unregister(&self, root: &Path) {
        let slot = crate::lock(&self.roots).remove(root);
        if let Some(slot) = slot {
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
    Events,
    Deadline,
}

/// One root's whole life: build its watcher, then loop over "what does the scheduler
/// say to do now, and what happens next".
async fn run_root(
    root: PathBuf,
    probe: ProbeFn,
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
    let built = tokio::task::spawn_blocking(move || {
        // The watcher reports canonical paths, so the git dir the filter compares them
        // against has to be canonical too; on macOS a `/var/folders/...` root is
        // reported under `/private/var/folders/...`.
        let canonical = std::fs::canonicalize(&watch_root).unwrap_or(watch_root);
        let git_dir = probe::resolve_git_dir(&canonical)
            .and_then(|git_dir| std::fs::canonicalize(git_dir).ok());
        watch::build(&canonical, git_dir.as_deref(), sender)
    })
    .await;
    // Held, not used: dropping it unwatches the root. Design decision 15 — a watcher
    // that could not be built is logged once and the root stays on the safety poll.
    let _watcher = match built {
        Ok(Ok(watcher)) => Some(watcher),
        Ok(Err(error)) => {
            tracing::warn!(?root, %error, "git watcher unavailable; polling only");
            None
        }
        Err(error) => {
            tracing::warn!(?root, %error, "git watcher setup failed; polling only");
            None
        }
    };

    let mut scheduler = Scheduler::new(Instant::now());
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

        match wake(in_flight.as_mut(), &mut events, wake_at).await {
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

/// Waits for whichever comes first: the outstanding probe returning, a watcher event,
/// or the scheduler's next deadline.
///
/// This is its own function so that the borrow of `in_flight` ends before the caller
/// reassigns it.
async fn wake(
    in_flight: Option<&mut JoinHandle<Option<GitState>>>,
    events: &mut UnboundedReceiver<()>,
    wake_at: Instant,
) -> Wake {
    match in_flight {
        Some(handle) => tokio::select! {
            biased;
            result = handle => Wake::Probe(result),
            Some(()) = events.recv() => Wake::Events,
            () = tokio::time::sleep_until(wake_at) => Wake::Deadline,
        },
        None => tokio::select! {
            biased;
            Some(()) = events.recv() => Wake::Events,
            () = tokio::time::sleep_until(wake_at) => Wake::Deadline,
        },
    }
}
