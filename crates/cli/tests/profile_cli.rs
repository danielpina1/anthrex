//! M8b.11: the repository profile end to end (decisions 8, 10 and 11): detection by a
//! read-only onboarding scout, verification of its commands, confirmation, rejection,
//! correction, staleness and a restart during detection, through `anthrex profile` and
//! raw profile requests against an isolated daemon. Both runtime commands are the
//! test's `fake-agent` (`RunHarness`), so nothing can reach a real agent binary.
//! The refusals are in `profile_cli_refusals.rs`.

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use daemon::profile::service::RESTART_REASON;
use daemon::run::driver::{INTERRUPT_GRACE, RETIRE_AFTER};
use proto::{ProfileSource, ProfileStatus, ProposalOrigin, ProposalState, ScoutState};
use serde_json::{Value, json};

use support::run_adapt::{PROFILE_LINES, PROFILE_WAIT};
use support::run_harness::RunHarness;

/// A stopped scout's window is removed `RETIRE_AFTER` after it failed; its kill is at
/// most `INTERRUPT_GRACE`; 10 s for the ticker and the listing
/// (`docs/timing-budgets.md`, M8b.11).
const WINDOW_GONE: Duration = RETIRE_AFTER
    .saturating_add(INTERRUPT_GRACE)
    .saturating_add(Duration::from_secs(10));

const CHECK: &str = "echo checking\n";
const TEST_OK: &str = "echo 'PASS t_ok'\n";

fn harness(env: &[(&str, &str)]) -> RunHarness {
    RunHarness::with_repo(
        PROFILE_LINES,
        env,
        true,
        &[
            ("check.sh", CHECK),
            ("tests/t_ok.sh", TEST_OK),
            ("Cargo.lock", "# lock v1\n"),
        ],
    )
}

