//! Milestone 9.10.7: the profile's plain words.

use super::*;
use proto::{
    CheckProgress, DeliveryMode, DeliveryProfile, ProfileSource, ProposalOrigin, ProposalRecord,
    ProposalState, SetupState,
};
use std::path::PathBuf;

/// A literal copy of `EDIT_KEYS` (`daemon/src/profile/proposal.rs:41`), so a key the
/// daemon gains goes red here until it has a label and a hint.
const EDIT_KEYS: &[&str] = &[
    "languages",
    "modules",
    "hub",
    "source",
    "generated",
    "protected",
    "setup",
    "check",
    "check_timeout_secs",
    "single_test",
    "test_passed",
    "sample_test",
    "output_filter",
    "filter_prefixes",
    "conventions",
    "manifests",
    "build_check",
    "module_test",
    "module_tests",
    "module_graph",
    "module_names",
    "full_triggers",
    "slow_tests",
    "timing_tests",
    "skip_markers",
    "test_paths",
    "full_shards",
    "toolchain_id",
    "env",
    "env.<NAME>",
    "delivery.mode",
    "delivery.remote",
];

const NOW: u64 = 1_000_000;

fn status() -> ProfileStatus {
    ProfileStatus {
        project: PathBuf::from("/p"),
        repo_dir: PathBuf::from("/d"),
        source: ProfileSource::None,
        confirmed_at: None,
        stale: Vec::new(),
        unparseable: None,
        proposal: None,
        scout: None,
        verify_confined: true,
        queued: Vec::new(),
        checking: None,
        verified_at: None,
        unreadable_text: None,
        dropped_goals: Vec::new(),
    }
}

fn stored(verified_ago: u64) -> ProfileStatus {
    ProfileStatus {
        source: ProfileSource::Stored,
        confirmed_at: Some(NOW - verified_ago),
        verified_at: Some(NOW - verified_ago),
        ..status()
    }
}

fn stale(paths: &[&str], mut s: ProfileStatus) -> ProfileStatus {
    s.stale = paths.iter().map(|p| p.to_string()).collect();
    s
}

fn proposal(state: ProposalState, origin: ProposalOrigin) -> ProposalRecord {
    ProposalRecord {
        project: PathBuf::from("/p"),
        state,
        origin,
        started_at: 0,
        updated_at: 0,
        base_sha: String::new(),
        scout_id: None,
        window_id: None,
        profile: None,
        verification: None,
        dropped: Vec::new(),
        proposed: None,
        trusted_project: Vec::new(),
        unconfined_checks: false,
        auto_confirm: false,
        edit: None,
    }
}

fn with(mut s: ProfileStatus, state: ProposalState, origin: ProposalOrigin) -> ProfileStatus {
    s.proposal = Some(proposal(state, origin));
    s
}

/// Decision 23: one status per rule, each line exactly.
#[test]
fn every_status_line() {
    let line = |s: &ProfileStatus| status_line(s, NOW);
    let auto = || ProposalOrigin::Auto { stale: Vec::new() };
    assert_eq!(line(&status()), "Not set up — press d to set up");
    assert_eq!(
        line(&ProfileStatus {
            unparseable: Some("bad".into()),
            ..status()
        }),
        "Can't read the profile file — ⏎ shows it"
    );
    for state in [ProposalState::Preparing, ProposalState::Scouting] {
        assert_eq!(
            line(&with(status(), state, ProposalOrigin::Detect)),
            "Setting up… reading the repo"
        );
    }
    let verifying = with(status(), ProposalState::Verifying, ProposalOrigin::Goal);
    assert_eq!(line(&verifying), "Setting up… checking commands");
    let counted = ProfileStatus {
        checking: Some(CheckProgress { done: 2, total: 5 }),
        ..verifying
    };
    assert_eq!(line(&counted), "Setting up… checking commands (2/5)");
    assert_eq!(
        line(&with(
            status(),
            ProposalState::Ready,
            ProposalOrigin::Detect
        )),
        "Needs review — anthrex has a proposal"
    );
    // A stored profile: a ninth line for stale with nothing running (decision 23, R).
    assert_eq!(
        line(&stale(&["Cargo.toml"], stored(60))),
        "Out of date — Cargo.toml changed · press d to check again"
    );
    let three = stale(&["a", "b", "c"], stored(60));
    assert_eq!(
        line(&three),
        "Out of date — a and 2 more changed · press d to check again"
    );
    assert_eq!(
        line(&with(three.clone(), ProposalState::Scouting, auto())),
        "Out of date — a and 2 more changed · re-checking"
    );
    assert_eq!(
        line(&with(three, ProposalState::Ready, auto())),
        "Out of date — a and 2 more changed · review the changes"
    );
    assert_eq!(line(&stored(45)), "Ready · verified 45s ago");
    assert_eq!(line(&stored(12 * 60)), "Ready · verified 12m ago");
    assert_eq!(line(&stored(5 * 3600)), "Ready · verified 5h ago");
    assert_eq!(line(&stored(86_400)), "Ready · verified 1 day ago");
    assert_eq!(line(&stored(3 * 86_400)), "Ready · verified 3 days ago");
}

