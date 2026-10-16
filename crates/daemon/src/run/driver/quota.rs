//! Milestone 9.16 decisions 5, 9, 10 and 23 (task M9.16.7): the readings `forward` feeds
//! the engine, one [`Quota::observe`] per session signal, before it is translated.
//!
//! - A Claude `rate_limit_event` (`SessionEvent::Quota`) is sent as
//!   `EventKind::Quota { runtime: Claude, .. }` at once.
//! - A Claude `result` line's `modelUsage.<model>.contextWindow`
//!   (`SessionEvent::ContextWindow`) is kept here, keyed by the model as written, for the
//!   context handoff (decision 23 (4), [`Quota::learned_window`]).
//! - A headless Codex window's `Init` names its thread; on its `TurnEnded`, at most once
//!   per [`ROLLOUT_READ_EVERY`] daemon-wide, a task of its own finds the thread's rollout
//!   (`transcript::rollout::find_rollout`, cached per thread), reads its tail and parses its
//!   last `token_count` (`transcript::fill::codex_limits`) on `spawn_blocking`, within
//!   [`QUOTA_READ_WAIT`], and sends `EventKind::Quota { runtime: Codex, .. }`. `observe`
//!   never awaits it. A window whose read has not returned starts no other, so a hung
//!   filesystem holds one blocking thread per window, not one a minute. A rollout that is missing, unreadable, without limits or too slow is
//!   no reading (decision 8, fail open), logged at `debug`.
//!
//! **Locks.** This module's own mutexes are held for a lookup or an insert. The one call
//! under the manager's lock is the window's runtime, a copy-out, made once per window and
//! never while this module's own lock is held (AGENTS.md rules 2 and 3). No file is read
//! on a tokio worker.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proto::Runtime;
use proto::models::base_model_id;

use super::unix_now;
use crate::headless::SessionEvent;
use crate::manager::{WindowSignal, WindowSignalKind};
use crate::run::engine::EventKind;
use crate::transcript::fill::codex_limits;
use crate::transcript::rollout::{TRANSCRIPT_TAIL_BYTES, find_rollout, read_tail};

/// Decision 5: at most one rollout read this often, daemon-wide.
pub const ROLLOUT_READ_EVERY: Duration = Duration::from_secs(60);
/// Decision 5: how long one rollout read (the listing and the tail) may take.
pub const QUOTA_READ_WAIT: Duration = Duration::from_secs(5);
/// Past this many windows (or threads' rollouts) remembered, the record is cleared and
/// rebuilt from the next signals: a window's runtime is looked up again, and a Codex
/// window's thread is learned again from its next `Init` (every `codex exec` turn has one).
const KEPT: usize = 4096;

/// Where an engine event goes (the driver's queue).
pub(super) type Sink = Arc<dyn Fn(EventKind) + Send + Sync>;
/// A window's runtime, from the manager (`None`: not a headless window).
pub(super) type RuntimeOf = Arc<dyn Fn(u32) -> Option<Runtime> + Send + Sync>;
/// One rollout read, blocking (`spawn_blocking`): given the sessions folder, the
/// thread's id and start, and its cached path, the path read and its tail; `None`
/// when no rollout was found.
pub(super) type ReadFn = Arc<
    dyn Fn(&Path, &str, u64, Option<PathBuf>) -> Option<(PathBuf, std::io::Result<String>)>
        + Send
        + Sync,
>;

/// The production [`ReadFn`]: `find_rollout` (unless cached), then `read_tail`.
fn read_rollout(
    sessions: &Path,
    id: &str,
    started_at: u64,
    cached: Option<PathBuf>,
) -> Option<(PathBuf, std::io::Result<String>)> {
    let path = cached.or_else(|| find_rollout(sessions, id, started_at))?;
    let tail = read_tail(&path, TRANSCRIPT_TAIL_BYTES);
    Some((path, tail))
}

/// See the module doc.
pub(super) struct Quota {
    sink: Sink,
    runtime_of: RuntimeOf,
    /// `$CODEX_HOME/sessions` (else `~/.codex/sessions`) of the daemon's environment,
    /// resolved once; `None` reads no rollout.
    sessions: Option<PathBuf>,
    read_wait: Duration,
    read: ReadFn,
    /// Decision 23 (4): every context window a headless Claude session reported.
    windows: Mutex<BTreeMap<String, u64>>,
    feed: Arc<Mutex<Feed>>,
}

#[derive(Default)]
struct Feed {
    runtimes: HashMap<u32, Option<Runtime>>,
    /// Each headless Codex window's thread, and when it was first seen (Unix seconds,
    /// the start decision 9 searches around).
    threads: HashMap<u32, Thread>,
    /// Decision 9: each thread's rollout, once found.
    rollouts: HashMap<String, PathBuf>,
    last_read: Option<Instant>,
    /// Task 7's review M1: the windows whose read has not returned yet (it is cleared
    /// by the blocking read itself, so a read that never returns keeps its window out).
    in_flight: HashSet<u32>,
}

