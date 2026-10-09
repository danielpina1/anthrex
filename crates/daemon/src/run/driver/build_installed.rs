//! Milestone 9.5 rulings RL-2 and I6: the onboarding start and each decider call route
//! over what is installed, reusing the run start's probe (`build::found`) through
//! [`installed_with`]: on `spawn_blocking`, never under a lock, bounded by
//! [`INSTALLED_PROBE_TIMEOUT`], and never cached. Part of `build.rs`.

use std::collections::BTreeMap;
use std::time::Duration;

use super::{Found, found};

/// How long the probe may take. It stats two paths; only a hung file system is slower.
/// A goal start's triage waits for it before the decider call (`docs/timing-budgets.md`).
pub const INSTALLED_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Ruling I6: [`found`] for the two configured binaries, within `timeout`; `None` past
/// it, or when the probe panicked.
#[cfg(test)]
pub(crate) async fn found_within(
    claude: String,
    codex: String,
    timeout: Duration,
) -> Option<Found> {
    within(move || found(&claude, &codex), timeout).await
}

/// `probe` on `spawn_blocking`, within `timeout` (a probe that blocks past it is left
/// to end on its own, holding no lock).
async fn within<T: Send + 'static>(
    probe: impl FnOnce() -> T + Send + 'static,
    timeout: Duration,
) -> Option<T> {
    let spawned = tokio::task::spawn_blocking(probe);
    tokio::time::timeout(timeout, spawned).await.ok()?.ok()
}

/// What an onboarding start or a decider call routes over: the probe's headless map
/// (`Found.headless`, the map a run records). Empty, so every runtime counts as
/// installed and the route resolves exactly as before, when the probe gave up or found
/// nothing installed.
pub(crate) fn installed_of(found: Option<Found>) -> BTreeMap<String, bool> {
    let headless = found.map(|f| f.headless);
    headless
        .filter(|m| m.values().any(|&v| v))
        .unwrap_or_default()
}

/// [`installed_of`] what [`found_within`] finds now, for `claude` and `codex`.
#[cfg(test)]
pub(crate) async fn installed_now(claude: String, codex: String) -> BTreeMap<String, bool> {
    installed_of(found_within(claude, codex, INSTALLED_PROBE_TIMEOUT).await)
}

/// Milestone 9.8 (preflight ruling F25): what [`found`] finds now, as
/// [`installed_of`] reads it, with `read` (a repository's `models.toml`) run on the same
/// `spawn_blocking`, bounded by [`INSTALLED_PROBE_TIMEOUT`] as the probe is. Past the
/// bound, or when either panicked: everything counted installed and no read.
pub(crate) async fn installed_with<T: Send + 'static>(
    claude: String,
    codex: String,
    read: impl FnOnce() -> T + Send + 'static,
) -> (BTreeMap<String, bool>, Option<T>) {
    let both = move || (found(&claude, &codex), read());
    match within(both, INSTALLED_PROBE_TIMEOUT).await {
        Some((found, read)) => (installed_of(Some(found)), Some(read)),
        None => (BTreeMap::new(), None),
    }
}

#[cfg(test)]
#[path = "build_installed_tests.rs"]
mod tests;
