//! Milestone 9.16 decisions 6–8 and 10: the quota brake's book. Pure: no file, no clock,
//! no lock; every time arrives as `now` (the engine event's) and every reading as a
//! [`QuotaSample`].
//!
//! Per runtime the book keeps each window's newest reading `{used, resets_at, read_at}`
//! and the windows it tripped. A reading at or over its window's threshold trips the
//! window until its `resets_at`, or `read_at + the window's length` when it has none
//! (decision 6). That `until` is also clamped to `read_at + the window's length` (task 3's
//! review I2): a reset in milliseconds, or garbage, cannot hold the brake past the window.
//!
//! A trip clears only at `now >= until`, or on a reading of that window newer than the
//! trip's below the threshold; never because its reading went stale (decision 7, the
//! deadlock guard: while braked no session runs, so no reading arrives). With no trip the
//! runtime is allowed, also when it has no fresh reading: `Blind` (decision 8, fail open).

use std::collections::BTreeMap;

use proto::Runtime;
use proto::brakes::{BrakeTripInfo, Permille, QuotaInfo, QuotaState, QuotaWindow, QuotaWindowInfo};

use crate::headless::quota::QuotaSample;

/// The four `[orchestrator.brakes]` keys as the engine holds them. The default is **off**
/// (ruling C3): U11's default-on lives in `config`, which the driver sends as
/// `EventKind::BrakesConfig` before the first step, so a bare `EngineState` brakes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrakeConfig {
    pub quota_brake: bool,
    pub pause_5h_percent: u8,
    pub pause_7d_percent: u8,
    pub fresh_secs: u64,
}

impl Default for BrakeConfig {
    fn default() -> Self {
        BrakeConfig {
            quota_brake: false,
            pause_5h_percent: 90,
            pause_7d_percent: 95,
            fresh_secs: 600,
        }
    }
}

impl From<config::BrakeKeys> for BrakeConfig {
    fn from(keys: config::BrakeKeys) -> Self {
        BrakeConfig {
            quota_brake: keys.quota_brake,
            pause_5h_percent: keys.pause_5h_percent,
            pause_7d_percent: keys.pause_7d_percent,
            fresh_secs: keys.fresh_secs,
        }
    }
}

impl BrakeConfig {
    /// The window's pause threshold, in percent.
    pub fn threshold(&self, window: QuotaWindow) -> u8 {
        match window {
            QuotaWindow::FiveHour => self.pause_5h_percent,
            QuotaWindow::SevenDay => self.pause_7d_percent,
        }
    }
}

/// A tripped window (decision 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trip {
    pub window: QuotaWindow,
    pub used: Permille,
    pub threshold_percent: u8,
    pub until: u64,
    pub read_at: u64,
}

impl Trip {
    pub fn info(&self) -> BrakeTripInfo {
        BrakeTripInfo {
            window: self.window,
            used: self.used,
            threshold_percent: self.threshold_percent,
            until: self.until,
        }
    }
}

/// What the book says about starting a session of a runtime now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A fresh reading below every threshold, or the brake is off.
    Allowed,
    /// No trip, but no fresh reading either: allowed (fail open, decision 8).
    Blind,
    Braked(Trip),
}

/// One window's newest reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Reading {
    used: Permille,
    resets_at: Option<u64>,
    read_at: u64,
}

/// One runtime's readings and trips.
#[derive(Debug, Clone, Default, PartialEq)]
struct Book {
    readings: BTreeMap<QuotaWindow, Reading>,
    trips: BTreeMap<QuotaWindow, Trip>,
    /// The newest reading's time, of any window.
    read_at: Option<u64>,
}

/// See the module doc. In memory only: a restart starts unmeasured (decision 6).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuotaBook {
    books: BTreeMap<Runtime, Book>,
}

/// The runtimes that have a quota, in `info`'s order.
const RUNTIMES: [Runtime; 2] = [Runtime::Claude, Runtime::Codex];

