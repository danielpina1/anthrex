//! M8b.11 review (I2, I3 M5, m2, m3): `profile edit --yes` stores a profile only when
//! verification dropped nothing, both of its paths store, and `profile confirm` stores
//! exactly the proposal it showed. Both runtime commands are the test's `fake-agent`.

mod support;

use proto::{ProfileReply, ProfileRequest, ProfileSource, ProfileStatus, ProposalState};
use serde_json::json;

use support::run_adapt::{PROFILE_LINES, PROFILE_WAIT};
use support::run_harness::RunHarness;

fn harness() -> RunHarness {
    RunHarness::with_repo(
        PROFILE_LINES,
        &[],
        true,
        &[
            ("check.sh", "echo checking\n"),
            ("tests/t_ok.sh", "echo 'PASS t_ok'\n"),
        ],
    )
}

fn ok(output: std::process::Output) -> String {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn settled(status: &ProfileStatus) -> bool {
    status
        .proposal
        .as_ref()
        .is_some_and(|r| matches!(r.state, ProposalState::Ready | ProposalState::Failed { .. }))
}

/// A stored profile with `check` and `single_test`, both verified.
fn stored(h: &RunHarness) {
    h.onboarding_report(
        1,
        json!({"check": "sh check.sh", "single_test": "sh tests/{test}.sh",
               "test_passed": "PASS {test}", "sample_test": "t_ok"}),
    );
    ok(h.profile(&["detect"]));
    let status = h.wait_profile("the detection to settle", settled, PROFILE_WAIT);
    assert_eq!(status.proposal.unwrap().dropped, []);
    ok(h.profile(&["confirm", "--yes"]));
    assert_eq!(h.profile_status().source, ProfileSource::Stored);
}

fn shown(h: &RunHarness) -> String {
    ok(h.profile(&["show"]))
}

/// Review I2: an untouched `single_test` that fails now is dropped, so `--yes` leaves
/// the proposal `Ready` for a human, and the stored profile keeps `single_test`.
#[test]
fn e2e_edit_yes_does_not_store_when_an_untouched_command_is_dropped() {
    let h = harness();
    stored(&h);
    std::fs::remove_file(h.repo.join("tests/t_ok.sh")).unwrap();
    h.git(&["commit", "-qam", "drop the test"]);
    ok(h.profile(&["edit", "check", "sh check.sh && true", "--yes"]));
    let status = h.wait_profile("the edit to settle", settled, PROFILE_WAIT);
    let record = status
        .proposal
        .expect("the edit's proposal waits for a human");
    assert_eq!(record.state, ProposalState::Ready);
    let dropped: Vec<&str> = record.dropped.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(dropped, ["single_test"]);
    let text = shown(&h);
    assert!(text.contains("check = \"sh check.sh\"\n"), "{text}");
    assert!(
        text.contains("single_test = \"sh tests/{test}.sh\"\n"),
        "{text}"
    );
}

/// Review I3 (M5): a re-verified `--yes` edit whose verification drops nothing is
/// stored with no confirmation.
#[test]
fn e2e_edit_yes_stores_once_verification_drops_nothing() {
    let h = harness();
    stored(&h);
    ok(h.profile(&["edit", "check", "sh check.sh && true", "--yes"]));
    h.wait_profile(
        "the edit to be stored",
        |s| s.proposal.is_none(),
        PROFILE_WAIT,
    );
    let text = shown(&h);
    assert!(text.contains("check = \"sh check.sh && true\"\n"), "{text}");
}

/// Review I3 (M5) and m2: a `--yes` edit that needs no verification is stored at once,
/// and says so.
#[test]
fn e2e_edit_yes_without_reverification_is_stored_at_once() {
    let h = harness();
    stored(&h);
    let out = ok(h.profile(&["edit", "modules", "[\"src/*\"]", "--yes"]));
    assert_eq!(
        out.trim(),
        "proposed: modules = [\"src/*\"]; stored (it needed no verification)"
    );
    assert_eq!(h.profile_status().proposal, None);
    assert!(shown(&h).contains("modules = [\"src/*\"]\n"));
}

/// Review m3: `confirm` stores only the proposal the user was shown.
#[test]
fn e2e_confirm_refuses_a_proposal_that_changed_since_it_was_shown() {
    let h = harness();
    stored(&h);
    ok(h.profile(&["edit", "modules", "[\"src/*\"]"]));
    let project = h.profile_status().project;
    let reply = h.profile_request(ProfileRequest::Confirm {
        dir: h.repo.clone(),
        shown: Some("modules = [\"lib/*\"]\n".into()),
    });
    assert_eq!(
        reply,
        ProfileReply::Refused {
            message: format!(
                "the proposal for {} changed since it was shown; run anthrex profile confirm again",
                project.display()
            )
        }
    );
    assert!(h.profile_status().proposal.is_some());
    assert!(!shown(&h).contains("modules = "));
    ok(h.profile(&["confirm", "--yes"]));
    assert!(shown(&h).contains("modules = [\"src/*\"]\n"));
}