/// Decision 23: the first matching rule wins.
#[test]
fn the_first_rule_wins() {
    let line = |s: &ProfileStatus| status_line(s, NOW);
    let running = with(
        ProfileStatus {
            unparseable: Some("bad".into()),
            ..status()
        },
        ProposalState::Scouting,
        ProposalOrigin::Detect,
    );
    assert_eq!(line(&running), "Can't read the profile file — ⏎ shows it");
    let auto = ProposalOrigin::Auto { stale: Vec::new() };
    assert_eq!(
        line(&with(
            stale(&["x"], stored(10)),
            ProposalState::Ready,
            auto.clone()
        )),
        "Out of date — x changed · review the changes"
    );
    // A row edit verifying on a fresh stored profile is not a review proposal.
    let edit = ProposalOrigin::Edit {
        keys: vec!["check".into()],
    };
    assert_eq!(
        line(&with(stored(60), ProposalState::Verifying, edit)),
        "Ready · verified 1m ago"
    );
}

#[test]
fn labels_and_hints_cover_every_key() {
    for key in EDIT_KEYS {
        let key = if *key == "env.<NAME>" {
            "env.PATH"
        } else {
            key
        };
        assert!(!label(key).is_empty(), "{key} has no label");
        assert!(!hint(key).is_empty(), "{key} has no hint");
    }
    assert_eq!(label("env.PATH"), "PATH");
    assert_eq!(label("env"), "add a variable");
    assert_eq!(label("delivery.mode"), "Delivery");
    assert_eq!(label("single_test"), "single test");
    assert_eq!(label("test_paths"), "tests");
}

/// Decision 25: a hint names only placeholders a command really has.
#[test]
fn hints_name_real_placeholders_only() {
    for key in EDIT_KEYS {
        assert!(!hint(key).contains("{crate}"), "{key}: {}", hint(key));
    }
    assert_eq!(
        hint("single_test"),
        "how anthrex runs one test; {test} is filled in"
    );
}

#[test]
fn took_and_age() {
    assert_eq!(took(12), "12s");
    assert_eq!(took(190), "3m10s");
    assert_eq!(took(3900), "1h05m");
    assert_eq!(age(45), "45s");
    assert_eq!(age(12 * 60), "12m");
    assert_eq!(age(5 * 3600), "5h");
    assert_eq!(age(86_400), "1 day");
    assert_eq!(age(3 * 86_400), "3 days");
}

/// Decision 26.
#[test]
fn delivery_texts() {
    let pr = DeliveryProfile {
        mode: DeliveryMode::Pr,
        remote: "origin".into(),
    };
    let local = DeliveryProfile {
        mode: DeliveryMode::Local,
        remote: "origin".into(),
    };
    assert_eq!(delivery_text(Some(&pr)), "pull request · remote origin");
    assert_eq!(delivery_text(Some(&local)), "merge here, no pull request");
    assert_eq!(delivery_text(None), "merge here, no pull request");
}

/// Decision 34: the set-up alert's text.
#[test]
fn setup_texts() {
    assert_eq!(
        setup_text(&SetupState::Reading),
        "setting up anthrex · reading the repo"
    );
    assert_eq!(
        setup_text(&SetupState::Checking {
            progress: Some(CheckProgress { done: 2, total: 4 })
        }),
        "setting up anthrex · checking commands (2/4)"
    );
    assert_eq!(
        setup_text(&SetupState::Failed {
            reason: "no network\nsecond line".into()
        }),
        "setting up failed: no network"
    );
}
