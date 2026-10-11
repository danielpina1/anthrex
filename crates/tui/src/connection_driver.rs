//! The connection's lifecycle driver (`event_loop`'s reconnect state), its own module so
//! `cli`'s integration binary can drive it against a real daemon (milestone 9.12
//! decision 11).

use crate::app::{App, Effect};
use crate::connection::Connection;
use crate::{reconnect, spawn_attempt};
use proto::DaemonMsg;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Everything about the connection's lifecycle beyond keyboard, mouse and the
/// once-a-frame tick: the retry schedule, the in-flight attempt task, and the last
/// `Bye` reason seen (decisions 30-32). Kept as its own small piece of state — rather
/// than three more local variables in `event_loop` — so `step` can be driven from a
/// test against a real socket with no terminal at all (see `reconnect_tests.rs`),
/// which is the only way this repo's `AGENTS.md` hard rule 6 ("test real behaviour")
/// can be honoured for an async selection loop this shaped.
pub struct ConnectionDriver {
    pub(crate) schedule: Option<reconnect::RetrySchedule>,
    pub(crate) inflight: Option<tokio::task::JoinHandle<anyhow::Result<Connection>>>,
    pub(crate) last_bye_reason: Option<String>,
}

// `new` is public only for `cli`'s tests (decision 11); a `Default` would be one more
// public item, which the decision rules out.
#[allow(clippy::new_without_default)]
impl ConnectionDriver {
    pub fn new() -> Self {
        Self {
            schedule: None,
            inflight: None,
            last_bye_reason: None,
        }
    }

    /// Waits for whichever comes next: a message on the (still open) connection, the
    /// link actually closing, the retry schedule coming due, or an in-flight attempt
    /// finishing — and returns the `App` effects it produced.
    ///
    /// Each branch's future is wrapped in its own `async { ... }` block rather than
    /// written as a bare `conn.as_mut().unwrap().recv()`-style expression: `tokio::select!`
    /// evaluates every branch's expression up front (even a disabled one) to build its
    /// future, and only skips *polling* the disabled ones. A bare `.unwrap()` would
    /// therefore panic on a `None` guard-skipped branch; wrapping it in `async {}`
    /// defers that body to poll time, where the guard has already ruled it out.
    pub async fn step(
        &mut self,
        conn: &mut Option<Connection>,
        app: &mut App,
        socket: &Path,
    ) -> Vec<Effect> {
        let due_at = self.schedule.map(|s| s.next_due());
        let conn_ready = conn.is_some();
        let due_ready = due_at.is_some() && self.inflight.is_none();
        let inflight_ready = self.inflight.is_some();

        tokio::select! {
            msg = async { conn.as_mut().unwrap().recv().await }, if conn_ready => {
                self.on_recv(conn, app, msg)
            }
            _ = async {
                tokio::time::sleep_until(tokio::time::Instant::from_std(due_at.unwrap())).await
            }, if due_ready => {
                self.inflight = Some(spawn_attempt(socket.to_path_buf(), None));
                vec![]
            }
            result = async { self.inflight.as_mut().unwrap().await }, if inflight_ready => {
                self.on_attempt_finished(conn, app, result)
            }
            // Decision 31's give-up path leaves every guard above false at once
            // (`conn = None`, `inflight = None`, `schedule = None`) — a `select!`
            // with no `else` and every arm disabled panics the instant it is
            // polled in that state, which happened live exactly 30s after a
            // daemon disappeared. The obvious alternative, `else => vec![]`,
            // returns immediately on every poll and turns `event_loop`'s outer
            // loop into a redraw-every-pass busy spin (measured 95.7-97.3% CPU) —
            // a regression that is *harder* to notice than the panic, since the
            // status bar still looks correct. Parking forever is correct instead:
            // nothing here can make progress until something re-arms the driver
            // (a manual `C-b r` via `reconnect_now`, called from `event_loop`
            // before the next `step`), so this branch must simply never resolve
            // on its own and let the outer `select!` keep servicing keyboard/tick
            // events. Measured CPU with this arm: ~3-4% (see the item's report).
            else => {
                std::future::pending::<()>().await;
                unreachable!("parks forever; only re-armed state reaches step again")
            }
        }
    }

