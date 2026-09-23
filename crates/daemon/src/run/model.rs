//! Pure engine model types. No `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` — design decision 2.
//!
//! This task (M8a.4) adds only [`ReviewLevel`], because `roster::pick_reviewer` needs it
//! before the rest of the model (`Profile`, `RunLimits`, `Task`, `Run`, …) arrives in
//! M8a.5 (refresh note C41: `ReviewLevel` was used in M8a.4 but would otherwise be
//! created only in M8a.5).

use serde::{Deserialize, Serialize};

/// How thoroughly a task is reviewed, decision 35: `S` tasks get `Small`, `M` tasks
/// `Medium`, hub tasks `Frontier`, each possibly raised by the level rule (no `check` in
/// the profile, or a non-`tdd` task whose `owns` touch `source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewLevel {
    Small,
    Medium,
    Frontier,
}
