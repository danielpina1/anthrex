//! Milestone 9.16 decisions 22–29 (U13): the context handoff's measurements.
//!
//! - At a goal boundary ([`RunService::measure_handoff`], from `chain_goal.rs`'s
//!   `continued_run`, only when the chain's window would be adopted): the window's
//!   session files are copied out of the manager (`session_files`, a lookup), the tail
//!   of its transcript (Claude) or rollout (Codex, found by its thread id) is read and
//!   parsed on `spawn_blocking` within the caller's bound, the window resolved
//!   ([`window_for`], decision 23), and [`handoff_due`] decides.
//! - Mid-run ([`RunService::check_live`], from `on_tick`, every [`FILL_CHECK_EVERY`]):
//!   each `Running` run's live orchestrator is measured the same way, on a task of its
//!   own, and `EventKind::OrchestratorFill` carries the result to the engine, which only
//!   shows it (decision 29). One check is in flight at a time.
//!
//! **Locks.** The engine lock is taken to copy the live orchestrators out, the manager's
//! for each window's session files; no file is read under either, nor on a tokio worker
//! (AGENTS.md rules 2 and 10).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use proto::{RunState, Runtime};

use super::{RunService, unix_now};
use crate::manager::SessionFiles;
use crate::run::engine::EventKind;
use crate::run::model::HandoffRecord;
use crate::run::orch::contract_rounds::{DEFAULT_CONTEXT_WINDOW, ONE_M_CONTEXT_WINDOW};
use crate::transcript::fill::{Fill, claude_fill, codex_fill, handoff_due};
use crate::transcript::rollout::{TRANSCRIPT_TAIL_BYTES, find_rollout, read_tail};

/// Decision 29: how often the live orchestrators are measured.
pub const FILL_CHECK_EVERY: Duration = Duration::from_secs(60);

/// The `[1m]` tag a route's model carries for a 1M context window (decision 23 (3)).
const ONE_M_TAG: &str = "[1m]";

/// One fill read, blocking (`spawn_blocking`): a window's session files and the Codex
/// sessions folder; `None` when nothing was measured.
pub(super) type FillRead = Arc<dyn Fn(&SessionFiles, Option<&Path>) -> Option<Fill> + Send + Sync>;

/// The service's handoff bookkeeping.
pub(super) struct FillCheck {
    read: std::sync::Mutex<FillRead>,
    /// When `check_live` last started a check.
    last: std::sync::Mutex<Option<Instant>>,
    /// A check is in flight (cleared by the check's task when it ends).
    busy: Arc<AtomicBool>,
}

impl Default for FillCheck {
    fn default() -> Self {
        FillCheck {
            read: std::sync::Mutex::new(Arc::new(read_fill)),
            last: std::sync::Mutex::new(None),
            busy: Arc::default(),
        }
    }
}

impl FillCheck {
    /// Tests only: `read` in place of the transcript and rollout read.
    #[cfg(test)]
    pub(super) fn set_reader(&self, read: FillRead) {
        *crate::lock(&self.read) = read;
    }

    /// Tests only: the next `check_live` is due at once (`on_tick` may have run one).
    #[cfg(test)]
    pub(super) fn rearm(&self) {
        *crate::lock(&self.last) = None;
    }
}

/// The production [`FillRead`]: Claude's transcript tail, or Codex's rollout of the
/// session's thread (decision 26), parsed by `claude_fill` or `codex_fill`. Only a
/// regular file is read (`read_tail`).
fn read_fill(files: &SessionFiles, sessions: Option<&Path>) -> Option<Fill> {
    let (path, parse): (PathBuf, fn(&str) -> Option<Fill>) = match files.runtime {
        Runtime::Claude => (files.transcript_path.clone()?, claude_fill),
        Runtime::Codex => {
            let id = files.session_id.as_deref()?;
            (find_rollout(sessions?, id, files.started_at)?, codex_fill)
        }
        _ => return None,
    };
    let tail = read_tail(&path, TRANSCRIPT_TAIL_BYTES).ok()?;
    parse(&tail)
}

/// Decision 23: the context window of a session of `runtime` on a route whose model is
/// `route_model`, the first that applies: the config's `context_window_tokens` when it
/// is not 0; Codex: the rollout's own; Claude: the route's `[1m]` tag, then a window
/// `learned` from a headless session (`Quota::learned_window(line model, route model)`),
/// then [`DEFAULT_CONTEXT_WINDOW`].
pub fn window_for(
    runtime: Runtime,
    configured: u64,
    route_model: &str,
    fill: &Fill,
    learned: impl Fn(&str, &str) -> Option<u64>,
) -> Option<u64> {
    if configured > 0 {
        return Some(configured);
    }
    match runtime {
        Runtime::Codex => fill.window,
        Runtime::Claude if route_model.ends_with(ONE_M_TAG) => Some(ONE_M_CONTEXT_WINDOW),
        Runtime::Claude => Some(
            (fill.model.as_deref())
                .and_then(|model| learned(model, route_model))
                .unwrap_or(DEFAULT_CONTEXT_WINDOW),
        ),
        _ => None,
    }
}

