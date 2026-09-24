//! The end of a headless session the engine stops (decision 52): retirement closes the
//! session's stdin, a kill signals its group, and either way a window with no process
//! left to exit is `Exited` at once, so a watcher sees the session is over (ruling
//! T18-N3). The process a window last started is what the engine matches a kill's exit
//! against. Nothing here does I/O under the manager lock (AGENTS.md rule 2).

use super::WindowManager;
use super::entry::Process;
use crate::headless::session::HeadlessHandle;
use proto::Status;
use std::time::Instant;

impl WindowManager {
    /// Decision 52's retirement: stdin is closed once its queued lines are written, so a
    /// Claude process ends on EOF; the engine kills the group after `INTERRUPT_GRACE`.
    pub fn headless_retire(&self, id: u32) -> anyhow::Result<()> {
        let handle = self.mark_ending(id)?;
        handle.close_stdin();
        Ok(())
    }

    /// The pid window `id`'s current process was started with, reaped or not: the pid
    /// its events carry. `None` for a window that never started one.
    pub fn headless_pid(&self, id: u32) -> Option<u32> {
        let inner = crate::lock(&self.inner);
        match &inner.entries.get(&id)?.process {
            Process::Headless(window) => window.handle.spawned_pid(),
            _ => None,
        }
    }

    /// Marks window `id`'s session as ending, under the lock, and returns its handle.
    pub(super) fn mark_ending(&self, id: u32) -> anyhow::Result<HeadlessHandle> {
        let mut inner = crate::lock(&self.inner);
        let entry = inner
            .entries
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        let Process::Headless(window) = &mut entry.process else {
            anyhow::bail!("window {id} is not a headless session");
        };
        window.ending = true;
        let handle = window.handle.clone();
        if handle.is_ended() && entry.status != Status::Exited {
            window.status.status = Status::Exited;
            window.status.turn_open = false;
            entry.status = Status::Exited;
            entry.since = Instant::now();
            entry.child_alive = false;
            self.publish(&inner);
        }
        Ok(handle)
    }
}
