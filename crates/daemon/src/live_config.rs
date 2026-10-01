//! Milestone 9.0.6 decision 29: the daemon's live `[orchestrator]` settings. A Settings
//! save swaps the settings-owned fields of the value new runs, goals and the onboarding
//! scout read; every other key stays as the daemon loaded it at start, and a
//! running run keeps the roster and limits it froze at build.
//!
//! **Lock.** `inner` is a `std::sync::Mutex<Arc<Live>>`, taken with `crate::lock` only to
//! clone or replace the `Arc` (Global Constraint 4): a reader takes [`LiveSettings::current`]
//! once and reads one consistent value from it, with no lock held.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proto::{Origin, SettingsDoc};

/// Decision 28: how long a settings save may take before the daemon answers without it.
pub const SETTINGS_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// One value of the live settings: the `[orchestrator]` table and where each owned key
/// came from (decision 26).
#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    pub orchestrator: config::Orchestrator,
    pub origin: BTreeMap<String, Origin>,
}

/// See the module doc.
#[derive(Debug)]
pub struct LiveSettings {
    inner: Mutex<Arc<Live>>,
}

impl LiveSettings {
    pub fn new(orchestrator: config::Orchestrator, origin: BTreeMap<String, Origin>) -> Arc<Self> {
        Arc::new(LiveSettings {
            inner: Mutex::new(Arc::new(Live {
                orchestrator,
                origin,
            })),
        })
    }

    /// `orchestrator` with every one of the thirteen keys `Default` (preflight F20: what a
    /// context built without a config file holds).
    pub fn defaults_of(orchestrator: config::Orchestrator) -> Arc<Self> {
        let origin = proto::settings::SETTINGS_KEYS
            .iter()
            .map(|k| (k.to_string(), Origin::Default))
            .collect();
        Self::new(orchestrator, origin)
    }

    /// The value now: a snapshot a later swap does not change.
    pub fn current(&self) -> Arc<Live> {
        crate::lock(&self.inner).clone()
    }

    /// Replaces only the settings-owned fields with `from`'s (`config::settings::
    /// apply_owned`) and the origin with `origin`. Callers serialise swaps (the run
    /// service's `settings_write`); the new value is built outside the lock.
    pub fn swap_owned(&self, from: &config::Orchestrator, origin: BTreeMap<String, Origin>) {
        let mut orchestrator = self.current().orchestrator.clone();
        config::settings::apply_owned(&mut orchestrator, from);
        let next = Arc::new(Live {
            orchestrator,
            origin,
        });
        *crate::lock(&self.inner) = next;
    }
}

/// The blocking save the run service calls: `config::settings::save` in the daemon, a
/// stand-in in tests.
pub type SaveFn = Arc<
    dyn Fn(&Path, &SettingsDoc, &AtomicBool) -> Result<config::settings::Saved, Vec<String>>
        + Send
        + Sync,
>;

/// How a Settings `Put` writes: the save and its timeout. A test seam, like
/// `RunContext.read_git`; the daemon always uses [`SettingsIo::default`].
#[derive(Clone)]
pub struct SettingsIo {
    pub timeout: Duration,
    pub save: SaveFn,
}

impl Default for SettingsIo {
    fn default() -> Self {
        SettingsIo {
            timeout: SETTINGS_WRITE_TIMEOUT,
            save: Arc::new(config::settings::save),
        }
    }
}

#[cfg(test)]
#[path = "live_config_tests.rs"]
mod tests;
