//! Milestone 9.15: the lines of `<repo_dir>/lessons.jsonl` and what they point at.
//! Task M9.15.2 adds only a failure signature's reference on a task record
//! (decision 6); the store's line types follow in M9.15.3.

use serde::{Deserialize, Serialize};

/// The gate whose failed output a signature was taken from (decision 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SigGate {
    /// A failed check that is no tier job.
    Check,
    Tier1,
    Tier2,
    /// A failed proof (pre-flight ruling C21: the history tally's definition).
    Proof,
}

/// One failure signature on a task record (decision 6): its gate, its fingerprint (the
/// first 16 hex characters of a SHA-256) and its headline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SigRef {
    pub gate: SigGate,
    pub fp: String,
    pub headline: String,
}