    /// A closed receive drops the connection, calls `app.on_link_lost`, and opens the
    /// automatic retry window (decision 30) — unless the effects it returned include
    /// `Effect::Quit`, i.e. this was `C-b Q`'s own wait ending, not an unplanned drop.
    pub(crate) fn on_recv(
        &mut self,
        conn: &mut Option<Connection>,
        app: &mut App,
        msg: Option<DaemonMsg>,
    ) -> Vec<Effect> {
        match msg {
            Some(DaemonMsg::Bye { reason }) => {
                self.last_bye_reason = Some(reason.clone());
                app.on_daemon(DaemonMsg::Bye { reason })
            }
            Some(msg) => app.on_daemon(msg),
            None => {
                *conn = None;
                let reason = self
                    .last_bye_reason
                    .take()
                    .unwrap_or_else(|| "connection closed".into());
                let effects = app.on_link_lost(reason);
                if !effects.contains(&Effect::Quit) {
                    self.schedule = Some(reconnect::RetrySchedule::after_drop(Instant::now()));
                }
                effects
            }
        }
    }

    /// On success, installs the new connection and hands its window list to
    /// `App::on_reconnected` (decision 33). On failure, records it against the
    /// schedule and reports it through `App::on_reconnect_failed`, which decides
    /// whether that was the one that gives up (decision 31).
    pub(crate) fn on_attempt_finished(
        &mut self,
        conn: &mut Option<Connection>,
        app: &mut App,
        result: Result<anyhow::Result<Connection>, tokio::task::JoinError>,
    ) -> Vec<Effect> {
        self.inflight = None;
        let outcome = match result {
            Ok(inner) => inner,
            Err(join_err) => Err(anyhow::anyhow!("reconnect task failed: {join_err}")),
        };
        match outcome {
            Ok(new_conn) => {
                let windows = new_conn.windows.clone();
                *conn = Some(new_conn);
                self.schedule = None;
                app.on_reconnected(windows)
            }
            Err(err) => {
                let now = Instant::now();
                let keep_going = self.schedule.as_mut().is_some_and(|s| s.after_failure(now));
                if !keep_going {
                    self.schedule = None;
                }
                app.on_reconnect_failed(err.to_string(), !keep_going)
            }
        }
    }

    /// `Effect::Reconnect` (decision 32, `C-b r`): starts an attempt at once and opens
    /// a fresh 30 s window.
    ///
    /// Fix wave 10, item 2: the user's intent in pressing `C-b r` is "try now, and
    /// start the daemon if it is not running" — decision 32's whole reason a manual
    /// attempt is allowed to carry `daemon_exe` where an automatic one never does. If
    /// an automatic retry (spawned by the "due" branch in `step`, always with
    /// `daemon_exe: None` per decision 31) is already running, that in-flight attempt
    /// cannot be handed this press's `daemon_exe` after the fact — a running future's
    /// captured arguments cannot be mutated, and reworking `reconnect::attempt` to poll
    /// a late-bound signal is a bigger change than this fix needs. So this press
    /// supersedes it instead: abort the stale attempt and spawn a fresh one that
    /// actually carries `daemon_exe`. The stale attempt was never going to start the
    /// daemon anyway (it is automatic), so it has nothing this fresh one loses by being
    /// replaced; `JoinHandle::abort` (AGENTS.md's own facts-learned-the-hard-way) takes
    /// effect at the aborted task's next yield point, and its `Connection`, if it had
    /// already connected, still drops and cleans itself up normally.
    pub(crate) fn reconnect_now(&mut self, socket: &Path, daemon_exe: Option<PathBuf>) {
        self.schedule = Some(reconnect::RetrySchedule::manual(Instant::now()));
        if let Some(stale) = self.inflight.take() {
            stale.abort();
        }
        self.inflight = Some(spawn_attempt(socket.to_path_buf(), daemon_exe));
    }
}
