//! The labels a run's triage prints, in the daemon's reports and the client's inspector
//! (decision 10), and the words `anthrex run stats` and the TUI's stats page share
//! (milestone 9.19 decision 14). Pure.

use proto::{DeciderSource, Scale, TaskKind};

/// `code`, `docs`, `research` or `review`.
pub fn kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Code => "code",
        TaskKind::Docs => "docs",
        TaskKind::Research => "research",
        TaskKind::Review => "review",
    }
}

pub fn scale_label(scale: Scale) -> &'static str {
    match scale {
        Scale::Single => "single",
        Scale::Plan => "plan",
        Scale::Large => "large",
    }
}

pub fn source_label(source: DeciderSource) -> &'static str {
    match source {
        DeciderSource::Decider => "decider",
        DeciderSource::Fallback => "fallback",
    }
}

/// `<kinds joined with ,>/<scale>`, as `code/single`.
pub fn kinds_scale(kinds: &[TaskKind], scale: Scale) -> String {
    let kinds: Vec<&str> = kinds.iter().map(|k| kind_label(*k)).collect();
    format!("{}/{}", kinds.join(","), scale_label(scale))
}

/// Milestone 9.19 decision 14: `anthrex run stats`' column headers; the TUI's stats
/// page prints the same words in lower case.
pub const STATS_HEADER: [&str; 9] = [
    "CLASS",
    "TASKS",
    "MERGED",
    "MED LINES",
    "MED CALLS",
    "MED TOKENS",
    "MED WORK",
    "BOUNCES (ALL)",
    "REVERTED",
];

/// Milestone 9.19 decision 14: the line under the table, in lower case on the TUI.
pub const STATS_FOOTER: &str = "MED = median over merged tasks \u{b7} tokens are billed (worker + reviewer + decider) \u{b7} BOUNCES counts every task";

/// `anthrex run stats`' deciders line, which the TUI's stats page prints word for word
/// (9.19 follow-up 2): `deciders: 6 calls, 1 fallback · size cross-check: 4 checked, 1
/// raised`.
pub fn deciders_text(stats: &proto::HistoryStats) -> String {
    let count = |n: u32, one: &str, many: &str| crate::numbers::count(u64::from(n), one, many);
    format!(
        "deciders: {}, {} \u{b7} size cross-check: {} checked, {} raised",
        count(stats.decider_calls, "call", "calls"),
        count(stats.decider_fallbacks, "fallback", "fallbacks"),
        stats.size_checked,
        stats.size_raised
    )
}

/// Milestone 9.19 decision 14: the rounds line's words after `rounds`: `3 ended · 1 run
/// iterated`, `2 runs` pluralised.
pub fn rounds_text(rounds: u32, iterated_runs: u32) -> String {
    let runs = crate::numbers::count(u64::from(iterated_runs), "run", "runs");
    format!("{rounds} ended \u{b7} {runs} iterated")
}
