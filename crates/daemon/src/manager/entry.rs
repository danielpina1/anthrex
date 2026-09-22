//! `Process`, `Entry` and `Inner`: what the manager owns and manipulates under its lock.
//!
//! Split out of `manager/mod.rs` (fix wave 4, item 6) along a real responsibility line,
//! not an arbitrary one: `manager/mod.rs` is `WindowManager`'s own `impl` block, the
//! manager's public surface — every `ClientMsg` this crate answers goes through one of
//! its methods. This file is the data those methods operate on: one window's whole record
//! (`Entry`), what actually runs behind it (`Process`), and the manager's whole internal
//! table (`Inner`). `manager::create`, `manager::remove` and `manager::restore` build and
//! consume `Entry`/`Inner` values directly (struct literals, not just method calls), so
//! every item and field here is `pub(super)` — visible throughout `manager` and its
//! descendants, exactly the scope those three siblings need, and no wider (the same
//! reasoning the M6.5 review's Deviation 2 already established for `Entry` staying out of
//! the rest of the crate: `pub(super)` declared here, one level below `manager`, resolves
//! to "visible in `manager`", not to the crate root the way it would if declared directly
//! in `manager/mod.rs`).

use crate::agent_state::AgentState;
use crate::status::{self, StatusContext, StatusEvent};
use crate::window::{Attachment, Window};
use crate::worktree::ManagedWorktree;
use bytes::Bytes;
use proto::{ExitInfo, Status, WindowInfo, WindowSpec};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Instant, SystemTime};
use tokio::sync::{broadcast, watch};

/// What actually runs behind a window (decision 14).
///
/// A restored window has no child, no PTY and no vt100 mirror — everything [`Window`]
/// owns — so it cannot simply hold a [`Window`] in some "not really running" state. This
/// enum is the alternative: [`Process::Live`] is every window `create` or (a later
/// milestone's) `restart` actually spawned, and [`Process::Dormant`] is every window
/// [`super::WindowManager::restore`] rebuilt from the state file. Routing `write_input`,
/// `resize`, `attach`, `snapshot`, `signal_group` and `pid` through this instead of
/// through `Window` directly is what lets every other call site treat a restored window
/// as an ordinary listed window rather than special-casing it.
pub(super) enum Process {
    Live(Window),
    /// `output` is a capacity-1 sender nothing is ever sent on. Its only job is to give
    /// a subscriber a [`broadcast::Receiver`] that exists, so `attach` has something to
    /// hand back — and, once a later milestone's `restart` swaps this entry's `Process`
    /// for a `Live` one, dropping this sender is what closes that receiver, which is how
    /// `forward_output_from` (decision 21) learns to re-attach.
    Dormant {
        output: broadcast::Sender<Bytes>,
        cols: u16,
        rows: u16,
    },
}

pub(super) struct Entry {
    pub(super) id: u32,
    pub(super) name: String,
    pub(super) spec: WindowSpec,
    /// The project root milestone 4's detection found, or `None` when this entry came
    /// from a restored record whose saved `project` was itself `null` (fix wave 4, ruling
    /// 7). `create` always resolves a concrete root before building an `Entry` — a plain
    /// window's own `cwd` is `detect_roots`'s own fallback when git finds nothing better
    /// — so only `restore` can leave this `None`. Never fabricated and never persisted as
    /// a derived value: `info` below derives a *display* value from it on every read
    /// instead, and `state_snapshot` writes it back exactly as it came in, so a record
    /// that arrived with `project: null` still reads `null` after any number of
    /// restore-then-save cycles, leaving a later boot free to re-detect it for real
    /// instead of one save baking a guess in permanently.
    pub(super) project: Option<PathBuf>,
    /// The git worktree *root* this window's git state is keyed on: milestone 4.5's
    /// field, which the registry watches. For a window this daemon made a worktree for
    /// it is that new checkout (design decision 21), which is why it is not the same
    /// question as `managed` below.
    pub(super) worktree: Option<PathBuf>,
    /// The worktree this daemon created *for* this window, `None` for every other
    /// window. Not to be confused with `worktree`: that one answers "which checkout do
    /// we watch", this one answers "did we make it, and may we remove it".
    // milestone 6: restart re-attaches to this, not to `spec.worktree_branch` (risk 7).
    pub(super) managed: Option<ManagedWorktree>,
    /// A `remove_with_worktree` is in flight for this window (design decision 19 step 1).
    /// The window stays listed and keeps running while it is set, because the removal can
    /// still be refused; what the flag stops is a *second* removal reaching git for the
    /// same checkout, which would have two `git worktree remove` calls and two
    /// `unregister`s for one directory.
    pub(super) removing: bool,
    /// A restart (`manager::restart`, task M6.7) is in flight for this window. Guarded by
    /// `manager::restart::Restarting`, which gives it back on every exit path — an early
    /// return, a panic on the blocking pool, or a caller that drops the future — so a
    /// failed restart never leaves the window permanently unrestartable.
    pub(super) restarting: bool,
    pub(super) status: Status,
    pub(super) state: AgentState,
    pub(super) viewers: u32,
    pub(super) since: Instant,
    pub(super) last_output: Instant,
    /// When this window was first created, preserved verbatim across a restore (decision
    /// 14) so the state file's `created_at` never resets just because the daemon did.
    /// Distinct from `since`, which restarts at every status change including a restore.
    pub(super) created_at: SystemTime,
    pub(super) exit: Option<ExitInfo>,
    pub(super) child_alive: bool,
    pub(super) process: Process,
    /// The loaded `WindowRecord.run` this window's entry came in with, kept opaque and
    /// carried verbatim (whole-branch-review Major 2). This milestone gives it no
    /// meaning of its own (decision 8) — `None` for every window `create` makes — but
    /// `state.rs`'s own forward-compatibility rationale for typing `run` as opaque JSON
    /// only holds if a value loaded from a newer daemon's file survives back out to
    /// `state_snapshot` unchanged instead of being zeroed on the first save.
    pub(super) run: Option<serde_json::Value>,
}

