//! The context handoff's three `[orchestrator]` keys (milestone 9.16 decisions 23 and 24,
//! U13): read at start, not settings-owned. `context_window_tokens` overrides the window
//! (0 = automatic); the handoff fires at whichever comes first of `handoff_at_percent` of
//! the window and `handoff_at_tokens`, each 0 disabling its own criterion.

use std::ops::RangeInclusive;

use super::{Orchestrator, read_u8_in_range};
use crate::{Problem, read_u64_in_range};

/// `handoff_at_percent`: 0 disables the percentage criterion.
pub const HANDOFF_AT_PERCENT_RANGE: RangeInclusive<u8> = 0..=100;
/// `handoff_at_tokens`: 0 disables the token cap.
pub const HANDOFF_AT_TOKENS_RANGE: RangeInclusive<u64> = 0..=10_000_000;
/// `context_window_tokens`: 0 is automatic; the brief names no bound, so the token cap's.
pub const CONTEXT_WINDOW_TOKENS_RANGE: RangeInclusive<u64> = 0..=10_000_000;

pub(super) const KNOWN_HANDOFF_KEYS: [&str; 3] = [
    "context_window_tokens",
    "handoff_at_percent",
    "handoff_at_tokens",
];

pub(super) const DEFAULT_HANDOFF_AT_PERCENT: u8 = 80;
pub(super) const DEFAULT_HANDOFF_AT_TOKENS: u64 = 300_000;

/// Reads the three keys out of `[orchestrator]` (`t`) into `o`. Never fails.
pub(super) fn read(t: &toml::Table, o: &mut Orchestrator, problems: &mut Vec<Problem>) {
    read_u64_in_range(
        t,
        "context_window_tokens",
        "orchestrator.context_window_tokens",
        &CONTEXT_WINDOW_TOKENS_RANGE,
        &mut o.context_window_tokens,
        problems,
    );
    read_u8_in_range(
        t,
        "handoff_at_percent",
        "orchestrator.handoff_at_percent",
        &HANDOFF_AT_PERCENT_RANGE,
        &mut o.handoff_at_percent,
        problems,
    );
    read_u64_in_range(
        t,
        "handoff_at_tokens",
        "orchestrator.handoff_at_tokens",
        &HANDOFF_AT_TOKENS_RANGE,
        &mut o.handoff_at_tokens,
        problems,
    );
}
