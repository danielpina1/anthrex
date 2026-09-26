//! M8b.11: what `anthrex profile detect` refuses (decisions 8, 9 and 12), and what the
//! flag that lifts each refusal starts. Split from `profile_cli.rs` for size. Both
//! runtime commands are the test's `fake-agent` (`RunHarness`).

mod support;

use std::time::Duration;

use daemon::run::driver::{INTERRUPT_GRACE, RETIRE_AFTER};
use proto::{ProfileStatus, ProposalState};
use serde_json::json;

use support::run_adapt::{PROFILE_LINES, PROFILE_WAIT};
use support::run_harness::RunHarness;

/// As in `profile_cli.rs` (`docs/timing-budgets.md`, M8b.11).
const WINDOW_GONE: Duration = RETIRE_AFTER
    .saturating_add(INTERRUPT_GRACE)
    .saturating_add(Duration::from_secs(10));

const HOOKED_SETTINGS: &str = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"true"}]}]}}"#;

fn harness(lines: &str, env: &[(&str, &str)], files: &[(&str, &str)]) -> RunHarness {
    let mut all = vec![
        ("check.sh", "echo checking\n"),
        ("tests/t_ok.sh", "echo 'PASS t_ok'\n"),
    ];
    all.extend_from_slice(files);
    RunHarness::with_repo(&format!("{PROFILE_LINES}{lines}"), env, true, &all)
}

fn report(h: &RunHarness) {
    h.onboarding_report(
        1,
        json!({"check": "sh check.sh", "single_test": "sh tests/{test}.sh",
               "test_passed": "PASS {test}", "sample_test": "t_ok"}),
    );
}

fn refused(output: std::process::Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stderr).trim().to_string()
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

/// Waits until a started detection has settled, so the daemon stops with no git or
/// scout work of it in flight.
fn settle(h: &RunHarness) -> ProfileStatus {
    h.wait_profile("the detection to settle", settled, PROFILE_WAIT)
}

#[test]
fn e2e_detect_refuses_codex_project_config_without_trust_project() {
    let h = harness(
        "scouts.runtime = \"codex\"\n",
        &[("ANTHREX_TEST_CODEX_PROJECT_CONFIG", "load")],
        &[(".codex/config.toml", "model = \"x\"\n")],
    );
    report(&h);
    assert_eq!(
        refused(h.profile(&["detect"])),
        "this repository has project settings that headless sessions would run without asking: .codex/config.toml; review them, then detect again with --trust-project"
    );
    assert_eq!(h.profile_status().proposal, None);
    ok(h.profile(&["detect", "--trust-project"]));
    let status = settle(&h);
    let record = status.proposal.unwrap();
    assert_eq!(record.trusted_project, [".codex/config.toml"]);
}

#[test]
fn e2e_detect_refuses_project_settings_without_trust_project() {
    let h = harness(
        "",
        &[("ANTHREX_TEST_NO_SETTING_SOURCES", "1")],
        &[(".claude/settings.json", HOOKED_SETTINGS)],
    );
    report(&h);
    assert_eq!(
        refused(h.profile(&["detect"])),
        "this repository has project settings that headless sessions would run without asking: .claude/settings.json; review them, then detect again with --trust-project"
    );
    assert_eq!(h.profile_status().proposal, None);
    ok(h.profile(&["detect", "--trust-project"]));
    let status = settle(&h);
    assert_eq!(
        status.proposal.unwrap().trusted_project,
        [".claude/settings.json"]
    );
}

#[test]
fn e2e_a_second_detect_is_refused_while_one_runs() {
    let h = harness("", &[], &[]);
    h.onboarding_script(1, &[json!({"hang": {}})]);
    ok(h.profile(&["detect"]));
    let project = h.profile_status().project;
    let message = refused(h.profile(&["detect"]));
    let prefix = format!(
        "detection is already running for {} (state ",
        project.display()
    );
    assert!(message.starts_with(&prefix), "{message}");
    assert!(
        message.ends_with("); anthrex profile reject stops it"),
        "{message}"
    );
    let status = h.wait_profile(
        "the scout to run",
        |s| s.proposal.as_ref().is_some_and(|r| r.window_id.is_some()),
        PROFILE_WAIT,
    );
    let window = status.proposal.unwrap().window_id.unwrap();
    ok(h.profile(&["reject"]));
    h.wait_windows(
        "the scout window to go",
        |ws| !ws.iter().any(|w| w.id == window),
        WINDOW_GONE,
    );
}

/// Ruling R-T10-1, the task's last test: verification is refused where it cannot be
/// confined unless the user allows it, as M8a's `start_refusal` refuses a run.
#[cfg(target_os = "macos")]
#[test]
fn e2e_detect_refuses_an_unconfinable_platform_without_the_flag() {
    let h = harness("", &[("ANTHREX_CHECK_CONFINEMENT", "unavailable")], &[]);
    report(&h);
    assert_eq!(
        refused(h.profile(&["detect"])),
        "this platform cannot confine the run's checks, proofs and setup, which run code the workers wrote; pass --unconfined-checks to run them unconfined anyway, or set [orchestrator] unconfined_checks = true in your config"
    );
    assert_eq!(h.profile_status().proposal, None);
    ok(h.profile(&["detect", "--unconfined-checks"]));
    let text = ok(h.profile(&["status"]));
    assert!(text.contains("  verification: unconfined ("), "{text}");
    let status = settle(&h);
    let record = status.proposal.unwrap();
    assert_eq!(record.state, ProposalState::Ready, "{record:#?}");
    assert!(record.unconfined_checks);
    assert!(!record.verification.unwrap().confined);
}