#[derive(Clone)]
struct Thread {
    id: String,
    started_at: u64,
}

impl Quota {
    pub(super) fn new(sink: Sink, runtime_of: RuntimeOf, sessions: Option<PathBuf>) -> Self {
        Quota {
            sink,
            runtime_of,
            sessions,
            read_wait: QUOTA_READ_WAIT,
            read: Arc::new(read_rollout),
            windows: Mutex::default(),
            feed: Arc::default(),
        }
    }

    /// This feed with a read wait other than [`QUOTA_READ_WAIT`] (a test seam).
    #[cfg(test)]
    pub(super) fn with_read_wait(mut self, wait: Duration) -> Self {
        self.read_wait = wait;
        self
    }

    /// This feed with `read` in place of the rollout read (a test seam).
    #[cfg(test)]
    pub(super) fn with_reader(mut self, read: ReadFn) -> Self {
        self.read = read;
        self
    }

    /// One signal of the session feed, now.
    pub(super) fn observe(&self, signal: &WindowSignal) {
        let _ = self.observe_at(signal, Instant::now(), unix_now());
    }

    /// [`observe`](Self::observe) at `now` (the read throttle's clock) and `unix` (a
    /// thread's start). Returns the rollout read's task when one was started.
    pub(super) fn observe_at(
        &self,
        signal: &WindowSignal,
        now: Instant,
        unix: u64,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let WindowSignalKind::Session(event) = &signal.kind else {
            return None;
        };
        let window = signal.window_id;
        match event {
            // Only Claude's stream carries a `rate_limit_event` (`codex exec --json` has
            // no rate limits, decision 5).
            SessionEvent::Quota(sample) => (self.sink)(EventKind::Quota {
                runtime: Runtime::Claude,
                sample: sample.clone(),
            }),
            SessionEvent::ContextWindow { model, tokens } => {
                crate::lock(&self.windows).insert(model.clone(), *tokens);
            }
            SessionEvent::Init { session_id, .. }
                if self.runtime(window) == Some(Runtime::Codex) =>
            {
                let mut feed = crate::lock(&self.feed);
                let known = feed
                    .threads
                    .get(&window)
                    .is_some_and(|t| t.id == *session_id);
                if !known {
                    let thread = Thread {
                        id: session_id.clone(),
                        started_at: unix,
                    };
                    feed.threads.insert(window, thread);
                }
            }
            SessionEvent::TurnEnded { .. } => return self.turn_ended(window, now),
            _ => {}
        }
        None
    }

    /// Window `window`'s runtime, looked up once (a copy-out under the manager's lock,
    /// made with this module's lock released).
    fn runtime(&self, window: u32) -> Option<Runtime> {
        if let Some(known) = crate::lock(&self.feed).runtimes.get(&window) {
            return *known;
        }
        let runtime = (self.runtime_of)(window);
        let mut feed = crate::lock(&self.feed);
        if feed.runtimes.len() >= KEPT {
            feed.runtimes.clear();
            feed.threads.clear();
        }
        feed.runtimes.insert(window, runtime);
        runtime
    }

    /// Decision 5: a Codex window's turn ended; its rollout is read when the last read
    /// is [`ROLLOUT_READ_EVERY`] old.
    fn turn_ended(&self, window: u32, now: Instant) -> Option<tokio::task::JoinHandle<()>> {
        let sessions = self.sessions.clone()?;
        if self.runtime(window) != Some(Runtime::Codex) {
            return None;
        }
        let (thread, cached) = {
            let mut feed = crate::lock(&self.feed);
            let thread = feed.threads.get(&window)?.clone();
            let recent = (feed.last_read)
                .is_some_and(|at| now.saturating_duration_since(at) < ROLLOUT_READ_EVERY);
            if recent || feed.in_flight.contains(&window) {
                return None;
            }
            feed.in_flight.insert(window);
            feed.last_read = Some(now);
            let cached = feed.rollouts.get(&thread.id).cloned();
            (thread, cached)
        };
        let (sink, feed, wait) = (self.sink.clone(), self.feed.clone(), self.read_wait);
        let reader = self.read.clone();
        Some(tokio::spawn(async move {
            let id = thread.id.clone();
            let flight = feed.clone();
            let read = tokio::task::spawn_blocking(move || {
                let read = reader(&sessions, &thread.id, thread.started_at, cached);
                crate::lock(&flight).in_flight.remove(&window);
                read
            });
            match tokio::time::timeout(wait, read).await {
                Ok(Ok(Some((path, Ok(tail))))) => {
                    let mut known = crate::lock(&feed);
                    if known.rollouts.len() >= KEPT {
                        known.rollouts.clear();
                    }
                    known.rollouts.insert(id.clone(), path);
                    drop(known);
                    match codex_limits(&tail) {
                        Some(sample) => sink(EventKind::Quota {
                            runtime: Runtime::Codex,
                            sample,
                        }),
                        None => {
                            tracing::debug!(thread = %id, "the Codex rollout has no rate limits yet")
                        }
                    }
                }
                Ok(Ok(Some((path, Err(error))))) => {
                    crate::lock(&feed).rollouts.remove(&id);
                    let path = path.display();
                    tracing::debug!(thread = %id, %path, %error, "the Codex rollout could not be read; quota unmeasured");
                }
                Ok(Ok(None)) => {
                    tracing::debug!(thread = %id, "no Codex rollout found; quota unmeasured");
                }
                Ok(Err(error)) => {
                    tracing::debug!(thread = %id, %error, "the Codex rollout read failed; quota unmeasured");
                }
                Err(_) => {
                    let ms = wait.as_millis();
                    tracing::debug!(thread = %id, ms, "the Codex rollout read did not finish in time; quota unmeasured");
                }
            }
        }))
    }

