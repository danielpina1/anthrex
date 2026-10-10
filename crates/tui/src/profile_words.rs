//! Milestone 9.10 decisions 23-26: the repository profile in plain words, pure.
//! The status line (shared by the Profile screen and `anthrex profile status`), the
//! labels and hints of every key, ages and durations, delivery's text and the set-up
//! alert's text. No I/O: `now` is passed in.

use proto::{
    DeliveryMode, DeliveryProfile, ProfileSource, ProfileStatus, ProposalOrigin, ProposalState,
    SetupState,
};

/// The folded Advanced line's summary.
pub const ADVANCED_SUMMARY: &str =
    "testing tiers, output filter, environment, timeouts, shared code area";

/// `<n>s`, `<n>m`, `<n>h`, `1 day`, `<n> days`.
pub fn age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        86_400..=172_799 => "1 day".to_string(),
        _ => format!("{} days", secs / 86_400),
    }
}

/// How long a command took: `12s`, `3m10s`, `1h05m`.
pub fn took(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
    }
}

/// The stale paths as a status line names them: the first, then ` and <n> more`.
fn files(stale: &[String]) -> String {
    match stale {
        [] => String::new(),
        [only] => only.clone(),
        [first, rest @ ..] => format!("{first} and {} more", rest.len()),
    }
}

/// What a status line says, for its colour (decision 23): unreadable, setting up or
/// re-checking, needing review or out of date, ready, or not set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusTone {
    Unreadable,
    Working,
    Attention,
    Ready,
    NotSetUp,
}

/// Decision 23: the one status line; the first rule that matches wins.
pub fn status_line(status: &ProfileStatus, now: u64) -> String {
    status_line_tone(status, now).0
}

/// Decision 23: the status line and its tone, both from the same rule.
pub fn status_line_tone(status: &ProfileStatus, now: u64) -> (String, StatusTone) {
    use StatusTone::*;
    if status.unparseable.is_some() {
        return (
            "Can't read the profile file — ⏎ shows it".into(),
            Unreadable,
        );
    }
    let stored = status.source == ProfileSource::Stored;
    let out_of_date = || format!("Out of date — {} changed", files(&status.stale));
    let review = status
        .proposal
        .as_ref()
        .filter(|p| !matches!(p.origin, ProposalOrigin::Edit { .. }));
    if let Some(p) = review {
        match &p.state {
            ProposalState::Preparing | ProposalState::Scouting | ProposalState::Verifying => {
                if stored && !status.stale.is_empty() {
                    return (format!("{} · re-checking", out_of_date()), Working);
                }
                let line = match (&p.state, status.checking) {
                    (ProposalState::Verifying, Some(c)) => {
                        format!("Setting up… checking commands ({}/{})", c.done, c.total)
                    }
                    (ProposalState::Verifying, None) => "Setting up… checking commands".into(),
                    _ => "Setting up… reading the repo".to_string(),
                };
                return (line, Working);
            }
            ProposalState::Ready => {
                if stored && !status.stale.is_empty() {
                    return (format!("{} · review the changes", out_of_date()), Attention);
                }
                return ("Needs review — anthrex has a proposal".into(), Attention);
            }
            ProposalState::Failed { .. } => {}
        }
    }
    if stored {
        if !status.stale.is_empty() {
            return (
                format!("{} · press d to check again", out_of_date()),
                Attention,
            );
        }
        let line = match status.verified_at.or(status.confirmed_at) {
            Some(at) => format!("Ready · verified {} ago", age(now.saturating_sub(at))),
            None => "Ready".to_string(),
        };
        return (line, Ready);
    }
    ("Not set up — press d to set up".into(), NotSetUp)
}

