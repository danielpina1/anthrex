//! `WindowManager::restart`: bringing a window whose process died — or whose daemon
//! restarted — back to life, resuming the agent's previous session (design decisions
//! 17-21, task M6.7).
//!
//! Like [`super::create`], this is a phase split forced by the same rule (AGENTS.md hard
//! rules 2 and 10): nothing that can block — killing a live child and waiting for it,
//! checking a directory, building a launch plan, spawning a PTY — may run under the
//! manager lock or on a tokio worker thread.
//!
//! - **Phase A**, under the lock and nowhere near a syscall that can stall: refuse a
//!   window that does not exist or is already restarting, note whether it is live, and
//!   set `restarting`. A [`Restarting`] guard gives the flag back on every exit path that
//!   does not reach phase D's swap — an early return, a panic on the blocking pool, or a
//!   caller that drops the future. This is the fix for this task's first named hazard: a
//!   flag left set after a failed restart would brick the window, every later restart
//!   refused, with nothing short of a daemon restart able to clear it.
//! - **Phase B**, no lock: if the window was live, kill it through milestone 3's kill
//!   path and wait — polling, never under the lock — until the child is actually gone.
//!
//!   Design decision 18 says to poll "until the status is Exited". This implementation
//!   polls `Entry.child_alive` instead, and that is a deliberate deviation, not an
//!   oversight: `manager/remove.rs`'s `kill_and_await_exit` already established, with a
//!   long comment, why status and child-liveness are not the same question. A
//!   `WindowEvent::ParserPanicked` sets the status to `Exited` immediately while the
//!   child is still being escalated through HUP/TERM/KILL on a background thread —
//!   `child_alive` only goes false once `WindowEvent::Exited` actually arrives. Restart
//!   has a sharper reason than removal to get this right: phase D reuses this window's
//!   *id* for a brand new `Process`, and every `WindowEvent` this crate delivers is keyed
//!   by id alone, with nothing distinguishing "the old spawn" from "the new spawn" of the
//!   same id. Swapping the process in while the old child's `WindowEvent::Exited` is
//!   still in flight would let that stale event land on the new `Entry` once it finally
//!   arrives — `handle_event`'s `Exited` arm sets `child_alive = false` and re-applies
//!   `StatusEvent::Exited` unconditionally, which would silently mark a freshly
//!   restarted, live window as exited. Waiting on `child_alive` closes that window;
//!   waiting on `status` alone, as decision 18 literally says, would not.
//!
//!   Decision 18 also specifies what happens when the wait itself runs out: the restart
//!   is refused — `Error { request: "restart", message: "window <id> did not exit; not
//!   restarted" }` — rather than proceeding anyway. This is not merely what the brief
//!   says; it is the one path the paragraph above's safety argument does not cover. That
//!   argument is that phase D never reuses this id for a new `Process` until
//!   `child_alive` is confirmed false, so a stale `Exited` can never reach the
//!   replacement. A timeout means `child_alive` was *not* confirmed false — restarting
//!   anyway would swap in a new `Process` with the old child's `Exited` still able to
//!   arrive later and land on it, which is exactly the corruption decision 18's refusal
//!   exists to rule out.
//! - **Phase C**, one `spawn_blocking` call, no lock: re-read the window's current spec,
//!   name and session id — never phase A's own snapshot, which the window could have
//!   outgrown while phase B ran with no lock held at all (this task's second named
//!   hazard) — check its cwd still exists, build the resume plan, and spawn a fresh
//!   `Window`.
//! - **Phase D**, under the lock again: the window may have been removed while phases B
//!   and C ran, so this re-checks it is still there before touching anything, rather than
//!   trusting phase A's finding. Gone means the freshly spawned process is killed at once
//!   and the restart fails without resurrecting an entry nothing references any more.
//!   Still there means the old `Process` is swapped for the new one — closing whatever
//!   broadcast channel a subscriber was reading, which is what wakes `forward_output_from`
//!   into reattaching (design decision 21) — the status fields are reset as for a fresh
//!   create, and the change is published.

use super::entry::Process;
use super::{ManagerConfig, WindowManager};
use crate::agent_state::AgentState;
use crate::launch::{self, LaunchContext};
use crate::window::{Window, WindowEvent};
use proto::{Status, WindowSpec};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// How often phase B asks whether the killed child is gone. Never with the manager lock
/// held (design decision 18).
const RESTART_POLL: Duration = Duration::from_millis(50);

/// Holds a window's `restarting` flag for as long as one restart owns it, and gives it
/// back on every exit path that leaves the window in the table — an early return, a
/// panic on the blocking pool, or a caller that drops the future.
///
/// Exactly the shape of `create`'s `Reservation` and `remove`'s `Removing`, guarding the
/// same class of resource for the same reason: see this module's own doc comment for why
/// a leaked flag here is worse than cosmetic.
struct Restarting<'a> {
    manager: &'a WindowManager,
    id: u32,
    held: bool,
}

