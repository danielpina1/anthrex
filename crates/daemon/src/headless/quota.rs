//! Milestone 9.16 decisions 2–4: a runtime's quota reading. Pure.
//!
//! Claude's `rate_limit_event` carries `rate_limit_info`, whose `unifiedWindows.five_hour` and
//! `.seven_day` are `{utilization, resetsAt}` with `utilization` a **fraction** (2.1.278 and
//! 2.1.280 alike). An `allowed_warning` line also repeats one window at the top level
//! (`utilization`, `rateLimitType`, `resetsAt`), read when `unifiedWindows` lacks it. Codex's
//! `rate_limits.primary` and `.secondary` are `{used_percent, window_minutes, resets_at}` with
//! `used_percent` a **percentage**, mapped by `window_minutes`, never by position (the
//! checked-in rollouts have the weekly window as `primary`).
//!
//! Both units go through `proto::brakes::Permille`'s two constructors, the only conversion in
//! the workspace. A window whose usage is missing, not a number, negative or (Claude) above
//! 1.5 is dropped, and a line with no usable window is no reading (fail open, decision 8):
//! never a wrong number.

use proto::brakes::{Permille, QuotaWindow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One reading of a runtime's windows. The reading's time is the engine event's `now`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaSample {
    /// `(window, used, resets_at)`, five-hour first, each window at most once.
    pub windows: Vec<(QuotaWindow, Permille, Option<u64>)>,
    /// The window a `status: "rejected"` line names (`rateLimitType`); it counts as full
    /// whatever its utilization (decision 4).
    pub rejected: Option<QuotaWindow>,
}

/// The windows in the order a sample lists them.
const WINDOWS: [QuotaWindow; 2] = [QuotaWindow::FiveHour, QuotaWindow::SevenDay];

/// A Claude `rate_limit_event`'s `rate_limit_info`, or `None` when it has no usable window.
pub fn claude_rate_limit(info: &Value) -> Option<QuotaSample> {
    let info = info.as_object()?;
    let top_window = info
        .get("rateLimitType")
        .and_then(Value::as_str)
        .and_then(QuotaWindow::from_claude_key);
    let top_resets = info.get("resetsAt").and_then(Value::as_u64);
    let unified = info.get("unifiedWindows").and_then(Value::as_object);
    let mut windows = Vec::new();
    for window in WINDOWS {
        let key = match window {
            QuotaWindow::FiveHour => "five_hour",
            QuotaWindow::SevenDay => "seven_day",
        };
        let reading = unified
            .and_then(|u| u.get(key))
            .and_then(|w| {
                let used = fraction(w.get("utilization")?)?;
                Some((used, w.get("resetsAt").and_then(Value::as_u64)))
            })
            .or_else(|| {
                // The warning's top-level copy of the window it is about.
                (top_window == Some(window))
                    .then(|| fraction(info.get("utilization")?))
                    .flatten()
                    .map(|used| (used, top_resets))
            });
        if let Some((used, resets_at)) = reading {
            windows.push((window, used, resets_at));
        }
    }
    let rejected = (info.get("status").and_then(Value::as_str) == Some("rejected"))
        .then_some(top_window)
        .flatten();
    if let Some(window) = rejected
        && !windows.iter().any(|(w, ..)| *w == window)
    {
        // A rejection is a reading on its own: that window is full until the line's reset.
        let at = WINDOWS.iter().position(|w| *w == window).unwrap_or(0);
        windows.insert(at.min(windows.len()), (window, Permille::FULL, top_resets));
    }
    (!windows.is_empty()).then_some(QuotaSample { windows, rejected })
}

/// Review I1: the largest `utilization` read as a fraction. Above it the value is no
/// fraction (a percent, say `5` for 5 %), so the window is dropped (fail open) instead of
/// clamped to full; (1.0, 1.5] is an overage and clamps to 1000 ‰ (decision 2).
const MAX_FRACTION: f64 = 1.5;

/// A Claude `utilization`, or `None` when it is not a plausible fraction.
fn fraction(value: &Value) -> Option<Permille> {
    value
        .as_f64()
        .filter(|x| *x <= MAX_FRACTION)
        .and_then(Permille::from_fraction)
}

/// A Codex `token_count` payload's `rate_limits`, or `None` when it has no usable window.
pub fn codex_rate_limits(limits: &Value) -> Option<QuotaSample> {
    let limits = limits.as_object()?;
    let read: Vec<(QuotaWindow, Permille, Option<u64>)> = ["primary", "secondary"]
        .into_iter()
        .filter_map(|slot| {
            let w = limits.get(slot)?.as_object()?;
            let window = QuotaWindow::from_codex_minutes(w.get("window_minutes")?.as_u64()?)?;
            let used = Permille::from_percent(w.get("used_percent")?.as_f64()?)?;
            Some((window, used, w.get("resets_at").and_then(Value::as_u64)))
        })
        .collect();
    let windows: Vec<_> = WINDOWS
        .iter()
        .filter_map(|want| read.iter().find(|(w, ..)| w == want).copied())
        .collect();
    (!windows.is_empty()).then_some(QuotaSample {
        windows,
        rejected: None,
    })
}

#[cfg(test)]
#[path = "quota_tests.rs"]
mod tests;