impl QuotaBook {
    /// Records `sample`, read at `now`, for `runtime`, and trips or clears each window it
    /// names. A reading older than the stored one of its window is ignored; a rejected
    /// window counts as full whatever its utilization (decision 4).
    pub fn record(&mut self, runtime: Runtime, sample: &QuotaSample, now: u64, cfg: &BrakeConfig) {
        let book = self.books.entry(runtime).or_default();
        let mut windows = sample.windows.clone();
        if let Some(rejected) = sample.rejected
            && !windows.iter().any(|(w, ..)| *w == rejected)
        {
            windows.push((rejected, Permille::FULL, None));
        }
        for (window, used, resets_at) in windows {
            let used = if sample.rejected == Some(window) {
                Permille::FULL
            } else {
                used
            };
            let reading = Reading {
                used,
                resets_at,
                read_at: now,
            };
            match book.readings.get(&window) {
                Some(old) if old.read_at > now => continue,
                // The same second twice: the fuller reading stands.
                Some(old) if old.read_at == now && old.used >= used => continue,
                _ => {}
            }
            book.readings.insert(window, reading);
            book.read_at = Some(book.read_at.map_or(now, |at| at.max(now)));
            match trip(window, &reading, cfg) {
                Some(t) => {
                    book.trips.insert(window, t);
                }
                None => {
                    // Only a reading newer than the trip's releases it (decision 7).
                    if book.trips.get(&window).is_some_and(|t| now > t.read_at) {
                        book.trips.remove(&window);
                    }
                }
            }
        }
    }

    /// Re-derives every trip from the stored readings under `cfg`'s thresholds (decision
    /// 10). A trip whose `until` has passed is dropped by the next [`Self::verdict`].
    pub fn reconfigure(&mut self, cfg: &BrakeConfig) {
        for book in self.books.values_mut() {
            book.trips = (book.readings.iter())
                .filter_map(|(w, r)| trip(*w, r, cfg).map(|t| (*w, t)))
                .collect();
        }
    }

    /// The runtime's verdict at `now`; drops the trips that have expired. A runtime with
    /// no quota (a shell) is always allowed.
    pub fn verdict(&mut self, runtime: Runtime, now: u64, cfg: &BrakeConfig) -> Verdict {
        if !cfg.quota_brake || !RUNTIMES.contains(&runtime) {
            return Verdict::Allowed;
        }
        let Some(book) = self.books.get_mut(&runtime) else {
            return Verdict::Blind;
        };
        book.trips.retain(|_, t| now < t.until);
        if let Some(t) = longest(book, now) {
            return Verdict::Braked(t.clone());
        }
        if fresh(book, now, cfg) {
            Verdict::Allowed
        } else {
            Verdict::Blind
        }
    }

    /// Each runtime's quota at `now`, Claude then Codex. Read-only: an expired trip is
    /// not shown, though only [`Self::verdict`] drops it. With the brake off no trip is.
    pub fn info(&self, now: u64, cfg: &BrakeConfig) -> Vec<QuotaInfo> {
        RUNTIMES
            .iter()
            .map(|&runtime| {
                let Some(book) = self.books.get(&runtime) else {
                    return QuotaInfo {
                        runtime,
                        state: QuotaState::Unmeasured,
                        windows: Vec::new(),
                        read_at: None,
                        trip: None,
                    };
                };
                let trip = cfg
                    .quota_brake
                    .then(|| longest(book, now).map(Trip::info))
                    .flatten();
                let state = if trip.is_some() {
                    QuotaState::Braked
                } else if book.read_at.is_none() {
                    QuotaState::Unmeasured
                } else if fresh(book, now, cfg) {
                    QuotaState::Fresh
                } else {
                    QuotaState::Stale
                };
                QuotaInfo {
                    runtime,
                    state,
                    windows: (book.readings.iter())
                        .map(|(w, r)| QuotaWindowInfo {
                            window: *w,
                            used: r.used,
                            resets_at: r.resets_at,
                        })
                        .collect(),
                    read_at: book.read_at,
                    trip,
                }
            })
            .collect()
    }
}

/// The trip `reading` makes under `cfg`, if it is at or over its threshold.
fn trip(window: QuotaWindow, reading: &Reading, cfg: &BrakeConfig) -> Option<Trip> {
    let threshold = cfg.threshold(window);
    reading.used.at_least(threshold).then(|| {
        let longest = reading.read_at.saturating_add(window.secs());
        Trip {
            window,
            used: reading.used,
            threshold_percent: threshold,
            until: reading.resets_at.unwrap_or(longest).min(longest),
            read_at: reading.read_at,
        }
    })
}

/// The unexpired trip that holds longest.
fn longest(book: &Book, now: u64) -> Option<&Trip> {
    (book.trips.values())
        .filter(|t| now < t.until)
        .max_by_key(|t| (t.until, t.window))
}

/// Whether the newest reading is at most `fresh_secs` old.
fn fresh(book: &Book, now: u64, cfg: &BrakeConfig) -> bool {
    book.read_at
        .is_some_and(|at| now.saturating_sub(at) <= cfg.fresh_secs)
}
