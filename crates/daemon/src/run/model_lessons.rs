//! Milestone 9.15 decisions 21 and 31: the lessons a run froze at its start, and the
//! `[lessons]` keys it froze with (ruling C4: seven of them). Pure data.

use proto::{LessonLayer, LessonStatus};
use serde::{Deserialize, Serialize};

/// What a run's start froze (decision 21). Ruling C8: `frozen_at` is the start's
/// `now`, the time selection scores recency against, so the worker's and the
/// reviewer's blocks are pure functions of the run and agree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLessons {
    pub limits: LessonLimits,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<LessonBrief>,
    pub frozen_at: u64,
}

/// The `[lessons]` keys a run is frozen with (decision 31, ruling C4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonLimits {
    pub enabled: bool,
    pub distill: bool,
    pub risk_note: bool,
    pub block_chars: u32,
    pub item_chars: u32,
    pub max_items: u32,
    pub dossier_chars: u32,
}

impl From<&config::Lessons> for LessonLimits {
    fn from(cfg: &config::Lessons) -> Self {
        LessonLimits {
            enabled: cfg.enabled,
            distill: cfg.distill,
            risk_note: cfg.risk_note,
            block_chars: cfg.block_chars,
            item_chars: cfg.item_chars,
            max_items: cfg.max_items,
            dossier_chars: cfg.dossier_chars,
        }
    }
}

impl Default for LessonLimits {
    /// `config::Lessons::default()`'s values.
    fn default() -> Self {
        (&config::Lessons::default()).into()
    }
}

/// One active lesson as the start froze it (decision 21).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonBrief {
    pub id: String,
    pub layer: LessonLayer,
    pub status: LessonStatus,
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    pub last_seen: u64,
    /// Per mille; `None` when unmeasured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usefulness: Option<u16>,
}
