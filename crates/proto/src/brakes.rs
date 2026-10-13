//! Milestone 9.16 (NA §5): the brakes' wire types. The dispatch hold (decision 15),
//! the quota brake's readings and trips (decisions 2, 3, 6, 13) and the context
//! handoff's record (decision 30).

use serde::{Deserialize, Serialize};

use crate::types::Runtime;

/// The 5-hour window's length.
pub const FIVE_HOUR_SECS: u64 = 18_000;
/// The 7-day window's length.
pub const SEVEN_DAY_SECS: u64 = 604_800;

/// A usage window both runtimes report (decision 3). Any other window is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaWindow {
    FiveHour,
    SevenDay,
}

impl QuotaWindow {
    /// `5h` or `7d`.
    pub fn label(self) -> &'static str {
        match self {
            QuotaWindow::FiveHour => "5h",
            QuotaWindow::SevenDay => "7d",
        }
    }

    /// The window's length in seconds.
    pub fn secs(self) -> u64 {
        match self {
            QuotaWindow::FiveHour => FIVE_HOUR_SECS,
            QuotaWindow::SevenDay => SEVEN_DAY_SECS,
        }
    }

    /// Codex's `rate_limits.{primary,secondary}.window_minutes`: mapped by length,
    /// never by position.
    pub fn from_codex_minutes(minutes: u64) -> Option<QuotaWindow> {
        match minutes {
            300 => Some(QuotaWindow::FiveHour),
            10_080 => Some(QuotaWindow::SevenDay),
            _ => None,
        }
    }

    /// Claude's `rate_limit_info.unifiedWindows` key or `rateLimitType`.
    pub fn from_claude_key(key: &str) -> Option<QuotaWindow> {
        match key {
            "five_hour" => Some(QuotaWindow::FiveHour),
            "seven_day" => Some(QuotaWindow::SevenDay),
            _ => None,
        }
    }
}

/// Usage in per mille, 0 to 1000. Decision 2: the only unit conversion in the
/// workspace; Claude reports a fraction, Codex a percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Permille(pub u16);

impl Permille {
    pub const FULL: Permille = Permille(1000);

    /// `round(x × 1000)`, clamped to 0..=1000; `None` for a NaN, an infinity or a
    /// negative value.
    pub fn from_fraction(x: f64) -> Option<Permille> {
        Permille::scaled(x, 1000.0)
    }

    /// `round(p × 10)`, clamped to 0..=1000; `None` as `from_fraction`.
    pub fn from_percent(p: f64) -> Option<Permille> {
        Permille::scaled(p, 10.0)
    }

    fn scaled(value: f64, factor: f64) -> Option<Permille> {
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        let scaled = (value * factor).round().min(1000.0);
        // In 0.0..=1000.0 here, so the cast neither truncates nor saturates.
        Some(Permille(scaled as u16))
    }

    /// The displayed percentage: `floor(self.0 / 10)`.
    pub fn percent(self) -> u16 {
        self.0 / 10
    }

    /// Whether the usage is at or over `percent` (every threshold comparison).
    pub fn at_least(self, percent: u8) -> bool {
        self.0 >= u16::from(percent) * 10
    }
}

/// Where a runtime's quota stands (decision 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaState {
    /// No reading since the daemon started.
    Unmeasured,
    Fresh,
    /// The newest reading is older than `quota_fresh_secs`.
    Stale,
    Braked,
}

/// One window's newest reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaWindowInfo {
    pub window: QuotaWindow,
    pub used: Permille,
    #[serde(default)]
    pub resets_at: Option<u64>,
}

/// A tripped window (decision 6): dispatch on the runtime waits until `until`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrakeTripInfo {
    pub window: QuotaWindow,
    pub used: Permille,
    pub threshold_percent: u8,
    pub until: u64,
}

/// One runtime's quota.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaInfo {
    pub runtime: Runtime,
    pub state: QuotaState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<QuotaWindowInfo>,
    #[serde(default)]
    pub read_at: Option<u64>,
    #[serde(default)]
    pub trip: Option<BrakeTripInfo>,
}

/// The daemon-wide dispatch hold (decision 15). `by` is `"user"`, or
/// `"corrupt brakes.json"` / `"unreadable brakes.json"` when loading fails closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchHold {
    pub since: u64,
    pub by: String,
}

/// `RunsSnapshot.brakes`: the hold and the quota brake.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrakesInfo {
    #[serde(default)]
    pub hold: Option<DispatchHold>,
    /// `[orchestrator.brakes] quota_brake`.
    #[serde(default)]
    pub quota_brake: bool,
    /// One per runtime, Claude then Codex; left out while empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quota: Vec<QuotaInfo>,
}

impl BrakesInfo {
    /// Equal to the default: `RunsSnapshot` leaves it out.
    pub fn is_empty(&self) -> bool {
        *self == BrakesInfo::default()
    }
}

/// A run whose orchestrator started fresh at the goal boundary (decision 30).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffInfo {
    /// The previous run's short id.
    pub from_run: String,
    pub fill_tokens: u64,
    #[serde(default)]
    pub window_tokens: Option<u64>,
    pub at: u64,
}