/// The scout's proposal of the brief's first scenario.
fn proposal() -> Value {
    json!({
        "check": "sh check.sh",
        "single_test": "sh tests/{test}.sh",
        "test_passed": "PASS {test}",
        "sample_test": "t_ok",
        "setup": "sh missing.sh",
        "generated": ["Cargo.lock"],
        "protected": [".claude/**", ".cursor/**"],
        "manifests": ["Cargo.lock"],
    })
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn ok(output: std::process::Output) -> String {
    assert!(
        output.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        stdout(&output),
        stderr(&output)
    );
    stdout(&output)
}

fn state(status: &ProfileStatus) -> Option<&ProposalState> {
    status.proposal.as_ref().map(|record| &record.state)
}

fn settled(status: &ProfileStatus) -> bool {
    matches!(
        state(status),
        Some(ProposalState::Ready | ProposalState::Failed { .. })
    )
}

/// Detects, and waits until the proposal is `Ready`.
fn detect_ready(h: &RunHarness) -> ProfileStatus {
    ok(h.profile(&["detect"]));
    let status = h.wait_profile("the proposal to settle", settled, PROFILE_WAIT);
    assert_eq!(state(&status), Some(&ProposalState::Ready), "{status:#?}");
    status
}

/// The detection checkouts and their repositories.
fn checkouts(h: &RunHarness, status: &ProfileStatus) -> Vec<PathBuf> {
    let wt = daemon::worktree::repo_worktrees_dir(&h.data().join("worktrees"), &status.project);
    [".onboarding", ".profile-verify"]
        .iter()
        .flat_map(|name| {
            [
                wt.join("runs").join(name),
                status.repo_dir.join("tasks").join(name),
            ]
        })
        .collect()
}

fn no_scout_window(windows: &[proto::WindowInfo]) -> bool {
    !windows.iter().any(|w| w.name.starts_with("scout/"))
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// `<flag> <value>` in a recorded argv.
fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let at = args.iter().position(|a| a == flag).expect(flag);
    &args[at + 1]
}

#[test]
fn e2e_detect_proposes_only_verified_commands() {
    let h = harness(&[]);
    h.onboarding_report(1, proposal());
    let status = detect_ready(&h);
    // Ruling R-T9-1: the scout's sandbox denies its checkout's own repository and the
    // object directory that repository borrows.
    let argv = h.io_lines("scout-onboarding-1", "args");
    let args: Vec<String> = serde_json::from_str(&argv[0]).unwrap();
    let settings: Value = serde_json::from_str(after(&args, "--settings")).unwrap();
    let denied: Vec<&str> = settings["sandbox"]["filesystem"]["denyWrite"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for path in [
        status.repo_dir.join("tasks/.onboarding"),
        status.project.join(".git/objects"),
    ] {
        assert!(
            denied.contains(&path.to_str().unwrap()),
            "{path:?} not in {denied:?}"
        );
    }
    let shown = ok(h.profile(&["show", "--proposed"]));
    assert!(shown.contains("check = \"sh check.sh\"\n"), "{shown}");
    assert!(
        shown.contains("single_test = \"sh tests/{test}.sh\"\n"),
        "{shown}"
    );
    assert!(shown.contains("generated = [\"Cargo.lock\"]\n"), "{shown}");
    assert!(!shown.contains("setup = "), "{shown}");
    assert!(
        shown.contains("# protected: built-in ") && shown.contains(" + .cursor/**\n"),
        "{shown}"
    );
    // `sh missing.sh` exits 127 under bash (macOS's `sh`) and 2 under dash (Linux's).
    assert!(
        shown.contains("# dropped\n#   setup: exit 127 after ")
            || shown.contains("# dropped\n#   setup: exit 2 after "),
        "{shown}"
    );
    assert!(shown.contains(": sh missing.sh\n"), "{shown}");
    let header = if cfg!(target_os = "macos") {
        "(confined)\n"
    } else {
        "(unconfined)\n"
    };
    assert!(shown.contains(header), "{shown}");
}

#[test]
fn e2e_confirm_stores_the_profile_outside_the_repository() {
    let h = harness(&[]);
    h.onboarding_report(1, proposal());
    detect_ready(&h);
    let confirmed = ok(h.profile_input(&["confirm", "--yes"], ""));
    assert!(confirmed.contains("stored the profile for "), "{confirmed}");
    let status = h.profile_status();
    assert_eq!(status.source, ProfileSource::Stored);
    assert_eq!(status.proposal, None);
    let stored = status.repo_dir.join("profile.toml");
    assert!(stored.starts_with(h.data().join("repos")), "{stored:?}");
    assert!(stored.is_file());
    assert_eq!(h.git(&["status", "--porcelain", "--ignored"]), "");
    assert!(!exists(&h.repo.join(".anthrex")));
    let listed: Vec<String> = h
        .git(&["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|l| l.strip_prefix("worktree ").map(str::to_string))
        .collect();
    assert_eq!(listed, [status.project.display().to_string()]);
    assert!(!exists(&status.project.join(".git/worktrees")));
    for path in checkouts(&h, &status) {
        assert!(!exists(&path), "{path:?} is left");
    }
    let shown = ok(h.profile(&["show"]));
    assert!(shown.contains("check = \"sh check.sh\"\n"), "{shown}");
}

#[test]
fn e2e_reject_stops_a_running_scout() {
    let h = harness(&[]);
    h.onboarding_script(1, &[json!({"hang": {}})]);
    ok(h.profile(&["detect"]));
    let status = h.wait_profile(
        "the scout to run",
        |s| s.proposal.as_ref().is_some_and(|r| r.window_id.is_some()),
        PROFILE_WAIT,
    );
    let window = status.proposal.as_ref().unwrap().window_id.unwrap();
    assert!(h.windows().iter().any(|w| w.id == window));
    assert_eq!(state(&status), Some(&ProposalState::Scouting));
    assert_eq!(
        status.scout.as_ref().map(|s| s.state),
        Some(ScoutState::Working)
    );
    let rejected = ok(h.profile(&["reject"]));
    assert!(rejected.contains("rejected the proposal"), "{rejected}");
    let status = h.wait_profile(
        "the proposal to be deleted",
        |s| s.proposal.is_none(),
        PROFILE_WAIT,
    );
    h.wait_windows(
        "the scout window to go",
        |ws| !ws.iter().any(|w| w.id == window),
        WINDOW_GONE,
    );
    let paths = checkouts(&h, &status);
    h.wait_profile(
        "the onboarding checkout to go",
        |_| paths.iter().all(|p| !exists(p)),
        PROFILE_WAIT,
    );
    assert_eq!(h.profile_status().proposal, None);
}

#[test]
fn e2e_edit_goes_through_verification() {
    let h = harness(&[]);
    h.onboarding_report(1, proposal());
    detect_ready(&h);
    ok(h.profile_input(&["confirm", "--yes"], ""));
    let edited = ok(h.profile(&["edit", "check", "sh broken.sh", "--yes"]));
    assert_eq!(
        edited.trim(),
        "proposed: check = sh broken.sh; it is stored as soon as verification passes (anthrex profile status)"
    );
    let status = h.wait_profile("the edit to settle", settled, PROFILE_WAIT);
    let record = status
        .proposal
        .expect("the edit's proposal is kept, not confirmed");
    assert_eq!(record.state, ProposalState::Ready);
    assert_eq!(
        record.origin,
        ProposalOrigin::Edit {
            keys: vec!["check".into()]
        }
    );
    assert!(record.auto_confirm);
    let dropped: Vec<&str> = record.dropped.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(dropped, ["check"], "{:?}", record.dropped);
    let shown = ok(h.profile(&["show"]));
    assert!(shown.contains("check = \"sh check.sh\"\n"), "{shown}");
}

#[test]
fn e2e_status_reports_stale_files_and_auto_detects() {
    let h = harness(&[]);
    h.onboarding_report(1, proposal());
    detect_ready(&h);
    ok(h.profile_input(&["confirm", "--yes"], ""));
    assert_eq!(h.profile_status().stale, Vec::<String>::new());
    std::fs::write(h.repo.join("Cargo.lock"), "# lock v2\n").unwrap();
    h.git(&["commit", "-qam", "bump the lock"]);
    h.onboarding_report(2, proposal());
    let status = h.profile_status();
    assert_eq!(status.stale, ["Cargo.lock"]);
    let record = status.proposal.expect("a re-detection started");
    assert_eq!(
        record.origin,
        ProposalOrigin::Auto {
            stale: vec!["Cargo.lock".into()]
        }
    );
    let text = ok(h.profile(&["status"]));
    assert!(
        text.contains("  stale: Cargo.lock changed since it was confirmed\n"),
        "{text}"
    );
    h.wait_profile("the re-detection to settle", settled, PROFILE_WAIT);
}

#[test]
fn e2e_detection_in_progress_at_restart_is_failed_and_cleaned() {
    let mut h = harness(&[]);
    h.onboarding_script(1, &[json!({"hang": {}})]);
    ok(h.profile(&["detect"]));
    let status = h.wait_profile(
        "the scout to run",
        |s| s.proposal.as_ref().is_some_and(|r| r.window_id.is_some()),
        PROFILE_WAIT,
    );
    assert_eq!(
        status.scout.as_ref().map(|s| s.state),
        Some(ScoutState::Working)
    );
    // Ruling R-T4-1: a crashed write's temp file is swept at the next start.
    let leftover = status.repo_dir.join("proposal.json.99999.0.tmp");
    std::fs::write(&leftover, "{").unwrap();
    let paths = checkouts(&h, &status);
    assert!(exists(&paths[0]), "the onboarding checkout is there");
    h.restart_daemon(&[]);
    let status = h.profile_status();
    assert_eq!(
        state(&status),
        Some(&ProposalState::Failed {
            reason: RESTART_REASON.into()
        })
    );
    for path in &paths {
        assert!(!exists(path), "{path:?} is left");
    }
    assert!(!exists(&leftover));
    assert!(no_scout_window(&h.windows()), "{:?}", h.windows());
}

/// Ruling R-T10-1: a verification that cannot run (here its checkout cannot be made,
/// because a file stands where the checkouts' directory goes) fails the proposal with
/// its reason.
#[test]
fn e2e_a_verification_that_cannot_run_fails_the_proposal() {
    let h = harness(&[]);
    h.onboarding_report(1, proposal());
    detect_ready(&h);
    ok(h.profile_input(&["confirm", "--yes"], ""));
    let status = h.profile_status();
    let runs = checkouts(&h, &status)[0].parent().unwrap().to_path_buf();
    std::fs::remove_dir_all(&runs).unwrap();
    std::fs::write(&runs, "not a directory").unwrap();
    ok(h.profile(&["edit", "check", "sh check.sh && true"]));
    let status = h.wait_profile("the edit to settle", settled, PROFILE_WAIT);
    match state(&status) {
        Some(ProposalState::Failed { reason }) => assert!(
            reason.starts_with("could not prepare the verification checkout: "),
            "{reason}"
        ),
        other => panic!("expected a failed proposal, got {other:?}"),
    }
    std::fs::remove_file(&runs).unwrap();
}

/// 2026-10-06, v0.1.0 on Ubuntu 26.04: Claude refused to start without bubblewrap and
/// socat (`failIfUnavailable`), and the user saw only "the scout ended two turns without
/// a report". The scout's failure, and so the proposal's, now carries Claude's stderr
/// and what to install; the daemon log has the stderr line at WARN.
#[test]
fn e2e_a_scout_that_dies_at_startup_fails_with_its_stderr() {
    const NO_BWRAP: &str = "Error: sandbox required but unavailable: sandbox is enabled but dependencies are missing: bubblewrap (bwrap) not installed, socat not installed";
    let h = harness(&[]);
    h.onboarding_script(
        1,
        &[json!({"startup_fail": {"stderr": ["starting", NO_BWRAP], "code": 1}})],
    );
    ok(h.profile(&["detect"]));
    let status = h.wait_profile("the proposal to settle", settled, PROFILE_WAIT);
    let reason = match state(&status) {
        Some(ProposalState::Failed { reason }) => reason.clone(),
        other => panic!("expected a failed proposal, got {other:?}"),
    };
    assert!(
        reason.contains(
            "claude exited at startup (code 1): starting | sandbox required but unavailable: sandbox is enabled but dependencies are missing: bubblewrap (bwrap) not installed, socat not installed"
        ),
        "{reason}"
    );
    assert!(
        reason.contains("install bubblewrap (bwrap) and socat"),
        "{reason}"
    );
    assert!(!reason.contains("two turns"), "{reason}");
    // `anthrex profile status` shows it.
    let shown = ok(h.profile(&["status"]));
    assert!(
        shown.contains("bubblewrap (bwrap) not installed"),
        "{shown}"
    );
    let log = std::fs::read_to_string(h.data().join("daemon.log")).unwrap_or_default();
    assert!(
        log.lines()
            .any(|l| l.contains("WARN") && l.contains("bubblewrap (bwrap) not installed")),
        "{log}"
    );
}
