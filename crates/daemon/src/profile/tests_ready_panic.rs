//! Master's review of the Use-this race fix (#93): a detection that panics after it
//! published `Ready` still ends its registration (`service_tail.rs`, `Registration`).
//! Before, the panic skipped `unregister`: the `Active` entry stayed, so every later
//! `confirm` (Use this) or `detect` waited out the tail bound, then refused "already
//! running (state ready)" for as long as the daemon ran.
//!
//! The detection's own background work runs on a spawned task, as `start_detection`
//! spawns it, so tokio catches the panic; the seam (`TailSeam.panic_after_ready`) panics
//! at the start of `after_ready`. A real service on a scratch data directory, a real git
//! repository and a real verification of `check = "true"`; no agent runs (Claude and
//! Codex are paths that do not exist). `verify_in_background` is driven, not
//! `detect_in_background`: reaching `Ready` through the latter needs an onboarding scout's
//! report. Both end their registration through the same `Registration`, and a `Detect`
//! origin takes `verify_phase`'s `after_ready` in either.

use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::{ProfileReply, ProfileRequest, ProposalState, RepoProfile};

use super::service_run::Job;
use super::store::{self, Stored};
use super::tests_ready::{Rig, record, repo};
use crate::run::driver::unix_now;

/// One request of the service: a git call and a few small file reads on
/// `spawn_blocking`; only a failure waits this long (`tests_ready_tail.rs`'s bound).
const REQUEST_WAIT: Duration = Duration::from_secs(75);

/// A tail bound no test reaches unless a request waits on a registration that a panic
/// left behind.
const LONG_BOUND: Duration = Duration::from_secs(3600);

/// A `Detect`-origin proposal for `project` verified to `Ready` by its own background
/// task, which then panics in `after_ready`. Returns once the task has ended with the
/// panic, with the `Ready` proposal on disk.
async fn panicked_after_ready(rig: &Rig) -> PathBuf {
    {
        let mut seam = crate::lock(&rig.profiles.tail_seam);
        seam.bound = Some(LONG_BOUND);
        seam.panic_after_ready = true;
    }
    let project = repo(rig.dir.path(), "app");
    let pre = rig.profiles.preflight(project.clone()).await.unwrap();
    let (generation, token) = rig.profiles.register(&project).expect("nothing runs");
    let mut record = record(&project, ProposalState::Preparing, unix_now());
    record.profile = None;
    record.proposed = Some(RepoProfile {
        check: Some("true".into()),
        ..Default::default()
    });
    record.base_sha = pre.base_sha.clone();
    let job = Job {
        generation,
        token,
        pre,
        record,
        codex_config: Vec::new(),
        route: None,
        anyway: false,
    };
    // The verification's own worst case (one `true` under `verify_timeout_secs`, and its
    // checkout's git calls), as a hang guard only.
    let verify = Duration::from_secs(rig.profiles.ctx.orchestrator.onboarding.verify_timeout_secs);
    let task = tokio::spawn(rig.profiles.clone().verify_in_background(job));
    let ended = tokio::time::timeout(verify + REQUEST_WAIT, task)
        .await
        .expect("the detection ends");
    let error = ended.expect_err("the seam panics the detection");
    assert!(error.is_panic(), "{error:?}");
    let state = rig.on_disk(&project).map(|r| r.state);
    assert_eq!(
        state,
        Some(ProposalState::Ready),
        "the panic came after Ready"
    );
    project
}

fn parked(rig: &Rig) -> usize {
    crate::lock(&rig.profiles.tail_seam).parked
}

/// Use this after the panic stores the proposal at once: the request's own legal worst
/// case (`project_of`'s git call within `git_timeout`, and file work), never the bound.
#[tokio::test(flavor = "multi_thread")]
async fn a_confirm_after_a_detection_panicked_past_ready_does_not_wait() {
    let rig = Rig::new();
    let project = panicked_after_ready(&rig).await;

    let limit = rig.profiles.git_timeout() + REQUEST_WAIT;
    assert!(limit < LONG_BOUND);
    let confirm = ProfileRequest::Confirm {
        dir: project.clone(),
        shown: None,
    };
    let reply = tokio::time::timeout(limit, rig.profiles.request(confirm))
        .await
        .expect("confirm answers at once, not after the tail bound");
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    assert_eq!(parked(&rig), 0, "confirm waited on a dead registration");
    assert!(
        rig.on_disk(&project).is_none(),
        "the proposal is still on disk"
    );
    let stored = store::load(&rig.repo_dir(&project));
    assert!(matches!(stored, Stored::Found { .. }), "nothing stored");
    assert!(!rig.profiles.registered(&project));
}

/// A new `detect` after the panic is accepted, not refused as already running.
#[tokio::test(flavor = "multi_thread")]
async fn a_detect_after_a_detection_panicked_past_ready_is_accepted() {
    let rig = Rig::new();
    let project = panicked_after_ready(&rig).await;

    let limit = rig.profiles.git_timeout() + REQUEST_WAIT;
    let detect = ProfileRequest::Detect {
        dir: project.clone(),
        trust_project: false,
        unconfined_checks: true,
    };
    let reply = tokio::time::timeout(limit, rig.profiles.request(detect))
        .await
        .expect("detect answers at once, not after the tail bound");
    assert_eq!(parked(&rig), 0, "detect waited on a dead registration");
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    stop(&rig, &project).await;
}

/// Whatever the new detection started (its agent does not exist), stop it.
async fn stop(rig: &Rig, project: &Path) {
    let reject = ProfileRequest::Reject {
        dir: project.to_path_buf(),
    };
    let _ = rig.profiles.request(reject).await;
}
