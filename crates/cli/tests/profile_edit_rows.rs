//! Milestone 9.10, task 6 (decisions 15 to 18): a row edit of the stored profile through
//! the real binary against an isolated daemon, `fake-agent` as every runtime
//! (`RunHarness`). `--anyway` stores the edit once verification has run, whatever it
//! found; `--yes` still needs a pass, and a failed edit waits to be rejected.

mod support;

use proto::{ProfileSource, ProfileStatus, ProposalState, RowEditState};
use serde_json::json;
use support::run_adapt::{ADAPT_FILES, PROFILE_LINES, PROFILE_WAIT};
use support::run_harness::RunHarness;

fn ok(out: std::process::Output) -> String {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn ready(status: &ProfileStatus) -> bool {
    matches!(
        status.proposal.as_ref().map(|p| &p.state),
        Some(ProposalState::Ready)
    )
}

/// A stored profile whose `check` is `sh check.sh`.
fn stored() -> RunHarness {
    let h = RunHarness::adapt("claude", PROFILE_LINES, &[], ADAPT_FILES);
    h.onboarding_report(1, json!({"check": "sh check.sh"}));
    ok(h.profile(&["detect"]));
    h.wait_profile("the proposal", ready, PROFILE_WAIT);
    ok(h.profile_input(&["use", "--yes"], ""));
    assert_eq!(h.profile_status().source, ProfileSource::Stored);
    h
}

/// Decision 18 (and M9.10.11's `profile_cli_words.rs` test, which this replaces): the
/// first `--anyway` verifies once, then stores the edit although its check failed.
#[test]
fn e2e_edit_anyway_stores_a_failing_check() {
    let h = stored();
    let out = ok(h.profile(&["edit", "check", "false", "--anyway"]));
    assert_eq!(
        out.trim_end(),
        "proposed: check = false; it is stored once verification has run, whatever it finds (anthrex profile status)"
    );
    h.wait_profile(
        "the edit to be stored",
        |s| s.proposal.is_none(),
        PROFILE_WAIT,
    );
    let shown = ok(h.profile(&["show"]));
    assert!(shown.contains("check = \"false\""), "{shown}");
}

/// Decision 16: `--yes` stores only on a pass; a ✗ is held for the user, and `profile
/// reject` discards it with the stored profile unchanged.
#[test]
fn e2e_edit_yes_still_needs_a_pass() {
    let h = stored();
    ok(h.profile(&["edit", "check", "false", "--yes"]));
    let status = h.wait_profile(
        "the edit's check",
        |s| {
            s.proposal
                .as_ref()
                .and_then(|p| p.edit.as_ref())
                .is_some_and(|e| matches!(e.state, RowEditState::Failed { .. }))
        },
        PROFILE_WAIT,
    );
    let record = status.proposal.unwrap();
    assert_eq!(record.state, ProposalState::Ready);
    // Final review C-I2: `profile status` says how the edit ended.
    let text = ok(h.profile(&["status"]));
    assert!(
        text.contains("  edit: check = false failed its check: "),
        "{text}"
    );
    assert!(
        text.contains("(save it anyway with --anyway, or anthrex profile reject)"),
        "{text}"
    );
    let shown = ok(h.profile(&["show"]));
    assert!(shown.contains("check = \"sh check.sh\""), "{shown}");
    let out = ok(h.profile(&["reject"]));
    assert!(out.contains("rejected the proposal for "), "{out}");
    assert_eq!(h.profile_status().proposal, None);
    let shown = ok(h.profile(&["show"]));
    assert!(shown.contains("check = \"sh check.sh\""), "{shown}");
}
