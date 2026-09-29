//! The `[testing]` table (milestone 9.1 decision 3, TT §8): the test scheduler's slot
//! count, the result cache's lifetime, and the rules of tier 3, flaky tests and bisect.
//! A top-level table, not part of `[orchestrator]`, because the scheduler serves every
//! run rather than one orchestrator.
//!
//! `test_slots` and `test_cache_days` are daemon-wide: the daemon reads them once at
//! start. `full_idle_secs`, `bisect_fix_max`, `flaky_quarantine_after` and
//! `flaky_window_days` are frozen into each run at its start (`RunLimits.testing`), so a
//! later edit cannot change a live run's rules.
//!
//! Parsing follows milestone 6 decision 5, as [`crate::parse`] does for every other
//! table: each key is validated on its own, an invalid value is a [`Problem`] and the
//! field keeps its default, and an unknown key is `unknown key, ignored`. A config
//! without the table gets every default.

use std::fmt::Display;
use std::ops::RangeInclusive;

use super::*;

/// The `[testing]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Testing {
    /// `None`: the number of logical cores minus 2, at least 1 (decided by the daemon,
    /// which knows the machine).
    pub test_slots: Option<u32>,
    pub full_idle_secs: u64,
    /// 0 turns the result cache off.
    pub test_cache_days: u32,
    pub flaky_quarantine_after: u32,
    pub flaky_window_days: u32,
    pub bisect_fix_max: u8,
}

impl Default for Testing {
    fn default() -> Self {
        Testing {
            test_slots: None,
            full_idle_secs: 120,
            test_cache_days: 14,
            flaky_quarantine_after: 3,
            flaky_window_days: 14,
            bisect_fix_max: 2,
        }
    }
}

/// Inclusive bounds of each `[testing]` key (Interfaces "config").
pub const TEST_SLOTS_RANGE: RangeInclusive<u32> = 1..=256;
pub const FULL_IDLE_SECS_RANGE: RangeInclusive<u64> = 10..=86_400;
pub const TEST_CACHE_DAYS_RANGE: RangeInclusive<u32> = 0..=365;
pub const FLAKY_QUARANTINE_AFTER_RANGE: RangeInclusive<u32> = 1..=100;
pub const FLAKY_WINDOW_DAYS_RANGE: RangeInclusive<u32> = 1..=365;
pub const BISECT_FIX_MAX_RANGE: RangeInclusive<u8> = 0..=10;

/// What an absent `test_slots` means, as a problem's `(using …)` text.
const TEST_SLOTS_DEFAULT_TEXT: &str = "logical cores minus 2, at least 1";

pub(crate) const KNOWN_TESTING_KEYS: &[&str] = &[
    "test_slots",
    "full_idle_secs",
    "test_cache_days",
    "flaky_quarantine_after",
    "flaky_window_days",
    "bisect_fix_max",
];

pub(crate) fn read_testing(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("testing") else {
        return;
    };
    let Some(testing) = value.as_table() else {
        problems.push(not_a_table_problem("testing"));
        return;
    };
    let t = &mut config.testing;
    if let Some(n) = ranged(
        testing,
        "test_slots",
        &TEST_SLOTS_RANGE,
        TEST_SLOTS_DEFAULT_TEXT,
        problems,
    ) {
        t.test_slots = Some(n);
    }
    read_into(
        testing,
        "full_idle_secs",
        &FULL_IDLE_SECS_RANGE,
        &mut t.full_idle_secs,
        problems,
    );
    read_into(
        testing,
        "test_cache_days",
        &TEST_CACHE_DAYS_RANGE,
        &mut t.test_cache_days,
        problems,
    );
    read_into(
        testing,
        "flaky_quarantine_after",
        &FLAKY_QUARANTINE_AFTER_RANGE,
        &mut t.flaky_quarantine_after,
        problems,
    );
    read_into(
        testing,
        "flaky_window_days",
        &FLAKY_WINDOW_DAYS_RANGE,
        &mut t.flaky_window_days,
        problems,
    );
    read_into(
        testing,
        "bisect_fix_max",
        &BISECT_FIX_MAX_RANGE,
        &mut t.bisect_fix_max,
        problems,
    );
}

/// Reads `testing.<key>` into `field` when it is an integer in `range`; otherwise a
/// problem, and `field` keeps its value.
fn read_into<T>(
    table: &toml::Table,
    key: &str,
    range: &RangeInclusive<T>,
    field: &mut T,
    problems: &mut Vec<Problem>,
) where
    T: TryFrom<i64> + PartialOrd + Display + Copy,
{
    if let Some(n) = ranged(table, key, range, &field.to_string(), problems) {
        *field = n;
    }
}

/// `testing.<key>` when present and an integer in `range`. A value of the wrong type,
/// out of range, or too large for the field is one problem in milestone 6's format,
/// with `default` as its `(using …)` text.
fn ranged<T>(
    table: &toml::Table,
    key: &str,
    range: &RangeInclusive<T>,
    default: &str,
    problems: &mut Vec<Problem>,
) -> Option<T>
where
    T: TryFrom<i64> + PartialOrd + Display + Copy,
{
    let value = table.get(key)?;
    let n = value
        .as_integer()
        .and_then(|n| T::try_from(n).ok())
        .filter(|n| range.contains(n));
    if n.is_none() {
        problems.push(Problem {
            key: format!("testing.{key}"),
            message: format!("must be between {} and {}", range.start(), range.end()),
            default: default.to_string(),
        });
    }
    n
}

#[cfg(test)]
#[path = "testing_tests.rs"]
mod tests;