impl Entry {
    pub(super) fn info(&self, now: Instant) -> WindowInfo {
        WindowInfo {
            id: self.id,
            name: self.name.clone(),
            runtime: self.spec.runtime,
            cwd: self.spec.cwd.clone(),
            // `WindowInfo.project` (the wire type) has no `null` case, so an unknown
            // project is derived here, at display time, rather than fabricated once and
            // persisted — the same "no worse than a plain window" fallback `restore`
            // would otherwise have baked into the saved state permanently.
            project: self
                .project
                .clone()
                .unwrap_or_else(|| self.spec.cwd.clone()),
            worktree: self.worktree.clone(),
            branch: self.spec.worktree_branch.clone(),
            status: self.status,
            tool: self.state.tool.clone(),
            since_secs: self.since.elapsed().as_secs(),
            last_output_secs: self.last_output.elapsed().as_secs(),
            session_id: self.state.session_id.clone(),
            model: self.spec.model.clone(),
            subagents: self.state.subagents.infos(now),
            exit: self.exit.clone(),
        }
    }

    /// Applies a status event; returns whether the status changed.
    pub(super) fn apply(&mut self, event: StatusEvent) -> bool {
        self.apply_with_context(event, self.state.context(self.viewers > 0))
    }

    pub(super) fn apply_with_context(&mut self, event: StatusEvent, ctx: StatusContext) -> bool {
        let next = status::next(self.status, event, self.spec.runtime, ctx);
        if next == self.status {
            return false;
        }
        self.status = next;
        self.since = Instant::now();
        true
    }

    pub(super) fn pid(&self) -> Option<u32> {
        match &self.process {
            Process::Live(window) => window.pid(),
            Process::Dormant { .. } => None,
        }
    }

    /// Decision 14's dormant behaviour: refuses with a message naming the restart paths.
    pub(super) fn write_input(&self, bytes: &[u8]) -> anyhow::Result<()> {
        match &self.process {
            Process::Live(window) => window.write_input(bytes),
            Process::Dormant { .. } => anyhow::bail!(
                "window is not running; restart it with C-b R or anthrex restart {}",
                self.id
            ),
        }
    }

