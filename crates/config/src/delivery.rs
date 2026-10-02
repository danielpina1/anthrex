//! The `[delivery]` table (milestone 9.2 decision 16, TT §8): how a `pr`-mode run
//! watches its stage pull requests, and the caps on the fix tasks CI and reviews may
//! add. Only tuning keys live here: the delivery mode and remote are the repository's,
//! in its profile (decision 3), so `mode` and `remote` here are reported and ignored.
//!
//! The daemon reads the table once at start, and each run freezes it at its start
//! (`RunDelivery.limits`), so a later edit cannot change a live run.
//!
//! Parsing follows milestone 6 decision 5, as `[testing]` does: each key is validated
//! on its own, an invalid value is a [`Problem`] and the field keeps its default, and an
//! unknown key is `unknown key, ignored`.

use std::fmt::Display;
use std::ops::RangeInclusive;

use super::*;

/// When a stage is synced with a moved base (decision 33).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SyncPolicy {
    /// When the lowest open stage PR reports a conflict.
    #[default]
    OnConflict,
    /// Whenever the remote base moves.
    Always,
}

/// The `[delivery]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub poll_secs: u64,
    pub poll_max_secs: u64,
    pub ci_log_max_bytes: u64,
    pub ci_fix_max: u32,
    pub review_fix_max: u32,
    pub review_batch_secs: u64,
    /// Logins counted as writers (TT §6.5).
    pub reviewers: Vec<String>,
    pub reply_to_comments: bool,
    pub sync: SyncPolicy,
    pub delete_merged_branches: bool,
    /// `[low, high]`, the planner's stage size guidance (9.1 defect 17).
    pub stage_target_lines: (u32, u32),
}

impl Default for Delivery {
    fn default() -> Self {
        Delivery {
            poll_secs: 60,
            poll_max_secs: 300,
            ci_log_max_bytes: 2_097_152,
            ci_fix_max: 2,
            review_fix_max: 3,
            review_batch_secs: 120,
            reviewers: Vec::new(),
            reply_to_comments: true,
            sync: SyncPolicy::OnConflict,
            delete_merged_branches: false,
            stage_target_lines: (300, 800),
        }
    }
}

/// Inclusive bounds of each `[delivery]` key (Interfaces "config").
pub const POLL_SECS_RANGE: RangeInclusive<u64> = 1..=3600;
pub const POLL_MAX_SECS_RANGE: RangeInclusive<u64> = 1..=3600;
pub const CI_LOG_MAX_BYTES_RANGE: RangeInclusive<u64> = 4096..=16_777_216;
pub const CI_FIX_MAX_RANGE: RangeInclusive<u32> = 0..=10;
pub const REVIEW_FIX_MAX_RANGE: RangeInclusive<u32> = 0..=10;
pub const REVIEW_BATCH_SECS_RANGE: RangeInclusive<u64> = 0..=3600;
pub const STAGE_TARGET_LINES_RANGE: RangeInclusive<u32> = 50..=5000;

/// Decision 3: the keys that belong to the profile, reported with this hint.
const PROFILE_KEYS: &[&str] = &["mode", "remote"];
const PROFILE_HINT: &str = "is set per repository in its profile (anthrex profile edit), ignored";

pub(crate) const KNOWN_DELIVERY_KEYS: &[&str] = &[
    "poll_secs",
    "poll_max_secs",
    "ci_log_max_bytes",
    "ci_fix_max",
    "review_fix_max",
    "review_batch_secs",
    "reviewers",
    "reply_to_comments",
    "sync",
    "delete_merged_branches",
    "stage_target_lines",
];

pub(crate) fn read_delivery(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("delivery") else {
        return;
    };
    let Some(t) = value.as_table() else {
        problems.push(not_a_table_problem("delivery"));
        return;
    };
    let d = &mut config.delivery;
    ranged(t, "poll_secs", &POLL_SECS_RANGE, &mut d.poll_secs, problems);
    ranged(
        t,
        "poll_max_secs",
        &POLL_MAX_SECS_RANGE,
        &mut d.poll_max_secs,
        problems,
    );
    let max_bytes = &mut d.ci_log_max_bytes;
    ranged(
        t,
        "ci_log_max_bytes",
        &CI_LOG_MAX_BYTES_RANGE,
        max_bytes,
        problems,
    );
    ranged(
        t,
        "ci_fix_max",
        &CI_FIX_MAX_RANGE,
        &mut d.ci_fix_max,
        problems,
    );
    let review_max = &mut d.review_fix_max;
    ranged(
        t,
        "review_fix_max",
        &REVIEW_FIX_MAX_RANGE,
        review_max,
        problems,
    );
    let batch = &mut d.review_batch_secs;
    ranged(
        t,
        "review_batch_secs",
        &REVIEW_BATCH_SECS_RANGE,
        batch,
        problems,
    );
    read_reviewers(t, d, problems);
    let reply = &mut d.reply_to_comments;
    read_bool_key(
        t,
        "reply_to_comments",
        "delivery.reply_to_comments",
        reply,
        problems,
    );
    read_sync(t, d, problems);
    let delete = &mut d.delete_merged_branches;
    read_bool_key(
        t,
        "delete_merged_branches",
        "delivery.delete_merged_branches",
        delete,
        problems,
    );
    read_stage_target_lines(t, d, problems);
}