/// What a goal boundary's measurement found (decisions 24 and 28).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Measured {
    /// Both criteria are 0: nothing is measured.
    Off,
    /// Nothing measured, or (with the token cap off) no window: adopt and say so.
    Unknown,
    /// Measured and below the threshold: adopt.
    Below,
    /// At or past the threshold: a fresh session.
    Due(HandoffRecord),
}

impl RunService {
    /// The fill of window `window_id`'s session, read off every lock within `bound`,
    /// with its window resolved (decision 23); the route's model is `route_model`.
    async fn fill_of(&self, window_id: u32, route_model: &str, bound: Duration) -> Option<Fill> {
        let bound = self.ctx.fill_read_cap.map_or(bound, |cap| cap.min(bound));
        let files = self.manager.session_files(window_id)?;
        let (read, sessions, copy) = (
            crate::lock(&self.handoff.read).clone(),
            self.ctx.codex_sessions.clone(),
            files.clone(),
        );
        let task = tokio::task::spawn_blocking(move || read(&copy, sessions.as_deref()));
        let mut fill = match tokio::time::timeout(bound, task).await {
            Ok(Ok(fill)) => fill?,
            Ok(Err(error)) => {
                tracing::debug!(window_id, %error, "the context fill read failed");
                return None;
            }
            Err(_) => {
                let ms = bound.as_millis();
                tracing::debug!(
                    window_id,
                    ms,
                    "the context fill read did not finish in time"
                );
                return None;
            }
        };
        let configured = self
            .ctx
            .settings
            .current()
            .orchestrator
            .context_window_tokens;
        fill.window = window_for(files.runtime, configured, route_model, &fill, |m, r| {
            self.quota.learned_window(m, r)
        });
        Some(fill)
    }

    /// Decisions 24, 25 and 28 at a goal boundary: whether the session in window
    /// `window_id` (the previous run `from_run`'s, on a route whose model is
    /// `route_model`) is due a handoff, measured within `bound`.
    pub(super) async fn measure_handoff(
        &self,
        (window_id, from_run): (u32, &str),
        route_model: &str,
        bound: Duration,
    ) -> Measured {
        let config = self.ctx.settings.current();
        let (percent, cap) = (
            config.orchestrator.handoff_at_percent,
            config.orchestrator.handoff_at_tokens,
        );
        if percent == 0 && cap == 0 {
            return Measured::Off;
        }
        let Some(fill) = self.fill_of(window_id, route_model, bound).await else {
            return Measured::Unknown;
        };
        if cap == 0 && fill.window.is_none() {
            return Measured::Unknown;
        }
        if !handoff_due(fill.tokens, fill.window, percent, cap) {
            return Measured::Below;
        }
        Measured::Due(HandoffRecord {
            from_run: from_run.to_string(),
            from_window: window_id,
            fill_tokens: fill.tokens,
            window_tokens: fill.window,
            at: unix_now(),
        })
    }

    /// Decision 29, from `on_tick`: at most every [`FILL_CHECK_EVERY`], and never while
    /// the last check is still running, each `Running` run's live orchestrator is
    /// measured on a task of its own and reported (`EventKind::OrchestratorFill`).
    pub(super) fn check_live(self: &Arc<Self>) {
        let now = Instant::now();
        {
            let mut last = crate::lock(&self.handoff.last);
            if last.is_some_and(|at| now.saturating_duration_since(at) < FILL_CHECK_EVERY)
                || self.handoff.busy.swap(true, Ordering::SeqCst)
            {
                return;
            }
            *last = Some(now);
        }
        let live: Vec<(String, u32, String, u64)> =
            (crate::lock(&self.state).runs.values()) // lookup
                .filter(|run| run.state == RunState::Running)
                .filter_map(|run| {
                    let o = run.orch.orchestrator.as_ref().filter(|o| o.live)?;
                    let model = o.route.model.clone();
                    Some((
                        run.id.clone(),
                        o.window_id?,
                        model,
                        run.limits.git_timeout_secs,
                    ))
                })
                .collect();
        let (service, busy) = (self.clone(), Busy(self.handoff.busy.clone()));
        tokio::spawn(async move {
            let _busy = busy;
            for (run_id, window_id, model, secs) in live {
                let bound = Duration::from_secs(secs);
                let fill = service.fill_of(window_id, &model, bound).await;
                service.send(EventKind::OrchestratorFill { run_id, fill });
            }
        });
    }
}

/// Clears [`FillCheck::busy`] when the check's task ends, however it ends.
struct Busy(Arc<AtomicBool>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
#[path = "handoff_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "handoff_chain_tests.rs"]
mod chain_tests;
