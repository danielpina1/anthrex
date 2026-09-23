//! The `[git]` table (milestone 6): its type, its bounds, and its parsing. Split out of
//! `lib.rs` (the final review of milestone 6.5, M5) with the `[conversation]` tables, to
//! keep every file under the 600-line rule; a pure move.

use super::*;

/// The `[git]` table that git-surface spec 3.7 assigns to this milestone: "Milestone 6
/// adds a `[git]` table with `enabled`, `poll_secs`, `debounce_ms` and an `ignore` list
/// appended to the built-in filter."
///
/// Every default here is that spec's own number, not a new choice: section 3.3 sets the
/// 30-second safety poll and the 300 ms debounce, and section 3.7 leaves the subsystem on
/// unless `ANTHREX_GIT` turns it off. The *ranges* below are this milestone's, since no
/// spec gives one.
///
/// `enabled` cannot turn git back on: the daemon computes
/// `git::enabled_from_env() && config.git.enabled`, so `ANTHREX_GIT=off` always wins.
/// An escape hatch a config file could override would not be one, and the smoke script
/// and CI depend on the variable winning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Git {
    pub enabled: bool,
    pub poll_secs: u64,
    pub debounce_ms: u64,
    /// Extra path components appended to the daemon's built-in deny list
    /// (`daemon::git::watch::DENY_COMPONENTS`). Each is a single component, matched
    /// against a path's components exactly as the built-in names are.
    pub ignore: Vec<String>,
}

/// Inclusive bounds for `git.poll_secs`. Below the lower bound a repository is probed
/// often enough to be its own event storm; above the upper one the "safety" poll is no
/// longer a safety net within any session a person would notice.
pub const GIT_POLL_SECS_RANGE: std::ops::RangeInclusive<u64> = 5..=3600;
/// Inclusive bounds for `git.debounce_ms`. Below the lower bound the debounce stops
/// coalescing a burst of writes at all; above the upper one the bottom bar lags an edit
/// by longer than a person waits before believing it is broken.
pub const GIT_DEBOUNCE_MS_RANGE: std::ops::RangeInclusive<u64> = 50..=5000;
/// How many entries `git.ignore` may carry, and how long each may be. The filter runs
/// on every accepted filesystem event, so the list is bounded rather than free.
pub const GIT_IGNORE_MAX_ENTRIES: usize = 32;
pub const GIT_IGNORE_MAX_CHARS: usize = 64;

impl Default for Git {
    fn default() -> Self {
        Git {
            enabled: true,
            poll_secs: 30,
            debounce_ms: 300,
            ignore: Vec::new(),
        }
    }
}

pub(super) const KNOWN_GIT_KEYS: &[&str] = &["enabled", "poll_secs", "debounce_ms", "ignore"];

/// One `git.ignore` entry's rule: a single path component, so it can be compared against
/// a path's components the way `DENY_COMPONENTS` is. Returns why it was rejected.
fn ignore_entry_problem(entry: &str) -> Option<&'static str> {
    if entry.is_empty() {
        return Some("must not be empty");
    }
    if entry.chars().count() > GIT_IGNORE_MAX_CHARS {
        return Some("must be at most 64 characters");
    }
    if entry.contains('/') {
        return Some("must be a single path component, with no /");
    }
    if entry == "." || entry == ".." {
        return Some("must not be . or ..");
    }
    None
}

/// Reads `git.ignore`. A bad entry costs that entry alone, never the whole list and
/// never the file: the list is a filter, and dropping every name because one was
/// mistyped would silently widen what the watcher accepts.
fn read_ignore(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("ignore") else {
        return;
    };
    let Some(array) = value.as_array() else {
        problems.push(Problem {
            key: "git.ignore".to_string(),
            message: "expected an array of strings".to_string(),
            default: "no extra ignores".to_string(),
        });
        return;
    };

    let mut ignore = Vec::new();
    for entry in array {
        if ignore.len() == GIT_IGNORE_MAX_ENTRIES {
            problems.push(Problem {
                key: "git.ignore".to_string(),
                message: format!("at most {GIT_IGNORE_MAX_ENTRIES} entries; the rest are ignored"),
                default: format!("the first {GIT_IGNORE_MAX_ENTRIES}"),
            });
            break;
        }
        let Some(text) = entry.as_str() else {
            problems.push(Problem {
                key: "git.ignore".to_string(),
                message: "expected a string".to_string(),
                default: "entry dropped".to_string(),
            });
            continue;
        };
        match ignore_entry_problem(text) {
            Some(message) => problems.push(Problem {
                key: "git.ignore".to_string(),
                message: format!("{text:?}: {message}"),
                default: "entry dropped".to_string(),
            }),
            None => ignore.push(text.to_string()),
        }
    }
    config.git.ignore = ignore;
}

pub(super) fn read_git(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("git") else {
        return;
    };
    let Some(git) = value.as_table() else {
        problems.push(not_a_table_problem("git"));
        return;
    };

    read_bool_key(
        git,
        "enabled",
        "git.enabled",
        &mut config.git.enabled,
        problems,
    );
    read_u64_in_range(
        git,
        "poll_secs",
        "git.poll_secs",
        &GIT_POLL_SECS_RANGE,
        &mut config.git.poll_secs,
        problems,
    );
    read_u64_in_range(
        git,
        "debounce_ms",
        "git.debounce_ms",
        &GIT_DEBOUNCE_MS_RANGE,
        &mut config.git.debounce_ms,
        problems,
    );
    read_ignore(git, config, problems);
}