    /// Decision 14: a dormant window has no PTY to resize, so this just records the size
    /// for the placeholder snapshot `attach`/`snapshot` build from.
    pub(super) fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        match &mut self.process {
            Process::Live(window) => window.resize(cols, rows),
            Process::Dormant {
                cols: c, rows: r, ..
            } => {
                *c = cols;
                *r = rows;
                Ok(())
            }
        }
    }

    pub(super) fn size(&self) -> (u16, u16) {
        match &self.process {
            Process::Live(window) => window.size(),
            Process::Dormant { cols, rows, .. } => (*cols, *rows),
        }
    }

    pub(super) fn attach(&self) -> Attachment {
        match &self.process {
            Process::Live(window) => window.attach(),
            Process::Dormant { output, cols, rows } => Attachment {
                output: output.subscribe(),
                snapshot: self.dormant_placeholder(),
                cols: *cols,
                rows: *rows,
            },
        }
    }

    pub(super) fn snapshot(&self) -> Vec<u8> {
        match &self.process {
            Process::Live(window) => window.snapshot(),
            Process::Dormant { .. } => self.dormant_placeholder(),
        }
    }

    /// Decision 14's placeholder screen: cleared, then a fixed explanation, then the
    /// session id to resume when one is known.
    fn dormant_placeholder(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\x1b[2J\x1b[H");
        out.extend_from_slice(b"[anthrex] this window stopped when the daemon restarted.\r\n");
        out.extend_from_slice(
            format!(
                "[anthrex] restart it with C-b R (default keys) or: anthrex restart {}\r\n",
                self.id
            )
            .as_bytes(),
        );
        if let Some(session_id) = &self.state.session_id {
            out.extend_from_slice(
                format!("[anthrex] the restart resumes session {session_id}.\r\n").as_bytes(),
            );
        }
        out
    }

    /// Decision 14: succeeds and does nothing for a dormant window — there is no process
    /// group to signal.
    pub(super) fn signal_group(&self, sig: i32) -> anyhow::Result<()> {
        match &self.process {
            Process::Live(window) => window.signal_group(sig),
            Process::Dormant { .. } => Ok(()),
        }
    }
}

pub(super) struct Inner {
    pub(super) next_id: u32,
    pub(super) shutting_down: bool,
    pub(super) entries: BTreeMap<u32, Entry>,
    /// Names of creates that have been admitted but whose window does not exist yet
    /// (design decision 15, phase A). A name is taken from the moment a create is
    /// admitted, because phase B can sit in `git worktree add` for seconds and two
    /// creates racing on one name would otherwise both pass the duplicate check.
    pub(super) reserved_names: BTreeSet<String>,
    /// Worktree directories that admitted creates are on their way to making, held for
    /// exactly as long as `reserved_names` holds their window's name.
    ///
    /// Without this, two creates with different names and the same branch in one
    /// repository both enter phase B and run git concurrently: the second one's
    /// pre-flight checks pass before the first's `worktree add` has registered anything,
    /// so it goes on to `worktree add` itself, fails with "already exists", and cleans up
    /// after what it thinks is its own half-made worktree — which is the first agent's
    /// live checkout, removed with `--force`, and its branch deleted with it. Running
    /// parallel agents on one repository is what this milestone is *for*, so that is the
    /// normal case, not an exotic one.
    pub(super) reserved_worktrees: BTreeSet<PathBuf>,
    // Cleanup owns a group beyond the leader's exit and even after window removal.
    pub(super) cleanups: BTreeMap<u32, watch::Receiver<bool>>,
    /// Cleanup records `restart`'s `finish_restart` evicted from `cleanups` because their
    /// id was about to be reused by a new `Process` (fix wave 5, Critical 1). `cleanups`
    /// is keyed by window id and `start_cleanup` short-circuits whenever a record for that
    /// id already exists, which was sound before restart existed — nothing respawned into
    /// a live id, so an id was killed at most once meaningfully. Once an id can be reused,
    /// leaving the old child's record under that key would make every `start_cleanup` for
    /// the *new* process a silent no-op forever: `kill`, the next `restart`'s own phase B,
    /// and `shutdown` would all believe cleanup was already in hand and signal nothing.
    ///
    /// So `finish_restart` moves the old record here instead of just dropping it: the
    /// escalation thread behind it (`crate::process::escalate`) owns its `Sender` and
    /// keeps running to HUP/TERM/KILL the old process group regardless of who, if anyone,
    /// holds the `Receiver` — but `shutdown` still needs to *wait* for it before the daemon
    /// exits and takes that thread down with it, or a descendant the old group's leader
    /// left behind could survive as an orphan. Not keyed by id, because after eviction
    /// nothing should ever look it up by the window's id again — it belongs to a process
    /// that no longer has one. `tick` prunes finished entries the same way it prunes
    /// `cleanups`.
    pub(super) orphaned_cleanups: Vec<watch::Receiver<bool>>,
    /// The loaded state file's top-level `runs`, kept opaque and carried verbatim
    /// (whole-branch-review Major 2), for the same reason as `Entry.run` above: this
    /// milestone never populates or reads it (decision 8), but `restore` must not
    /// silently drop what a newer daemon's file put here, or the first `state_snapshot`
    /// after loading would erase it instead of round-tripping it unchanged. Overwritten,
    /// not merged, on every `restore` call — the same "the loaded file is the new
    /// starting point" treatment `next_id` gets, since `restore` runs once at startup in
    /// production and this field has no shape of its own for this module to merge by.
    pub(super) runs: Vec<serde_json::Value>,
}

impl Inner {
    /// Returns whether this call actually inserted a `cleanups[id]` record — `false`
    /// both when one was already there (short-circuited below) and when the window has
    /// no live child to escalate. Callers that need to know whether *they* now own the
    /// record under `id` (`restart.rs`'s `Restarting` guard, final-gate finding F1) must
    /// use this return value rather than assuming that reaching this call at all means
    /// ownership: an already-occupied `cleanups[id]` belongs to whoever inserted it
    /// first, and this call does nothing to it either way.
    pub(super) fn start_cleanup(&mut self, id: u32) -> anyhow::Result<bool> {
        if self.cleanups.contains_key(&id) {
            return Ok(false);
        }
        let entry = self
            .entries
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        if entry.child_alive
            && let Some(pid) = entry.pid()
        {
            self.cleanups.insert(id, crate::process::escalate(pid)?);
            return Ok(true);
        }
        Ok(false)
    }

    /// `restart`'s `finish_restart`: this id's `Process` is about to be swapped for a new
    /// one, so any cleanup record still keyed on `id` belongs to the process being
    /// replaced, not the one that will run under this id from now on. Moving it here
    /// rather than dropping it keeps `shutdown` able to wait for it — see
    /// `orphaned_cleanups`' own doc comment — while leaving `cleanups` free of the stale
    /// entry that made `start_cleanup` a no-op for the new process (fix wave 5, Critical 1).
    pub(super) fn orphan_cleanup(&mut self, id: u32) {
        if let Some(done) = self.cleanups.remove(&id) {
            self.orphaned_cleanups.push(done);
        }
    }
}
