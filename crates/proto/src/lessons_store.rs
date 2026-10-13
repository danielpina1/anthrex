//! Milestone 9.15: the lines of `<repo_dir>/lessons.jsonl` and what they point at.
//! Task M9.15.2 added a failure signature's reference on a task record (decision 6);
//! task M9.15.3 the store's line types (decisions 2, 9 and 13).
//!
//! No type here uses `deny_unknown_fields`, so a later version can add fields and an
//! older reader still reads the line; a line of an unknown `type` fails to decode
//! alone, and the fold skips it (decision 4).

use serde::{Deserialize, Serialize};

use crate::lessons::LessonLayer;
use crate::run_info::TokenUsage;

/// The `v` every line written now carries.
pub const LESSONS_VERSION: u32 = 1;

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

/// Why a task or a run was queued for distillation (decision 9): a closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistillCause {
    TaskBlocked,
    TaskRecovered,
    ReviewBlocking,
    RunDiscarded,
    RunFailed,
}

/// One lesson as the distiller wrote it (decision 13), before the fold decides whether
/// it is new, a reconfirmation or dropped (decision 16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonDraft {
    pub layer: LessonLayer,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub files: Vec<String>,
}

/// One line of `lessons.jsonl` (decision 2), tagged `"type": "queued" | "attempt" |
/// "distilled" | "gave_up" | "verdict" | "retired" | "dismissed"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LessonLine {
    Queued(QueuedLine),
    Attempt(AttemptLine),
    Distilled(DistilledLine),
    GaveUp(GaveUpLine),
    Verdict(VerdictLine),
    Retired(RetiredLine),
    Dismissed(DismissedLine),
}

impl LessonLine {
    /// The line's unique record id (`append_once`'s key).
    pub fn record_id(&self) -> &str {
        match self {
            LessonLine::Queued(l) => &l.record_id,
            LessonLine::Attempt(l) => &l.record_id,
            LessonLine::Distilled(l) => &l.record_id,
            LessonLine::GaveUp(l) => &l.record_id,
            LessonLine::Verdict(l) => &l.record_id,
            LessonLine::Retired(l) => &l.record_id,
            LessonLine::Dismissed(l) => &l.record_id,
        }
    }

    /// When the line was written (Unix seconds).
    pub fn at(&self) -> u64 {
        match self {
            LessonLine::Queued(l) => l.at,
            LessonLine::Attempt(l) => l.at,
            LessonLine::Distilled(l) => l.at,
            LessonLine::GaveUp(l) => l.at,
            LessonLine::Verdict(l) => l.at,
            LessonLine::Retired(l) => l.at,
            LessonLine::Dismissed(l) => l.at,
        }
    }
}

/// A task or run waiting for the distiller. `record_id` is `<run>/<task>/distill`, or
/// `<run>/run/distill` for a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedLine {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub cause: DistillCause,
    pub dossier: String,
    /// The failure signatures' fingerprints the dossier names.
    #[serde(default)]
    pub fps: Vec<String>,
}

/// One distiller call for the `queued` entry: `record_id` is `<queued>/attempt/<n>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptLine {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub queued: String,
    pub n: u8,
    /// The fallback's reason when the call gave no answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// The distiller's answer, which closes the entry: `record_id` is
/// `<queued>/distilled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistilledLine {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub queued: String,
    #[serde(default)]
    pub items: Vec<LessonDraft>,
    /// Items the parser dropped alone (decision 13).
    #[serde(default)]
    pub dropped: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// The entry closed without an answer: `record_id` is `<queued>/gave_up`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GaveUpLine {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub queued: String,
    pub reason: String,
}

/// The user approved or rejected lesson `id`: `record_id` is `<id>/verdict/<at>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictLine {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub id: String,
    pub approved: bool,
}

/// Lesson `id` retired: `record_id` is `<id>/retired`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetiredLine {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub id: String,
    pub reason: String,
}

/// The known failure of fingerprint `fp` dismissed: `record_id` is
/// `kf-<fp>/dismissed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DismissedLine {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub fp: String,
}