/// `[delivery]`'s unknown keys: `mode` and `remote` with decision 3's hint, any other
/// with M8a's `unknown key, ignored`.
pub(crate) fn report_unknown_delivery(value: &toml::Value, problems: &mut Vec<Problem>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for key in table.keys() {
        if PROFILE_KEYS.contains(&key.as_str()) {
            problems.push(Problem {
                key: format!("delivery.{key}"),
                message: PROFILE_HINT.to_string(),
                default: "nothing".to_string(),
            });
        } else if !KNOWN_DELIVERY_KEYS.contains(&key.as_str()) {
            problems.push(unknown_key_problem(&format!("delivery.{key}")));
        }
    }
}

/// `delivery.<key>` into `field` when it is an integer in `range`; otherwise a problem
/// in milestone 6's format, and `field` keeps its value.
fn ranged<T>(
    table: &toml::Table,
    key: &str,
    range: &RangeInclusive<T>,
    field: &mut T,
    problems: &mut Vec<Problem>,
) where
    T: TryFrom<i64> + PartialOrd + Display + Copy,
{
    let Some(value) = table.get(key) else {
        return;
    };
    match value
        .as_integer()
        .and_then(|n| T::try_from(n).ok())
        .filter(|n| range.contains(n))
    {
        Some(n) => *field = n,
        None => problems.push(Problem {
            key: format!("delivery.{key}"),
            message: format!("must be between {} and {}", range.start(), range.end()),
            default: field.to_string(),
        }),
    }
}

/// A GitHub login, as decision 22 reads one (a bot's `[bot]` suffix allowed).
fn is_login(text: &str) -> bool {
    let name = text.strip_suffix("[bot]").unwrap_or(text);
    (1..=39).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn read_reviewers(table: &toml::Table, d: &mut Delivery, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("reviewers") else {
        return;
    };
    let logins: Option<Vec<String>> = value.as_array().and_then(|items| {
        items
            .iter()
            .map(|item| item.as_str().filter(|s| is_login(s)).map(str::to_string))
            .collect()
    });
    match logins {
        Some(logins) => d.reviewers = logins,
        None => problems.push(Problem {
            key: "delivery.reviewers".to_string(),
            message: "must be an array of GitHub logins".to_string(),
            default: "[]".to_string(),
        }),
    }
}

fn read_sync(table: &toml::Table, d: &mut Delivery, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("sync") else {
        return;
    };
    match value.as_str() {
        Some("on_conflict") => d.sync = SyncPolicy::OnConflict,
        Some("always") => d.sync = SyncPolicy::Always,
        _ => problems.push(Problem {
            key: "delivery.sync".to_string(),
            message: "must be on_conflict or always".to_string(),
            default: "on_conflict".to_string(),
        }),
    }
}

fn read_stage_target_lines(table: &toml::Table, d: &mut Delivery, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("stage_target_lines") else {
        return;
    };
    let int = |v: &toml::Value| {
        v.as_integer()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| STAGE_TARGET_LINES_RANGE.contains(n))
    };
    let pair = match value.as_array().map(Vec::as_slice) {
        Some([low, high]) => int(low).zip(int(high)).filter(|(l, h)| l < h),
        _ => None,
    };
    match pair {
        Some(pair) => d.stage_target_lines = pair,
        None => problems.push(Problem {
            key: "delivery.stage_target_lines".to_string(),
            message: format!(
                "must be two integers between {} and {}, the first below the second",
                STAGE_TARGET_LINES_RANGE.start(),
                STAGE_TARGET_LINES_RANGE.end()
            ),
            default: format!("[{}, {}]", d.stage_target_lines.0, d.stage_target_lines.1),
        }),
    }
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
