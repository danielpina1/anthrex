//! Milestone 9.10 task M9.10.3: a verification's progress (decision 10) and the
//! status's new fields `checking`, `verified_at` (decision 23) and `unreadable_text`
//! (decision 32). Real commands in a scratch checkout, and a real service on a scratch
//! data directory and git repository; no agent runs (Claude and Codex are paths that
//! do not exist).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use proto::{
    CheckProgress, ProfileMeta, ProfileReply, ProfileRequest, ProfileStatus, ProfileVerification,
    ProposalState, RepoProfile,
};

use super::store::{self, META_FILE, PROFILE_FILE, UNREADABLE_TEXT_MAX};
use super::tests_ready::{Rig, repo};
use super::tests_tiers::steps;
use super::verify::{CheckCounter, planned, run_commands};

/// How long the status test waits for the edit's verification: `PROFILE_WAIT`'s
/// shape (`cli/tests/support/run_adapt.rs`, `docs/timing-budgets.md`). The command
/// itself sleeps 2 s; the rest is the verification checkout's git steps, each bounded
/// by `git_timeout_secs` (60 s by default), of which a verification runs a handful.
/// Only a failure waits this long; the poll returns as soon as the state is seen.
const STATUS_WAIT: Duration = Duration::from_secs(300);
/// Between two `Status` requests.
const POLL: Duration = Duration::from_millis(20);

fn profile(edit: impl FnOnce(&mut RepoProfile)) -> RepoProfile {
    let mut profile = RepoProfile {
        check: Some("true".into()),
        ..Default::default()
    };
    edit(&mut profile);
    profile
}

#[test]
fn planned_counts_what_run_commands_runs() {
    let cases: Vec<(&str, RepoProfile, u32)> = vec![
        ("check only", profile(|_| {}), 1),
        (
            "setup and check",
            profile(|p| p.setup = Some("true".into())),
            2,
        ),
        (
            "check, single_test, sample_test and test_passed",
            profile(|p| {
                p.single_test = Some("true {test}".into());
                p.sample_test = Some("a::b".into());
                p.test_passed = Some("{test} ok".into());
            }),
            2,
        ),
        (
            "a single_test without {test}",
            profile(|p| {
                p.single_test = Some("true".into());
                p.sample_test = Some("a::b".into());
                p.test_passed = Some("{test} ok".into());
            }),
            1,
        ),
        (
            "check with build_check and module_graph",
            profile(|p| {
                p.build_check = Some("true".into());
                p.module_graph = Some("true".into());
            }),
            3,
        ),
    ];
    for (name, profile, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        let ticks = Arc::new(AtomicU64::new(5));
        let counter = Arc::new(CheckCounter {
            ticks: ticks.clone(),
            ..CheckCounter::default()
        });
        let steps = steps(dir.path()).counted(counter.clone());
        run_commands(
            dir.path(),
            &profile,
            None,
            Duration::from_secs(30),
            0,
            &steps,
        );
        let done = counter.done.load(Ordering::Relaxed);
        assert_eq!(planned(&profile), expected, "{name}: planned");
        assert_eq!(done, expected, "{name}: what run_commands ran");
        assert_eq!(
            ticks.load(Ordering::Relaxed),
            5 + u64::from(done),
            "{name}: one tick per command, on the shared counter"
        );
    }
}

async fn status(rig: &Rig, project: &Path) -> ProfileStatus {
    match rig
        .profiles
        .request(ProfileRequest::Status {
            dir: project.to_path_buf(),
        })
        .await
    {
        ProfileReply::Status(status) => status,
        other => panic!("status: {other:?}"),
    }
}

/// Polls `Status` until `done` holds, failing after [`STATUS_WAIT`].
async fn wait_status(
    rig: &Rig,
    project: &Path,
    what: &str,
    done: impl Fn(&ProfileStatus) -> bool,
) -> ProfileStatus {
    let deadline = Instant::now() + STATUS_WAIT;
    loop {
        let status = status(rig, project).await;
        if done(&status) {
            return status;
        }
        assert!(Instant::now() < deadline, "{what}: last status {status:?}");
        tokio::time::sleep(POLL).await;
    }
}

fn state(status: &ProfileStatus) -> Option<&ProposalState> {
    status.proposal.as_ref().map(|record| &record.state)
}

