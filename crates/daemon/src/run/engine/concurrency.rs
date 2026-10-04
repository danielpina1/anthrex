//! Milestone 9.5 decision 16 (rulings RC-1, RC-3, RR-9): per-runtime writer caps inside
//! a run. A rate-limit event in a task session halves its runtime's cap, never below
//! one and at most once per `halve_hold_secs`; every quiet `recover_after_secs` adds one
//! back, up to `max_writers`. A task takes a writer slot only when both the run's
//! `max_writers` and its runtime's cap allow it; a lower cap stops nothing. With
//! `adaptive_concurrency` off (and on every run recorded before 9.5) nothing here
//! changes a run. Pure (design decision 1).

use std::collections::BTreeMap;

use proto::Runtime;

use super::requests::log;
use super::schedule::writer_slots;
use crate::run::model::{Run, RuntimeConcurrency};

/// `runtime`'s writer cap: `max_writers` until a rate limit lowered it.
pub fn cap(run: &Run, runtime: Runtime) -> u8 {
    let max = run.limits.max_writers;
    if !run.limits.adaptive_concurrency {
        return max;
    }
    run.concurrency
        .get(runtime.label())
        .map_or(max, |c| c.cap.min(max))
}

/// The writer slots held on `runtime`, lanes counted.
pub fn writers_busy_on(run: &Run, runtime: Runtime) -> u8 {
    let n = (run.tasks.iter())
        .flat_map(writer_slots)
        .filter(|r| *r == runtime)
        .count();
    u8::try_from(n).unwrap_or(u8::MAX)
}

/// Whether a task on `runtime` may take a writer slot as far as its cap goes.
pub(super) fn has_room(run: &Run, runtime: Runtime) -> bool {
    writers_busy_on(run, runtime) < cap(run, runtime)
}

/// A rate-limit event on `runtime` (`signals::count_rate_limit`, the one place that
/// counts `Run.rate_limits`). Returns whether the cap changed; a change is logged.
pub fn on_rate_limit(run: &mut Run, runtime: Runtime, now: u64) -> bool {
    if !run.limits.adaptive_concurrency {
        return false;
    }
    let (max, hold) = (run.limits.max_writers, run.limits.halve_hold_secs);
    let entry = (run.concurrency)
        .entry(runtime.label().to_string())
        .or_insert_with(|| RuntimeConcurrency::new(max));
    entry.last_rate_limit_at = Some(now);
    let held = entry
        .last_change_at
        .is_some_and(|at| now.saturating_sub(at) < hold);
    let from = entry.cap.min(max);
    let to = (from / 2).max(1);
    if held || to == from {
        return false;
    }
    entry.cap = to;
    entry.last_change_at = Some(now);
    entry.halvings = entry.halvings.saturating_add(1);
    log(
        run,
        now,
        format!("rate limit on {}: writers {from} → {to}", runtime.label()),
    );
    true
}

/// The scheduler's tick: each capped runtime quiet for `recover_after_secs` gets one
/// writer back. Returns whether a cap changed; a tick that changes none changes nothing.
pub fn on_tick(run: &mut Run, now: u64) -> bool {
    if !run.limits.adaptive_concurrency || run.state.is_terminal() {
        return false;
    }
    let (max, after) = (run.limits.max_writers, run.limits.recover_after_secs);
    let mut lines = Vec::new();
    for (label, c) in run.concurrency.iter_mut() {
        let quiet_since = c.last_rate_limit_at.max(c.last_change_at).unwrap_or(0);
        if c.cap >= max || now.saturating_sub(quiet_since) < after {
            continue;
        }
        c.cap += 1;
        c.last_change_at = Some(now);
        c.recoveries = c.recoveries.saturating_add(1);
        lines.push(format!(
            "no rate limit on {label} for {} min: writers {} → {}",
            after / 60,
            c.cap - 1,
            c.cap
        ));
    }
    let changed = !lines.is_empty();
    for line in lines {
        log(run, now, line);
    }
    changed
}

/// `RunInfo.writer_caps`: each runtime whose cap is below `max_writers`.
pub fn writer_caps(run: &Run) -> BTreeMap<String, u8> {
    let max = run.limits.max_writers;
    if !run.limits.adaptive_concurrency {
        return BTreeMap::new();
    }
    (run.concurrency.iter())
        .filter(|(_, c)| c.cap < max)
        .map(|(label, c)| (label.clone(), c.cap))
        .collect()
}

/// Ruling RC-3: the run's attention line for each capped runtime, while the run can
/// still dispatch (no new alert kind).
pub fn attention(run: &Run) -> Vec<String> {
    if run.state.is_terminal() {
        return Vec::new();
    }
    let max = run.limits.max_writers;
    (writer_caps(run).into_iter())
        .map(|(label, cap)| format!("{label} writers capped at {cap} of {max} after a rate limit"))
        .collect()
}

/// The report's `## Concurrency` section: every runtime a rate limit reached.
pub fn report_section(run: &Run, out: &mut String) {
    if run.concurrency.is_empty() {
        return;
    }
    let plural = |n: u32, one: &str, many: &str| match n {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    };
    out.push_str("\n## Concurrency\n\n");
    for (label, c) in &run.concurrency {
        let events = run.rate_limits.get(label).copied().unwrap_or(0);
        out.push_str(&format!(
            "- {label}: writers {} of {} ({}, {}, {})\n",
            c.cap.min(run.limits.max_writers),
            run.limits.max_writers,
            plural(events, "rate limit", "rate limits"),
            plural(c.halvings, "halving", "halvings"),
            plural(c.recoveries, "recovery", "recoveries"),
        ));
    }
}