impl Restarting<'_> {
    /// The restart reached phase D's swap, which already cleared the flag itself under
    /// the same lock as the swap (design decision 21). Nothing is left for `Drop` to do.
    fn forget(mut self) {
        self.held = false;
    }
}

impl Drop for Restarting<'_> {
    fn drop(&mut self) {
        if !self.held {
            return;
        }
        let mut inner = crate::lock(&self.manager.inner);
        if let Some(entry) = inner.entries.get_mut(&self.id) {
            entry.restarting = false;
        }
    }
}

/// What phase C needs, read fresh under the lock right before the blocking work starts —
/// never phase A's own snapshot (this task's second named hazard).
struct ForRelaunch {
    spec: WindowSpec,
    name: String,
    session_id: Option<String>,
    cols: u16,
    rows: u16,
}

impl WindowManager {
    /// Restarts a window: relaunches its agent, resuming the session it last knew about
    /// (design decision 15), killing whatever was running first if the window was live
    /// (design decisions 17-18).
    ///
    /// `self: &Arc<Self>`, not `&self`, only because phase B calls `Self::kill`, which
    /// needs it for the same reason `kill` itself does.
    pub async fn restart(self: &Arc<Self>, id: u32) -> anyhow::Result<()> {
        // Phase A.
        let (was_live, guard) = self.begin_restart(id)?;

        // Phase B: never under the lock. Decision 18: if the wait times out, this does
        // *not* fall through to phase C — see `wait_for_exit`'s own doc comment for why
        // restarting anyway is exactly the case the `child_alive` deviation cannot cover.
        if was_live {
            self.kill(id)?;
            if !self.wait_for_exit(id).await {
                anyhow::bail!("window {id} did not exit; not restarted");
            }
        }

        // Phase C: no lock, not on a tokio worker. Re-read fresh rather than trusting
        // phase A's own snapshot, which could be stale by now — the window could have
        // been removed, resized, or (in a later milestone) had its spec changed while
        // phase B ran with no lock held at all.
        let Some(info) = self.snapshot_for_relaunch(id) else {
            anyhow::bail!("window {id} was removed while it was restarting");
        };
        let config = self.config.clone();
        let events = self.events.clone();
        let window =
            tokio::task::spawn_blocking(move || spawn_for_restart(id, &config, events, info))
                .await
                .map_err(|error| anyhow::anyhow!("restart failed: {error}"))??;

        // Phase D.
        self.finish_restart(id, window, guard)
    }

    /// Phase A. Nothing here can block.
    fn begin_restart(&self, id: u32) -> anyhow::Result<(bool, Restarting<'_>)> {
        let mut inner = crate::lock(&self.inner);
        let entry = inner
            .entries
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        if entry.restarting {
            anyhow::bail!("window {id} is already restarting");
        }
        let was_live = entry.child_alive;
        entry.restarting = true;
        Ok((
            was_live,
            Restarting {
                manager: self,
                id,
                held: true,
            },
        ))
    }

    /// Phase B's wait. See this module's doc comment for why this polls `child_alive`
    /// rather than the status decision 18 names, and never with the lock held.
    ///
    /// Returns `true` once the child is confirmed gone (or the window itself is gone —
    /// phase C's own fresh read handles that case), `false` if the kill grace ran out
    /// first. Decision 18 is explicit that a timeout here must not restart anyway: `window
    /// <id> did not exit; not restarted`, not a warning and a fall-through. This matters
    /// beyond the literal wording — the `child_alive` deviation this module's own doc
    /// comment explains only closes the stale-`Exited` race *because* phase D never swaps
    /// in a new `Process` until `child_alive` is confirmed false. Restarting on a timeout
    /// would swap one in anyway, with the old child's `Exited` still unconfirmed and
    /// therefore still able to arrive after the swap and land on the live replacement —
    /// exactly the corruption the deviation exists to rule out, reopened on precisely the
    /// path a timeout takes.
    ///
    /// Bounded by the kill escalation's own total grace plus two seconds (decision 18):
    /// production's `self.config.kill_grace` is `crate::process::KILL_GRACE`, the exact
    /// deadline `crate::process::escalate` is built around, so this can never give up
    /// while that escalation could legitimately still be running.
    async fn wait_for_exit(&self, id: u32) -> bool {
        let deadline = Instant::now() + self.config.kill_grace + Duration::from_secs(2);
        loop {
            let child_alive = {
                let inner = crate::lock(&self.inner);
                inner.entries.get(&id).map(|entry| entry.child_alive)
            };
            match child_alive {
                Some(true) => {}
                Some(false) | None => return true,
            }
            if Instant::now() >= deadline {
                tracing::warn!(
                    id,
                    "window's child did not exit within the restart kill grace; not restarting"
                );
                return false;
            }
            tokio::time::sleep(RESTART_POLL).await;
        }
    }

