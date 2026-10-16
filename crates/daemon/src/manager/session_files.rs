//! Milestone 9.16 decision 26 (ruling C16): the files a window's root session writes,
//! copied out under the manager's lock for the context handoff to read off every lock.
//! No I/O.

use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use proto::Runtime;

use super::WindowManager;

/// A window's root session, as its hooks named it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFiles {
    pub runtime: Runtime,
    /// Claude's `transcript_path` (its `SessionStart` hook).
    pub transcript_path: Option<PathBuf>,
    /// The session's id: Codex's thread id names its rollout (decision 9).
    pub session_id: Option<String>,
    /// When the window was created (Unix seconds, kept across restores): the date a
    /// rollout is searched around.
    pub started_at: u64,
}

impl WindowManager {
    /// Window `window_id`'s [`SessionFiles`]; `None` for a window that is not there.
    pub fn session_files(&self, window_id: u32) -> Option<SessionFiles> {
        let inner = crate::lock(&self.inner);
        let entry = inner.entries.get(&window_id)?;
        Some(SessionFiles {
            runtime: entry.spec.runtime,
            transcript_path: entry.conversations.transcript_path().map(PathBuf::from),
            session_id: entry.conversations.session_id().map(str::to_owned),
            started_at: (entry.created_at.duration_since(UNIX_EPOCH)).map_or(0, |d| d.as_secs()),
        })
    }
}

#[cfg(test)]
#[path = "session_files_tests.rs"]
mod tests;
