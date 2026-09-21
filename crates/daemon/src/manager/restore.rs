//! `WindowManager::restore` and `WindowManager::state_snapshot`: the two directions of
//! milestone 6's persistence (decisions 9 and 14). `restore` turns a loaded
//! [`state::StateFile`] into dormant entries at startup; `state_snapshot` turns the live
//! entry table back into one, for [`crate::state::spawn_persister`] and the final flush
//! in [`crate::lifecycle::run`] to write.
//!
//! Both run entirely under the manager lock and do no I/O of any kind — no filesystem,
//! no git, no process spawn. `state_snapshot` in particular is decision 9's "a clone
//! taken under the manager lock with no I/O under it": every field it reads is already
//! in memory, so the lock is held for exactly as long as cloning a handful of small
//! values takes, never for as long as serializing or writing them would.

use super::{DAEMON_RESTARTED, Entry, Process, WindowManager};
use crate::agent_state::AgentState;
use crate::state::{self, StateFile, WindowRecord, WorktreeRecord};
use crate::worktree::ManagedWorktree;
use proto::{ExitInfo, Status, WindowSpec};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio::sync::broadcast;

/// A restored window has no saved terminal size (the state file does not record one), so
/// it starts at this and is resized to the client's real size on the first `Subscribe`,
/// exactly as `server::handle_client` already does for a live window before `attach`.
const RESTORED_SIZE: (u16, u16) = (80, 24);

impl WindowManager {
    /// Rebuilds the window table from a loaded state file (decision 14), called once at
    /// startup, before the socket is bound, so the first client's `Welcome` already lists
    /// every restored window.
    ///
    /// Each record becomes one dormant [`Entry`]: no process, status `Exited`, `exit` set
    /// to [`DAEMON_RESTARTED`]. `next_id` is derived independently of the state file's own
    /// `next_id` field — recomputed here from the loaded records' own ids, the same
    /// saturating max-plus-one `state::load` already applies — rather than trusted
    /// verbatim. `state::load`'s own `next_id` is already correct when this state came
    /// from it, but `restore` is a public entry point in its own right (this milestone's
    /// tests call it directly with a hand-built `StateFile`), and a caller that got
    /// `next_id` wrong must not be able to hand a later `create` a colliding id — the same
    /// class of bug the previous task's review found twice in `state.rs`. Recomputing
    /// here means a restored set with gaps (ids 1 and 9, not 1 and 2) still leaves
    /// `next_id` past every one of them, and a record already holding `u32::MAX` still
    /// saturates instead of wrapping.
    ///
    /// Publishes exactly once, after every record is inserted, rather than once per
    /// record: a watcher (in particular the persister this same startup sequence spawns
    /// moments later) must never see a partially-restored table.
    pub fn restore(&self, state: StateFile) {
        let mut inner = crate::lock(&self.inner);
        let now = Instant::now();
        let mut next_id = inner.next_id;

        for record in state.windows {
            let WindowRecord {
                id,
                name,
                runtime,
                cwd,
                project,
                worktree,
                model,
                initial_prompt,
                session_id,
                created_at,
                status: _saved_status,
                run: _,
            } = record;

            let managed = worktree.map(
                |WorktreeRecord {
                     repo_root,
                     path,
                     branch,
                 }| {
                    ManagedWorktree {
                        repo_root,
                        path,
                        branch,
                    }
                },
            );
            // Decision 14 leaves `Entry.managed` and a worktree window's `cwd` untouched
            // across a restore, so a future restart runs in the same checkout without
            // ever calling `worktree::create` again. `Entry.worktree` — the *watched*
            // root, milestone 4.5's field — is only recoverable here for a window this
            // daemon made the worktree for; a window merely standing inside someone
            // else's existing worktree has no saved record of that at all (decision 8
            // only captures `managed`), so it comes back unwatched until it is restarted.
            let watched_worktree = managed.as_ref().map(|m| m.path.clone());
            let spec = WindowSpec {
                name: Some(name.clone()),
                runtime,
                cwd: cwd.clone(),
                worktree_branch: managed.as_ref().map(|m| m.branch.clone()),
                model,
                initial_prompt,
            };
            // "Starting point": a null saved `project` should re-detect the way milestone
            // 4 does at window creation. That detection is `project::detect_roots`, a
            // blocking git subprocess, which cannot run here — this function is
            // synchronous and holds the manager lock the whole time (AGENTS.md hard
            // rules 2 and 10). Falling back to `cwd`, `detect_roots`'s own fallback when
            // git itself gives no better answer, is deliberately the same "no worse than
            // a plain window" outcome, not a real detection; a real one would need to run
            // before this call, outside the lock, in `lifecycle::run`. Recorded as a
            // deviation in the task report.
            let project = project.unwrap_or_else(|| cwd.clone());
            let (output, _unused) = broadcast::channel(1);

            let entry = Entry {
                id,
                name,
                spec,
                project,
                worktree: watched_worktree,
                managed,
                removing: false,
                restarting: false,
                status: Status::Exited,
                state: AgentState {
                    session_id,
                    ..AgentState::default()
                },
                viewers: 0,
                since: now,
                last_output: now,
                created_at: UNIX_EPOCH
                    .checked_add(Duration::from_secs(created_at))
                    .unwrap_or(UNIX_EPOCH),
                exit: Some(ExitInfo {
                    code: None,
                    reason: DAEMON_RESTARTED.to_string(),
                }),
                child_alive: false,
                process: Process::Dormant {
                    output,
                    cols: RESTORED_SIZE.0,
                    rows: RESTORED_SIZE.1,
                },
            };

            next_id = next_id.max(id.saturating_add(1));
            inner.entries.insert(id, entry);
        }

        inner.next_id = next_id.max(state.next_id);
        self.publish(&inner);
    }

    /// A clone of the live window table as a [`StateFile`] (decision 9), taken under the
    /// manager lock with no I/O of any kind — every field below is already resident in
    /// `Entry`, `Entry.spec` or `Entry.state`, so this is a handful of clones, not a
    /// filesystem call. `runs` is always empty and every record's `run` is always `null`
    /// in this milestone (decision 8); milestone 8 gives them real content.
    pub fn state_snapshot(&self) -> StateFile {
        let inner = crate::lock(&self.inner);
        let windows = inner
            .entries
            .values()
            .map(|entry| WindowRecord {
                id: entry.id,
                name: entry.name.clone(),
                runtime: entry.spec.runtime,
                cwd: entry.spec.cwd.clone(),
                project: Some(entry.project.clone()),
                worktree: entry.managed.as_ref().map(|m| WorktreeRecord {
                    repo_root: m.repo_root.clone(),
                    path: m.path.clone(),
                    branch: m.branch.clone(),
                }),
                model: entry.spec.model.clone(),
                initial_prompt: entry.spec.initial_prompt.clone(),
                session_id: entry.state.session_id.clone(),
                created_at: entry
                    .created_at
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                status: entry.status,
                run: None,
            })
            .collect();
        StateFile {
            version: state::STATE_VERSION,
            next_id: inner.next_id,
            windows,
            runs: Vec::new(),
        }
    }
}