    /// What phase C needs, read fresh right before the blocking work starts. `None` means
    /// the window is gone — removed while phase B's kill wait ran with no lock held.
    fn snapshot_for_relaunch(&self, id: u32) -> Option<ForRelaunch> {
        let inner = crate::lock(&self.inner);
        let entry = inner.entries.get(&id)?;
        let (cols, rows) = entry.size();
        Some(ForRelaunch {
            spec: entry.spec.clone(),
            name: entry.name.clone(),
            session_id: entry.state.session_id.clone(),
            cols,
            rows,
        })
    }

    /// Phase D. Re-checks the window is still there before touching anything (this
    /// task's second named hazard): gone means the freshly spawned process is killed at
    /// once and the restart fails without resurrecting an entry nothing references any
    /// more. Still there means the swap happens under the same lock `attach` takes, so a
    /// re-attach always sees the new process (design decision 21).
    fn finish_restart(&self, id: u32, window: Window, guard: Restarting<'_>) -> anyhow::Result<()> {
        let mut inner = crate::lock(&self.inner);
        // Critical 1 (fix wave 5): this id is about to be reused for `window`, so any
        // cleanup record `start_cleanup` left under `id` belongs to the process being
        // replaced. Left in place, it would make `kill`, this same id's next `restart`,
        // and `shutdown` all believe the *new* process already has a cleanup in flight and
        // signal nothing — see `Inner::orphan_cleanup` and `orphaned_cleanups`'s own doc
        // comment for why this evicts rather than drops. Done before the "gone" check
        // below too: a window removed mid-restart can still have a stale record from
        // phase B's kill, and `tick` already keeps that alive by id-absence alone, but
        // evicting it here is harmless and keeps this one call site unconditional.
        inner.orphan_cleanup(id);
        let Some(entry) = inner.entries.get_mut(&id) else {
            drop(inner);
            let _ = window.signal_group(libc::SIGKILL);
            anyhow::bail!("window {id} was removed while it was restarting");
        };
        let now = Instant::now();
        let session_id = entry.state.session_id.clone();
        entry.process = Process::Live(window);
        entry.status = Status::Starting;
        // Decision 17: the session id is kept, `exit`, `tool` and the sub-agent list are
        // cleared — the same shape `AgentState::default()` gives a fresh create, with the
        // one field carried over.
        entry.state = AgentState {
            session_id,
            ..AgentState::default()
        };
        entry.since = now;
        entry.last_output = now;
        entry.exit = None;
        entry.child_alive = true;
        // Cleared here, under the same lock as the swap, rather than left for the
        // guard's `Drop` a moment later: this is the one lock acquisition decision 21
        // requires the swap to happen under, so the flag's own release rides along with
        // it instead of taking the lock a second time for nothing.
        entry.restarting = false;
        tracing::info!(id, name = %entry.name, "window restarted");
        self.publish(&inner);
        drop(inner);
        guard.forget();
        Ok(())
    }
}

/// Phase C, the only phase that can take real time: called only from `spawn_blocking`,
/// holding no lock of any kind.
fn spawn_for_restart(
    id: u32,
    config: &ManagerConfig,
    events: mpsc::UnboundedSender<(u32, WindowEvent)>,
    info: ForRelaunch,
) -> anyhow::Result<Window> {
    // Design decision 19: the same check `create`'s phase B makes, and the same message.
    // A restart never calls `worktree::create` (decision 17), so this is the only
    // precondition it has to check on its own.
    if !info.spec.cwd.is_dir() {
        anyhow::bail!("directory does not exist: {}", info.spec.cwd.display());
    }
    let plan = launch::plan(
        &info.spec,
        &LaunchContext {
            window_id: id,
            // The window's *current* name, not `spec.name` (its name at creation time):
            // decision 22 says a resume passes a renamed window's new name to `--name`.
            name: &info.name,
            socket_path: &config.socket_path,
            shell: &config.shell,
            exe: &config.exe,
            claude_bin: &config.claude_bin,
            codex_bin: &config.codex_bin,
            codex_hook_source: config.codex_hook_source.as_deref(),
            codex_bypass_hook_trust: config.codex_bypass_hook_trust,
            resume: info.session_id.as_deref(),
        },
    );
    Window::spawn(id, &plan, info.cols.max(1), info.rows.max(1), events)
}
