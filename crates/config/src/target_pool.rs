//! The `[target_pool]` table (milestone 9.14 decisions 1 and 2): whether agents'
//! checkouts lease cargo target directories from a per-repository pool, its sizes, the
//! size cap of each directory, how long a free directory may sit idle, and the
//! repositories that opt out.
//!
//! Daemon-wide: the daemon reads the table once at start, as it reads `test_slots`; a
//! later edit (or a live `[orchestrator] max_writers` change) does not resize a pool.
//!
//! Parsing follows milestone 6 decision 5, as [`crate::parse`] does for every other
//! table: each key is validated on its own, an invalid value is a [`Problem`] and the
//! field keeps its default, and an unknown key is `unknown key, ignored`. A config
//! without the table gets every default.

use std::fmt::Display;
use std::ops::RangeInclusive;

use super::*;

/// The `[target_pool]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetPool {
    pub enabled: bool,
    /// `None`: `[orchestrator] max_writers + 2`, clamped (see [`TargetPool::task_dirs_for`]).
    pub task_dirs: Option<u16>,
    pub engine_dirs: u16,
    /// Each directory's size cap, in GB, enforced at release.
    pub max_gb: u32,
    pub idle_days: u32,
    /// Absolute repository roots the pool never applies to.
    pub disabled_repos: Vec<String>,
}

impl Default for TargetPool {
    fn default() -> Self {
        TargetPool {
            enabled: true,
            task_dirs: None,
            engine_dirs: 2,
            max_gb: 20,
            idle_days: 7,
            disabled_repos: Vec::new(),
        }
    }
}

/// Inclusive bounds of each `[target_pool]` key (Interfaces "config").
pub const TARGET_POOL_TASK_DIRS_RANGE: RangeInclusive<u16> = 1..=64;
pub const TARGET_POOL_ENGINE_DIRS_RANGE: RangeInclusive<u16> = 1..=16;
pub const TARGET_POOL_MAX_GB_RANGE: RangeInclusive<u32> = 1..=1024;
pub const TARGET_POOL_IDLE_DAYS_RANGE: RangeInclusive<u32> = 1..=365;

/// What an absent `task_dirs` means, as a problem's `(using …)` text.
const TASK_DIRS_DEFAULT_TEXT: &str = "[orchestrator] max_writers + 2";

impl TargetPool {
    /// `task_dirs`, or `max_writers + 2` clamped to `TARGET_POOL_TASK_DIRS_RANGE`
    /// (decision 2, C26). `max_writers` is at most 8 today (`MAX_WRITERS_RANGE`), so the
    /// clamp is a guard only.
    pub fn task_dirs_for(&self, max_writers: u8) -> u16 {
        self.task_dirs.unwrap_or_else(|| {
            (u16::from(max_writers) + 2).clamp(
                *TARGET_POOL_TASK_DIRS_RANGE.start(),
                *TARGET_POOL_TASK_DIRS_RANGE.end(),
            )
        })
    }
}

pub(crate) const KNOWN_TARGET_POOL_KEYS: &[&str] = &[
    "enabled",
    "task_dirs",
    "engine_dirs",
    "max_gb",
    "idle_days",
    "disabled_repos",
];

pub(crate) fn read_target_pool(
    table: &toml::Table,
    config: &mut Config,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get("target_pool") else {
        return;
    };
    let Some(pool) = value.as_table() else {
        problems.push(not_a_table_problem("target_pool"));
        return;
    };
    let p = &mut config.target_pool;
    if let Some(enabled) = pool.get("enabled") {
        match enabled.as_bool() {
            Some(b) => p.enabled = b,
            None => problems.push(Problem {
                key: "target_pool.enabled".to_string(),
                message: "must be true or false".to_string(),
                default: p.enabled.to_string(),
            }),
        }
    }
    if let Some(n) = ranged(
        pool,
        "task_dirs",
        &TARGET_POOL_TASK_DIRS_RANGE,
        TASK_DIRS_DEFAULT_TEXT,
        problems,
    ) {
        p.task_dirs = Some(n);
    }
    read_into(
        pool,
        "engine_dirs",
        &TARGET_POOL_ENGINE_DIRS_RANGE,
        &mut p.engine_dirs,
        problems,
    );
    read_into(
        pool,
        "max_gb",
        &TARGET_POOL_MAX_GB_RANGE,
        &mut p.max_gb,
        problems,
    );
    read_into(
        pool,
        "idle_days",
        &TARGET_POOL_IDLE_DAYS_RANGE,
        &mut p.idle_days,
        problems,
    );
    read_disabled_repos(pool, p, problems);
}

/// Reads `target_pool.disabled_repos`. A bad entry costs that entry alone: dropping the
/// whole list because one path was relative would silently turn the pool back on for
/// every repository the user opted out.
fn read_disabled_repos(table: &toml::Table, pool: &mut TargetPool, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("disabled_repos") else {
        return;
    };
    let key = "target_pool.disabled_repos";
    let Some(array) = value.as_array() else {
        problems.push(Problem {
            key: key.to_string(),
            message: "must be a list of absolute paths".to_string(),
            default: "no disabled repositories".to_string(),
        });
        return;
    };
    let mut repos = Vec::new();
    for entry in array {
        match entry.as_str() {
            Some(text) if Path::new(text).is_absolute() => repos.push(text.to_string()),
            other => problems.push(Problem {
                key: key.to_string(),
                message: match other {
                    Some(text) => format!("{text:?}: must be a list of absolute paths"),
                    None => format!("{entry}: must be a list of absolute paths"),
                },
                default: crate::ENTRY_SKIPPED.to_string(),
            }),
        }
    }
    pool.disabled_repos = repos;
}

/// Reads `target_pool.<key>` into `field` when it is an integer in `range`; otherwise a
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

/// `target_pool.<key>` when present and an integer in `range`. A value of the wrong
/// type, out of range, or too large for the field is one problem in milestone 6's
/// format, with `default` as its `(using …)` text.
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
            key: format!("target_pool.{key}"),
            message: format!("must be between {} and {}", range.start(), range.end()),
            default: default.to_string(),
        });
    }
    n
}

#[cfg(test)]
#[path = "target_pool_tests.rs"]
mod tests;
