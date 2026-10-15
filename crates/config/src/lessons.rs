//! The `[lessons]` table (milestone 9.15 decision 31): whether lessons reach prompts,
//! whether finished tasks are distilled, whether reviewers get a risk note, and the
//! sizes and windows of the lessons store's consolidation.
//!
//! Daemon-wide: the daemon reads the table once at start. `enabled`, `distill`,
//! `risk_note`, `block_chars`, `item_chars`, `max_items` and `dossier_chars` are frozen
//! into each run at its start (pre-flight ruling C4).
//!
//! Parsing follows milestone 6 decision 5, as `[target_pool]` does: each key is
//! validated on its own, an invalid value is a [`Problem`] and the field keeps its
//! default, and an unknown key is `unknown key, ignored` (`report_unknown_nested`). A
//! config without the table gets every default.

use std::ops::RangeInclusive;

use super::*;

/// The `[lessons]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lessons {
    /// Off: no block in any prompt and nothing queued for distillation; signatures are
    /// still recorded (they are history).
    pub enabled: bool,
    pub distill: bool,
    /// Independent of `enabled`.
    pub risk_note: bool,
    pub block_chars: u32,
    pub item_chars: u32,
    pub max_items: u32,
    pub dossier_chars: u32,
    pub episodic_trial_runs: u32,
    pub known_min: u32,
    pub known_window_days: u32,
    pub known_clean_runs: u32,
    pub expiry_days: u32,
    /// 0 stops the distill worker from calling.
    pub max_distills_per_day: u32,
}

impl Default for Lessons {
    fn default() -> Self {
        Lessons {
            enabled: true,
            distill: true,
            risk_note: true,
            block_chars: 4000,
            item_chars: 600,
            max_items: 5,
            dossier_chars: 12000,
            episodic_trial_runs: 20,
            known_min: 2,
            known_window_days: 14,
            known_clean_runs: 5,
            expiry_days: 90,
            max_distills_per_day: 20,
        }
    }
}

/// Inclusive bounds of each ranged `[lessons]` key (decision 31).
pub const LESSONS_BLOCK_CHARS_RANGE: RangeInclusive<u32> = 500..=20000;
pub const LESSONS_ITEM_CHARS_RANGE: RangeInclusive<u32> = 100..=2000;
pub const LESSONS_MAX_ITEMS_RANGE: RangeInclusive<u32> = 1..=10;
pub const LESSONS_DOSSIER_CHARS_RANGE: RangeInclusive<u32> = 2000..=50000;
pub const LESSONS_EPISODIC_TRIAL_RUNS_RANGE: RangeInclusive<u32> = 1..=200;
pub const LESSONS_KNOWN_MIN_RANGE: RangeInclusive<u32> = 2..=10;
pub const LESSONS_KNOWN_WINDOW_DAYS_RANGE: RangeInclusive<u32> = 1..=365;
pub const LESSONS_KNOWN_CLEAN_RUNS_RANGE: RangeInclusive<u32> = 1..=50;
pub const LESSONS_EXPIRY_DAYS_RANGE: RangeInclusive<u32> = 1..=3650;
pub const LESSONS_MAX_DISTILLS_PER_DAY_RANGE: RangeInclusive<u32> = 0..=500;

pub(crate) const KNOWN_LESSONS_KEYS: &[&str] = &[
    "enabled",
    "distill",
    "risk_note",
    "block_chars",
    "item_chars",
    "max_items",
    "dossier_chars",
    "episodic_trial_runs",
    "known_min",
    "known_window_days",
    "known_clean_runs",
    "expiry_days",
    "max_distills_per_day",
];

pub(crate) fn read_lessons(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("lessons") else {
        return;
    };
    let Some(table) = value.as_table() else {
        problems.push(not_a_table_problem("lessons"));
        return;
    };
    let l = &mut config.lessons;
    for (key, field) in [
        ("enabled", &mut l.enabled),
        ("distill", &mut l.distill),
        ("risk_note", &mut l.risk_note),
    ] {
        read_bool(table, key, field, problems);
    }
    for (key, range, field) in [
        ("block_chars", LESSONS_BLOCK_CHARS_RANGE, &mut l.block_chars),
        ("item_chars", LESSONS_ITEM_CHARS_RANGE, &mut l.item_chars),
        ("max_items", LESSONS_MAX_ITEMS_RANGE, &mut l.max_items),
        (
            "dossier_chars",
            LESSONS_DOSSIER_CHARS_RANGE,
            &mut l.dossier_chars,
        ),
        (
            "episodic_trial_runs",
            LESSONS_EPISODIC_TRIAL_RUNS_RANGE,
            &mut l.episodic_trial_runs,
        ),
        ("known_min", LESSONS_KNOWN_MIN_RANGE, &mut l.known_min),
        (
            "known_window_days",
            LESSONS_KNOWN_WINDOW_DAYS_RANGE,
            &mut l.known_window_days,
        ),
        (
            "known_clean_runs",
            LESSONS_KNOWN_CLEAN_RUNS_RANGE,
            &mut l.known_clean_runs,
        ),
        ("expiry_days", LESSONS_EXPIRY_DAYS_RANGE, &mut l.expiry_days),
        (
            "max_distills_per_day",
            LESSONS_MAX_DISTILLS_PER_DAY_RANGE,
            &mut l.max_distills_per_day,
        ),
    ] {
        read_ranged(table, key, &range, field, problems);
    }
}

/// `lessons.<key>` into `field` when it is a bool; otherwise a problem, and `field`
/// keeps its value.
fn read_bool(table: &toml::Table, key: &str, field: &mut bool, problems: &mut Vec<Problem>) {
    let Some(value) = table.get(key) else {
        return;
    };
    match value.as_bool() {
        Some(b) => *field = b,
        None => problems.push(Problem {
            key: format!("lessons.{key}"),
            message: "must be true or false".to_string(),
            default: field.to_string(),
        }),
    }
}

/// `lessons.<key>` into `field` when it is an integer in `range`; otherwise a problem
/// in milestone 6's format, and `field` keeps its value.
fn read_ranged(
    table: &toml::Table,
    key: &str,
    range: &RangeInclusive<u32>,
    field: &mut u32,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get(key) else {
        return;
    };
    match value
        .as_integer()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| range.contains(n))
    {
        Some(n) => *field = n,
        None => problems.push(Problem {
            key: format!("lessons.{key}"),
            message: format!("must be between {} and {}", range.start(), range.end()),
            default: field.to_string(),
        }),
    }
}

#[cfg(test)]
#[path = "lessons_tests.rs"]
mod tests;
