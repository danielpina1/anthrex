//! `[orchestrator.brakes]` (milestone 9.16 decision 10): the quota brake's four
//! settings-owned keys, applied live. U11's defaults: on, pausing at 90 % of the 5-hour
//! window and 95 % of the 7-day window, a reading fresh for 600 s. Read by [`read`] from
//! `orchestrator::read`; unknown keys are reported by `orchestrator/unknown.rs`.
//!
//! As everywhere in `[orchestrator]`, an invalid value is a [`Problem`] and keeps its
//! default. The ranges are the ones `settings::validate` checks a save against.

use std::ops::RangeInclusive;

use super::read_u8_in_range;
use crate::{Problem, not_a_table_problem, read_bool_key, read_u64_in_range};

/// `quota_pause_5h_percent` and `quota_pause_7d_percent`.
pub const QUOTA_PAUSE_PERCENT_RANGE: RangeInclusive<u8> = 1..=100;
/// `quota_fresh_secs`: from a minute to a day.
pub const QUOTA_FRESH_SECS_RANGE: RangeInclusive<u64> = 60..=86_400;

pub(super) const KNOWN_BRAKES_KEYS: &[&str] = &[
    "quota_brake",
    "quota_pause_5h_percent",
    "quota_pause_7d_percent",
    "quota_fresh_secs",
];

/// `[orchestrator.brakes]`. `config::Orchestrator.brakes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrakeKeys {
    pub quota_brake: bool,
    pub pause_5h_percent: u8,
    pub pause_7d_percent: u8,
    pub fresh_secs: u64,
}

impl Default for BrakeKeys {
    fn default() -> Self {
        BrakeKeys {
            quota_brake: true,
            pause_5h_percent: 90,
            pause_7d_percent: 95,
            fresh_secs: 600,
        }
    }
}

/// Reads `[orchestrator.brakes]` out of `[orchestrator]` (`orchestrator`). Never fails.
pub(super) fn read(orchestrator: &toml::Table, problems: &mut Vec<Problem>) -> BrakeKeys {
    let mut b = BrakeKeys::default();
    let Some(value) = orchestrator.get("brakes") else {
        return b;
    };
    let Some(t) = value.as_table() else {
        problems.push(not_a_table_problem("orchestrator.brakes"));
        return b;
    };
    let key = |k: &str| format!("orchestrator.brakes.{k}");
    read_bool_key(
        t,
        "quota_brake",
        &key("quota_brake"),
        &mut b.quota_brake,
        problems,
    );
    for (k, field) in [
        ("quota_pause_5h_percent", &mut b.pause_5h_percent),
        ("quota_pause_7d_percent", &mut b.pause_7d_percent),
    ] {
        read_u8_in_range(t, k, &key(k), &QUOTA_PAUSE_PERCENT_RANGE, field, problems);
    }
    read_u64_in_range(
        t,
        "quota_fresh_secs",
        &key("quota_fresh_secs"),
        &QUOTA_FRESH_SECS_RANGE,
        &mut b.fresh_secs,
        problems,
    );
    b
}
