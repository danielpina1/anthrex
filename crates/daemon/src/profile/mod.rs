//! The repository profile (milestone 8b decisions 4 to 11).
//!
//! A repository's profile lives only in anthrex's data directory,
//! `<data_dir>/repos/<basename>-<hash8>/` ([`repo_dir`]), shared by every linked worktree
//! of the repository. Nothing is ever written into the repository itself: an agent
//! could edit such a file to weaken the checks it is judged by.
//!
//! - `resolve.rs` (pure): decision 6's precedence, stored profile over plan over config.
//! - `proposal.rs` (pure): validation, the scout's findings, and `anthrex profile edit`.
//! - `store.rs` (blocking I/O): `profile.toml`, `profile.meta.json`, `proposal.json`, and
//!   decision 7's fingerprint.
//!
//! The profile holds no confinement setting: `cache_dirs` and the `confined_*` tables
//! are the user's own config, keyed by repository root, so a model-written profile can
//! never widen what a confined command may write or reach.

use std::path::{Path, PathBuf};

use proto::RepoProfile;

use crate::run::plan::BUILTIN_PROTECTED;

pub mod proposal;
pub mod resolve;
pub mod store;

/// The onboarding scout's disposable checkout, under `<wt>/runs/` (decision 8).
pub const ONBOARDING_CHECKOUT: &str = ".onboarding";
/// The verification checkout, under `<wt>/runs/` (decision 9).
pub const VERIFY_CHECKOUT: &str = ".profile-verify";

/// Decision 4: a repository's data directory, keyed by its project root (the main
/// checkout), so every linked worktree shares one.
pub fn repo_dir(data_dir: &Path, project: &Path) -> PathBuf {
    crate::worktree::repo_worktrees_dir(&data_dir.join("repos"), project)
}

/// The profile in a few lines, for triage and `get_context` (Interfaces, "Contracts and
/// texts"). A list line is omitted when the list is empty; `protected` always prints.
pub fn summary(profile: &RepoProfile) -> String {
    let mut lines = Vec::new();
    for (label, list) in [
        ("languages", &profile.languages),
        ("modules", &profile.modules),
        ("hub", &profile.hub),
        ("source", &profile.source),
        ("generated", &profile.generated),
    ] {
        if !list.is_empty() {
            lines.push(format!("{label}: {}", list.join(", ")));
        }
    }
    let extras = if profile.protected.is_empty() {
        "no extras".to_string()
    } else {
        profile.protected.join(", ")
    };
    lines.push(format!(
        "protected: built-in {} + {extras}",
        BUILTIN_PROTECTED.join(", ")
    ));
    lines.push(format!(
        "setup: {}",
        profile.setup.as_deref().unwrap_or("none")
    ));
    lines.push(format!(
        "check: {}",
        profile
            .check
            .as_deref()
            .unwrap_or("none (runs are unverified)")
    ));
    lines.push(format!(
        "single test: {}",
        profile
            .single_test
            .as_deref()
            .unwrap_or("none (tdd is impossible; code tasks use check)")
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests;