#[tokio::test(flavor = "multi_thread")]
async fn status_reports_checking_while_verifying() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    let reply = rig
        .profiles
        .request(ProfileRequest::Edit {
            dir: project.clone(),
            key: "check".into(),
            value: Some("sh -c 'sleep 2; true'".into()),
            yes: false,
            unconfined_checks: false,
            anyway: false,
            on_proposal: false,
        })
        .await;
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    let checking = wait_status(&rig, &project, "the check being counted", |s| {
        state(s) == Some(&ProposalState::Verifying)
            && matches!(s.checking, Some(CheckProgress { total: 1, .. }))
    })
    .await;
    let progress = checking.checking.expect("checked above");
    assert!(progress.done <= progress.total, "{progress:?}");
    let ready = wait_status(&rig, &project, "the edit's verification", |s| {
        state(s) == Some(&ProposalState::Ready)
    })
    .await;
    assert_eq!(ready.checking, None, "no progress once it is ready");
}

/// `project`'s data directory, as the status reports it.
async fn repo_dir(rig: &Rig, project: &Path) -> PathBuf {
    let dir = status(rig, project).await.repo_dir;
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread")]
async fn verified_at_is_the_verifications_time_else_the_confirmation() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let dir = repo_dir(&rig, &project).await;
    assert_eq!(status(&rig, &project).await.verified_at, None, "no profile");
    let profile = profile(|_| {});
    let meta = |verification| ProfileMeta {
        confirmed_at: 9,
        report: None,
        verification,
        fingerprint: BTreeMap::new(),
        edited_keys: Vec::new(),
        project: Some(project.clone()),
    };
    let verification = ProfileVerification {
        at: 7,
        confined: false,
        setup: None,
        check: None,
        single_test: None,
        build_check: None,
        module_graph: None,
        module_test: None,
        module_tests: None,
        toolchain_id: None,
    };
    store::save(&dir, &profile, &meta(Some(verification))).unwrap();
    assert_eq!(status(&rig, &project).await.verified_at, Some(7));
    store::save(&dir, &profile, &meta(None)).unwrap();
    assert_eq!(status(&rig, &project).await.verified_at, Some(9));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unparseable_profile_sends_its_text_capped() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let dir = repo_dir(&rig, &project).await;
    assert_eq!(status(&rig, &project).await.unreadable_text, None);

    std::fs::write(dir.join(PROFILE_FILE), "check = [").unwrap();
    let status_now = status(&rig, &project).await;
    assert!(status_now.unparseable.is_some(), "{status_now:?}");
    assert_eq!(status_now.unreadable_text.as_deref(), Some("check = ["));

    // 70 KiB, with a two-byte character across the 64 KiB line.
    let mut big = "check = [\n#".to_string();
    big.push_str(&"#".repeat(UNREADABLE_TEXT_MAX - 1 - big.len()));
    big.push('é');
    big.push_str(&"x".repeat(70 * 1024 - big.len()));
    std::fs::write(dir.join(PROFILE_FILE), &big).unwrap();
    let text = status(&rig, &project)
        .await
        .unreadable_text
        .expect("the text is sent");
    let note = "\n… (cut at 64 KiB)";
    assert!(text.ends_with(note), "{:?}", &text[text.len() - 40..]);
    let body = &text[..text.len() - note.len()];
    assert_eq!(body, &big[..UNREADABLE_TEXT_MAX - 1], "cut at a character");
    assert_eq!(UNREADABLE_TEXT_MAX, 64 * 1024);

    // A broken meta file is the unreadable file then; its text is what is sent.
    std::fs::write(dir.join(PROFILE_FILE), "check = \"true\"\n").unwrap();
    std::fs::write(dir.join(META_FILE), "{ not json").unwrap();
    let status_now = status(&rig, &project).await;
    assert!(status_now.unparseable.is_some(), "{status_now:?}");
    assert_eq!(status_now.unreadable_text.as_deref(), Some("{ not json"));

    std::fs::remove_file(dir.join(META_FILE)).unwrap();
    let parsed = status(&rig, &project).await;
    assert_eq!(parsed.unparseable, None);
    assert_eq!(parsed.unreadable_text, None, "set only with unparseable");
}
