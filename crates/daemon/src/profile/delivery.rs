//! Milestone 9.2 decision 3 (task M9.2.12), I/O: `[delivery]` detected at `profile
//! detect`. After verification, a proposal carries `[delivery] mode = "pr"` and `remote =
//! "origin"` only when `origin` is a GitHub URL and `gh` is installed, logged in to its
//! host and can see the repository (preflight's checks 1 to 4, [`CodeHost::detect`]: no
//! push); otherwise it carries no `[delivery]` table, so every proposal is unchanged
//! where `gh` is absent. The host call runs on `spawn_blocking` within a bound. The
//! user confirms the table with the profile, like every other key.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use proto::{DeliveryMode, DeliveryProfile};

use super::service::ProfileService;
use crate::host::{CodeHost, HOST_READ_TIMEOUT};

/// Detection's bound: its four checks' own (decision 9), and a margin.
pub const DETECT_BOUND: Duration = Duration::from_secs(4 * HOST_READ_TIMEOUT.as_secs() + 5);

/// The remote detection looks at (decision 3).
pub const DETECTED_REMOTE: &str = "origin";

/// Decision 3, blocking: the table a proposal for the repository at `root` carries.
pub fn detect(root: &Path, host: &dyn CodeHost) -> Option<DeliveryProfile> {
    match host.detect(root, DETECTED_REMOTE) {
        Ok(_) => Some(DeliveryProfile {
            mode: DeliveryMode::Pr,
            remote: DETECTED_REMOTE.to_string(),
        }),
        Err(error) => {
            tracing::debug!(%error, "no [delivery] table proposed");
            None
        }
    }
}

/// [`detect`] on a blocking thread within [`DETECT_BOUND`]; no table without a host, on
/// a timeout, or when the host panics.
pub async fn detected(host: Option<Arc<dyn CodeHost>>, root: PathBuf) -> Option<DeliveryProfile> {
    let host = host?;
    let work = tokio::task::spawn_blocking(move || detect(&root, &*host));
    match tokio::time::timeout(DETECT_BOUND, work).await {
        Ok(Ok(table)) => table,
        _ => None,
    }
}

impl ProfileService {
    /// Decision 15: the daemon's code host, which detection asks (set once by `wire`).
    pub fn set_host(&self, host: Arc<dyn CodeHost>) {
        let _ = self.host.set(host);
    }

    pub(super) fn code_host(&self) -> Option<Arc<dyn CodeHost>> {
        self.host.get().cloned()
    }
}

#[cfg(test)]
#[path = "tests_detect.rs"]
mod tests;