    /// Decision 23 (4): the context window a headless Claude session reported for the
    /// transcript line's `model` (as written), matched exactly; else by canonical id
    /// (`proto::models::base_model_id`) among the entries whose context tag is the route's
    /// own (`[1m]` or none), when the route carries the tag or exactly one entry matches.
    /// A transcript line never carries `[1m]` (M9.16.1 finding 2), so a plain route never
    /// takes a tagged session's window, and an ambiguous plain match is no window.
    // M9.16.9 (the context handoff) reads it; until then only its tests do.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn learned_window(&self, model: &str, route_model: &str) -> Option<u64> {
        let windows = crate::lock(&self.windows);
        if let Some(window) = windows.get(model) {
            return Some(*window);
        }
        let canonical = base_model_id(model);
        let tag = context_tag(route_model);
        let mut matches = (windows.iter())
            .filter(|(key, _)| base_model_id(key) == canonical && context_tag(key) == tag)
            .map(|(_, window)| *window);
        let first = matches.next()?;
        match (tag.is_some(), matches.next()) {
            (_, None) => Some(first),
            // The route's tag names the window size; every entry with it agrees.
            (true, Some(second)) => Some(first.max(second).max(matches.max().unwrap_or(0))),
            (false, Some(_)) => None,
        }
    }

    /// Every learned context window, by model (tests).
    #[cfg(test)]
    pub(super) fn windows(&self) -> Vec<(String, u64)> {
        let windows = crate::lock(&self.windows);
        windows.iter().map(|(k, v)| (k.clone(), *v)).collect()
    }
}

/// The run service's feed: events go to its queue, a window's runtime comes from
/// `manager` (its headless spec, copied out), and rollouts from `sessions`.
pub(super) fn for_service(
    manager: &Arc<crate::manager::WindowManager>,
    sessions: Option<PathBuf>,
    tx: &tokio::sync::mpsc::UnboundedSender<super::Msg>,
) -> Quota {
    let (tx, manager) = (tx.clone(), manager.clone());
    let sink: Sink = Arc::new(move |kind| {
        let _ = tx.send(super::Msg::Event(kind));
    });
    let runtime_of: RuntimeOf = Arc::new(move |id| manager.headless_spec(id).map(|s| s.runtime));
    Quota::new(sink, runtime_of, sessions)
}

/// Decision 10: the engine's `EventKind::BrakesConfig` for `orchestrator`'s
/// `[orchestrator.brakes]`, sent after every config load and settings save.
pub(super) fn brakes_config(orchestrator: &config::Orchestrator) -> EventKind {
    EventKind::BrakesConfig(orchestrator.brakes.into())
}

/// Decision 10 and ruling C3: the engine's first state, the loaded config's brakes
/// stepped into it before anything else (the restore's step included), so U11's
/// default-on brake holds from the first dispatch.
pub(super) fn configured_state(
    orchestrator: &config::Orchestrator,
) -> crate::run::engine::EngineState {
    let event = crate::run::engine::Event {
        now: unix_now(),
        kind: brakes_config(orchestrator),
    };
    // A fresh state's only effect could be a publish, with no subscriber yet.
    let (state, _) = crate::run::engine::step(Default::default(), event);
    state
}

/// A model id's trailing context tag (`[1m]`), if it has one.
fn context_tag(id: &str) -> Option<&str> {
    let at = id.strip_suffix(']').and_then(|s| s.rfind('['))?;
    (at > 0).then(|| &id[at..])
}

#[cfg(test)]
#[path = "quota_tests.rs"]
mod tests;
