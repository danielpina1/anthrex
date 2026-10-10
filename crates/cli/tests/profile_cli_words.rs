//! Milestone 9.10, task 11 (decisions 36-39): `profile use` and its `confirm` alias,
//! `profile edit --anyway`, `profile status`'s status line and waiting goals, and
//! `run start --goal`'s loud queued failure, through the real binary against an isolated
//! daemon with `fake-agent` as every runtime (`RunHarness`).

mod support;

use std::process::Output;

use proto::{ProfileSource, ProposalState};
use serde_json::json;
use support::run_adapt::{ADAPT_FILES, PROFILE_LINES, PROFILE_WAIT};
use support::run_harness::RunHarness;

const QUEUED_LINE: &str =
    "queued: add a waits for the repository profile (anthrex profile); no run started yet";

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn ok(out: Output) -> String {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        stdout(&out),
        stderr(&out)
    );
    stdout(&out)
}

fn harness() -> RunHarness {
    RunHarness::adapt("claude", PROFILE_LINES, &[], ADAPT_FILES)
}

fn ready(status: &proto::ProfileStatus) -> bool {
    matches!(
        status.proposal.as_ref().map(|p| &p.state),
        Some(ProposalState::Ready)
    )
}

/// `  Ready · verified <n>s ago`, the age being digits and a unit.
fn is_ready_line(line: &str) -> bool {
    let Some(age) = line
        .strip_prefix("  Ready · verified ")
        .and_then(|rest| rest.strip_suffix(" ago"))
    else {
        return false;
    };
    let digits = age.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && matches!(&age[digits.len()..], "s" | "m" | "h")
}

/// Detect, `use --yes` stores; then an edit proposal is stored with the alias `confirm`.
/// Not fail-first: `profile use` landed in M9.10.5 (the controller's ruling).
#[test]
fn e2e_use_stores_and_confirm_still_works() {
    let h = harness();
    h.onboarding_report(1, json!({"check": "sh check.sh"}));
    ok(h.profile(&["detect"]));
    h.wait_profile("the proposal", ready, PROFILE_WAIT);
    let used = ok(h.profile_input(&["use", "--yes"], ""));
    assert!(used.contains("stored the profile for "), "{used}");
    let status = h.profile_status();
    assert_eq!(status.source, ProfileSource::Stored);
    assert_eq!(status.proposal, None);
    // A second proposal (an edit, with no --yes, stays a proposal) is stored by the alias.
    ok(h.profile(&["edit", "check", "sh check.sh && true"]));
    let status = h.wait_profile("the edit proposal", ready, PROFILE_WAIT);
    assert!(status.proposal.is_some());
    let confirmed = ok(h.profile_input(&["confirm", "--yes"], ""));
    assert!(confirmed.contains("stored the profile for "), "{confirmed}");
    let shown = ok(h.profile(&["show"]));
    assert!(shown.contains("check = \"sh check.sh && true\""), "{shown}");
}

#[test]
fn e2e_status_prints_the_status_line() {
    let h = harness();
    h.onboarding_report(1, json!({"check": "sh check.sh"}));
    let text = ok(h.profile(&["status"]));
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("profile: "), "{text}");
    assert_eq!(lines[1], "  Not set up — press d to set up", "{text}");
    assert_eq!(lines[2], "  stored: no", "{text}");
    // A goal waits while the proposal is reviewed.
    let out = h.start_goal("add a", &[]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    h.wait_profile("the proposal", ready, PROFILE_WAIT);
    let text = ok(h.profile(&["status"]));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[1], "  Needs review — anthrex has a proposal",
        "{text}"
    );
    assert!(lines.contains(&"  waiting: add a"), "{text}");
    ok(h.profile_input(&["use", "--yes"], ""));
    let text = ok(h.profile(&["status"]));
    let line = text.lines().nth(1).unwrap();
    assert!(is_ready_line(line), "{text}");
    assert!(!text.contains("  waiting:"), "{text}");
}

#[test]
fn e2e_edit_anyway_is_in_the_help() {
    let h = harness();
    let help = ok(h.anthrex(&["profile", "edit", "--help"]));
    assert!(
        help.contains("--anyway"),
        "profile edit --help lists --anyway\n{help}"
    );
    assert!(
        help.contains("Store it once verification has run, even if a check fails (implies --yes)"),
        "{help}"
    );
    let use_help = ok(h.anthrex(&["profile", "--help"]));
    assert!(
        use_help.contains("Use the ready proposal: store it, then start any queued goals"),
        "{use_help}"
    );
    // Final review M8: `reject` says it drops the waiting goals too.
    assert!(
        use_help
            .contains("Stop a detection and delete the proposal (and any goals waiting for it)"),
        "{use_help}"
    );
}

#[test]
fn e2e_a_queued_goal_fails_loudly() {
    let h = harness();
    h.onboarding_report(1, json!({"check": "sh check.sh"}));
    let out = h.start_goal("add a", &[]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out).trim_end(), QUEUED_LINE);
    // Leave no detection behind.
    h.wait_profile("the proposal", ready, PROFILE_WAIT);
}

#[test]
fn e2e_run_start_help_documents_exit_3() {
    let h = harness();
    let help = ok(h.anthrex(&["run", "start", "--help"]));
    assert!(
        help.contains(
            "In a repository with no profile the goal waits for it: exit 3, nothing on stdout"
        ),
        "{help}"
    );
}
