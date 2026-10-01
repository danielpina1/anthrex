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
//! - `verify.rs` (blocking I/O): decision 9's verification of a proposal's commands in a
//!   fresh, confined, standalone checkout.
//! - `service.rs` (+ `service_run.rs`, `service_requests.rs`): `ProfileService`,
//!   decisions 8, 10 and 11: detection, verification, confirmation, `anthrex profile`.
//!
//! The profile holds no confinement setting: `cache_dirs` and the `confined_*` tables
//! are the user's own config, keyed by repository root, so a model-written profile can
//! never widen what a confined command may write or reach.

use std::path::{Path, PathBuf};

use proto::RepoProfile;

pub mod proposal;
mod proposal_delivery;
mod proposal_tiers;
pub mod resolve;
pub mod service;
mod service_requests;
mod service_restore;
pub mod service_run;
pub mod service_start;
pub mod store;
pub mod verify;
mod verify_steps;
pub(crate) mod verify_tiers;

/// The onboarding scout's disposable checkout, under `<wt>/runs/` (decision 8).
pub const ONBOARDING_CHECKOUT: &str = ".onboarding";
/// The verification checkout, under `<wt>/runs/` (decision 9).
pub const VERIFY_CHECKOUT: &str = ".profile-verify";

/// Decision 4: a repository's data directory, keyed by its project root (the main
/// checkout), so every linked worktree shares one.
pub fn repo_dir(data_dir: &Path, project: &Path) -> PathBuf {
    crate::worktree::repo_worktrees_dir(&data_dir.join("repos"), project)
}

/// Decision 9's hint on a `setup` or `check` that failed confined, naming the
/// repository root whose `[orchestrator.*]` tables would allow what it needs. It lives
/// here, not in the pure `proposal.rs`, so no model-writable surface's file spells a
/// confinement key (the brief's Verification grep).
pub fn confined_hint(root: &Path) -> String {
    format!(
        "it ran confined, as runs do: if it needs the network, a cache directory, a Unix \
         socket or a localhost port, allow it for {} in your config \
         ([orchestrator.confined_network], [orchestrator.cache_dirs], \
         [orchestrator.confined_unix_sockets], [orchestrator.confined_localhost_ports]), \
         then run anthrex profile detect",
        root.display()
    )
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
    lines.push(proposal::protected_line(profile));
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
    lines.extend(proposal_tiers::summary_line(profile));
    lines.join("\n")
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_delivery;
#[cfg(test)]
mod tests_ready;
#[cfg(test)]
mod tests_tiers;
#[cfg(test)]
mod tests_tiers_expansion;
#[cfg(test)]
mod tests_tiers_review;
#[cfg(test)]
mod tests_verify;
#[cfg(test)]
mod tests_verify_steps;