/// Decision 24's labels: the plain name of a key (`env.NAME` is `NAME`).
pub fn label(key: &str) -> String {
    if let Some(name) = key.strip_prefix("env.") {
        return name.to_string();
    }
    match key {
        "single_test" => "single test",
        "test_paths" => "tests",
        "delivery.mode" => "Delivery",
        "delivery.remote" => "remote",
        "build_check" => "build check",
        "module_test" => "test one module",
        "module_tests" => "test modules",
        "module_graph" => "module graph",
        "module_names" => "module names",
        "full_triggers" => "full-run triggers",
        "slow_tests" => "slow tests",
        "timing_tests" => "timing tests",
        "skip_markers" => "skip markers",
        "full_shards" => "full-run shards",
        "toolchain_id" => "toolchain",
        "output_filter" => "output filter",
        "filter_prefixes" => "filter prefixes",
        "env" => "add a variable",
        "check_timeout_secs" => "check timeout",
        "test_passed" => "pass pattern",
        "sample_test" => "sample test",
        "hub" => "shared code",
        other => other,
    }
    .to_string()
}

/// Decisions 24 and 25: the one-line hint under a key; names only real placeholders.
pub fn hint(key: &str) -> &'static str {
    if key.starts_with("env.") {
        return "set for every check";
    }
    match key {
        "setup" => "how anthrex prepares a fresh checkout before any check",
        "check" => "the command that must pass before work is merged",
        "single_test" => "how anthrex runs one test; {test} is filled in",
        "source" => "where the code lives",
        "test_paths" => "where the tests live",
        "generated" => "files a tool writes; agents never edit them by hand",
        "protected" => "files agents may not change, beyond the built-in ones",
        "delivery.mode" => "how finished work reaches you: a local merge or a pull request",
        "build_check" => "a quick build that runs before any test",
        "module_test" => "how anthrex tests one module; {module} is filled in",
        "module_tests" => "how anthrex tests several modules; {modules} is filled in",
        "module_graph" => "prints which modules depend on which",
        "module_names" => "how modules are named in those commands",
        "full_triggers" => "files whose change runs every test",
        "slow_tests" => "the tests left out of quick runs",
        "timing_tests" => "tests that must run alone",
        "skip_markers" => "text that marks a test as skipped",
        "full_shards" => "how many parts a full test run is split into",
        "toolchain_id" => "prints the toolchain version, so cached results match it",
        "output_filter" => "which test output agents see",
        "filter_prefixes" => "commands whose output is filtered",
        "env" => "adds an environment variable set for every check",
        "check_timeout_secs" => "seconds before a check is stopped",
        "test_passed" => "the line that shows a test passed; {test} is filled in",
        "sample_test" => "one real test, used to check the single-test command",
        "hub" => "code many tasks touch, changed one task at a time",
        "languages" => "the repo's languages",
        "modules" => "the repo's module directories",
        "manifests" => "files whose change makes the profile out of date",
        "conventions" => "files that describe how to work here",
        "delivery.remote" => "the git remote pull requests go to",
        _ => "",
    }
}

/// Decision 26: delivery in words.
pub fn delivery_text(delivery: Option<&DeliveryProfile>) -> String {
    match delivery {
        Some(d) if d.mode == DeliveryMode::Pr => format!("pull request · remote {}", d.remote),
        _ => "merge here, no pull request".to_string(),
    }
}

/// Decision 34: the set-up alert's text for one repository's queued goals.
pub fn setup_text(state: &SetupState) -> String {
    match state {
        SetupState::Reading => "setting up anthrex · reading the repo".to_string(),
        SetupState::Checking { progress: Some(c) } => {
            format!(
                "setting up anthrex · checking commands ({}/{})",
                c.done, c.total
            )
        }
        SetupState::Checking { progress: None } => {
            "setting up anthrex · checking commands".to_string()
        }
        SetupState::NeedsReview => "review how anthrex will work here".to_string(),
        // Its first line with words (task 10 review M5: not a leading blank one).
        SetupState::Failed { reason } => format!(
            "setting up failed: {}",
            reason
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
        ),
    }
}

#[cfg(test)]
#[path = "profile_words_tests.rs"]
mod tests;
