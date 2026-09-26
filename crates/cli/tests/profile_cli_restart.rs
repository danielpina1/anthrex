//! M8b.11 review (I4, m1, I3 M20): decision 7's automatic re-detection at the
//! daemon's start and at `run start`, and detection checkouts left over with no
//! proposal. Nothing here asks `profile status` before the assertion that matters,
//! since `status` would start the re-detection itself. Both runtime commands are the
//! test's `fake-agent`.

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::{ProposalOrigin, ProposalRecord, ProposalState};
use serde_json::{Value, json};

use support::run_adapt::{PROFILE_LINES, PROFILE_WAIT};
use support::run_harness::{RunHarness, git_in, init_repo};
use support::run_plans::{plan, task, until};

fn harness() -> RunHarness {
    RunHarness::with_repo(
        PROFILE_LINES,
        &[],
        true,
        &[("check.sh", "echo checking\n"), ("Cargo.lock", "# v1\n")],
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

fn profile() -> Value {
    json!({"check": "sh check.sh", "manifests": ["Cargo.lock"]})
}

fn project(h: &RunHarness) -> PathBuf {
    h.repo.canonicalize().unwrap()
}

fn proposal_file(h: &RunHarness) -> PathBuf {
    daemon::profile::repo_dir(&h.data(), &project(h)).join("proposal.json")
}

/// Reads `proposal.json` directly (never through `profile status`).
fn proposal(h: &RunHarness) -> Option<ProposalRecord> {
    let text = std::fs::read_to_string(proposal_file(h)).ok()?;
    serde_json::from_str(&text).ok()
}

/// A confirmed profile watching `Cargo.lock`, which is then changed and committed.
fn stale_profile(h: &RunHarness) {
    h.onboarding_report(1, profile());
    ok(h.profile(&["detect"]));
    let status = h.wait_profile(
        "the detection to settle",
        |s| {
            s.proposal
                .as_ref()
                .is_some_and(|r| r.state == ProposalState::Ready)
        },
        PROFILE_WAIT,
    );
    assert!(status.proposal.is_some());
    ok(h.profile(&["confirm", "--yes"]));
    std::fs::write(h.repo.join("Cargo.lock"), "# v2\n").unwrap();
    h.git(&["commit", "-qam", "bump the lock"]);
    h.onboarding_report(2, profile());
}

fn auto_proposal(h: &RunHarness) -> ProposalRecord {
    until("an automatic proposal", PROFILE_WAIT, || proposal(h))
}

fn wait_settled(h: &RunHarness) {
    until("the re-detection to settle", PROFILE_WAIT, || {
        proposal(h)
            .filter(|r| matches!(r.state, ProposalState::Ready | ProposalState::Failed { .. }))
    });
}

/// Review I4: a stored profile that went stale while the daemon was down starts a
/// re-detection at the daemon's start (decision 7).
#[test]
fn e2e_a_restart_with_a_stale_profile_starts_an_automatic_proposal() {
    let mut h = harness();
    stale_profile(&h);
    assert!(proposal(&h).is_none());
    h.restart_daemon(&[]);
    let record = auto_proposal(&h);
    assert_eq!(
        record.origin,
        ProposalOrigin::Auto {
            stale: vec!["Cargo.lock".into()]
        }
    );
    wait_settled(&h);
}

/// Review I4: `run start` with a stale stored profile starts a re-detection too.
#[test]
fn e2e_run_start_with_a_stale_profile_starts_an_automatic_proposal() {
    let h = harness();
    stale_profile(&h);
    assert!(proposal(&h).is_none());
    h.start(&plan("", &[task("t1", &["a.txt"], "")]), false);
    let record = auto_proposal(&h);
    assert_eq!(
        record.origin,
        ProposalOrigin::Auto {
            stale: vec!["Cargo.lock".into()]
        }
    );
    wait_settled(&h);
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Review m1: a dirty detection checkout that no proposal names (here another
/// repository's, made as the daemon makes one) is salvaged and removed at the start.
#[test]
fn e2e_a_leftover_checkout_without_a_proposal_is_cleaned_at_restart() {
    let mut h = harness();
    let other = h.dir.path().join("other");
    init_repo(&other, &[("a.txt", "a\n")]);
    let other = other.canonicalize().unwrap();
    let git = std::ffi::OsStr::new("git");
    let timeout = Duration::from_secs(30);
    let pre = daemon::run::git::preflight(git, &other, timeout).unwrap();
    let path = daemon::profile::verify::checkout_path(&h.data().join("worktrees"), &other);
    let repo =
        daemon::run::git::checkout_repo_dir(&daemon::profile::repo_dir(&h.data(), &other), &path);
    daemon::profile::verify::prepare(git, &pre, &path, &repo, timeout).unwrap();
    std::fs::write(path.join("work.txt"), "unsaved\n").unwrap();
    h.restart_daemon(&[]);
    assert!(!exists(&path), "{path:?} is left");
    assert!(!exists(&repo), "{repo:?} is left");
    let salvaged = git_in(
        &other,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/anthrex/salvage/onboarding/",
        ],
    );
    assert_eq!(salvaged.lines().count(), 1, "{salvaged}");
    let reference = salvaged.trim();
    assert_eq!(
        git_in(&other, &["show", &format!("{reference}:work.txt")]),
        "unsaved"
    );
}

/// Review I3 (M20): `profile reject` with no work running discards a leftover
/// detection checkout (decision 10).
#[test]
fn e2e_reject_with_no_work_running_discards_leftover_checkouts() {
    let h = harness();
    h.onboarding_report(1, profile());
    ok(h.profile(&["detect"]));
    let status = h.wait_profile(
        "the detection to settle",
        |s| {
            s.proposal
                .as_ref()
                .is_some_and(|r| r.state == ProposalState::Ready)
        },
        PROFILE_WAIT,
    );
    let leftover =
        daemon::profile::verify::checkout_path(&h.data().join("worktrees"), &status.project)
            .with_file_name(".onboarding");
    std::fs::create_dir_all(&leftover).unwrap();
    std::fs::write(leftover.join("junk"), "x\n").unwrap();
    ok(h.profile(&["reject"]));
    assert!(!exists(&leftover), "{leftover:?} is left");
    assert_eq!(h.profile_status().proposal, None);
}
