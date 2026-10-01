//! Milestone 9.2's delivery model (design decision 1: `run/delivery/` is pure). Task
//! M9.2.3 starts it with what a run freezes at its start, `RunDelivery.mode` and
//! `RunDelivery.limits` (decisions 3 and 16); task M9.2.6 adds the rest of the model
//! (the repository, the stages' pull requests, the watermarks, CI and threads).

use serde::{Deserialize, Serialize};

use proto::DeliveryMode;

/// A run's delivery, frozen at its start; `#[serde(default)]` on `Run`, so a 9.1
/// `run.json` loads `local` with the default limits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunDelivery {
    /// Decision 3: the resolved mode (the profile's, or `--delivery`).
    pub mode: DeliveryMode,
    /// Decision 16: `[delivery]`, frozen.
    pub limits: DeliveryLimits,
}

/// Decision 33's `[delivery] sync`, as a run records it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPolicy {
    #[default]
    OnConflict,
    Always,
}

impl From<config::SyncPolicy> for SyncPolicy {
    fn from(sync: config::SyncPolicy) -> Self {
        match sync {
            config::SyncPolicy::OnConflict => SyncPolicy::OnConflict,
            config::SyncPolicy::Always => SyncPolicy::Always,
        }
    }
}

/// Decision 16: the `[delivery]` keys a run is frozen with at start, so a later config
/// edit never changes a live run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeliveryLimits {
    pub poll_secs: u64,
    pub poll_max_secs: u64,
    pub ci_log_max_bytes: u64,
    pub ci_fix_max: u32,
    pub review_fix_max: u32,
    pub review_batch_secs: u64,
    pub reviewers: Vec<String>,
    pub reply_to_comments: bool,
    pub sync: SyncPolicy,
    pub delete_merged_branches: bool,
    pub stage_target_lines: (u32, u32),
}

impl From<&config::Delivery> for DeliveryLimits {
    fn from(d: &config::Delivery) -> Self {
        DeliveryLimits {
            poll_secs: d.poll_secs,
            poll_max_secs: d.poll_max_secs,
            ci_log_max_bytes: d.ci_log_max_bytes,
            ci_fix_max: d.ci_fix_max,
            review_fix_max: d.review_fix_max,
            review_batch_secs: d.review_batch_secs,
            reviewers: d.reviewers.clone(),
            reply_to_comments: d.reply_to_comments,
            sync: d.sync.into(),
            delete_merged_branches: d.delete_merged_branches,
            stage_target_lines: d.stage_target_lines,
        }
    }
}

impl Default for DeliveryLimits {
    fn default() -> Self {
        (&config::Delivery::default()).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `run.json`'s spelling of `[delivery] sync` is the config's: `on_conflict` and
    /// `always` (M9.2.3's review fix 2).
    #[test]
    fn sync_policy_is_persisted_in_snake_case() {
        for (sync, text) in [
            (SyncPolicy::OnConflict, "on_conflict"),
            (SyncPolicy::Always, "always"),
        ] {
            let limits = DeliveryLimits {
                sync,
                ..DeliveryLimits::default()
            };
            let json = serde_json::to_value(&limits).unwrap();
            assert_eq!(json["sync"], serde_json::json!(text));
            let back: DeliveryLimits =
                serde_json::from_value(serde_json::json!({ "sync": text })).unwrap();
            assert_eq!(back.sync, sync);
        }
    }
}
