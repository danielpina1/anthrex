//! Master's review of the Use-this race fix (#93): background profile work that panics
//! still ends its registration (`service_tail.rs`, `Registration`). Before, a panic
//! skipped `unregister`: the `Active` entry stayed, so the project refused `detect` and
//! `confirm` (Use this) for as long as the daemon ran, `reject` included, and after a
//! detection's `Ready` each such request first waited out the tail bound.
//!
//! Each guarded function is spawned for real, as the service spawns it, so tokio catches
//! the panic; a seam flag (`TailSeam`'s `panic_*`) picks where it panics:
//! - `detect_in_background` in `save_if_current`'s `ready_gap`, at its first save,
//!   `advance(Scouting)`, after the onboarding checkout is prepared and before any agent;
//! - `verify_in_background` with a `Detect` origin at the start of `after_ready`, after
//!   its verification published `Ready`;
//! - `verify_row_edit` (a review proposal's row edit) after its verification.
//!
//! Each test asserts the registration has ended as soon as the task is joined, then that
//! the project takes new work. A real service on a scratch data directory, a real git
//! repository and real verifications of `true`; no agent runs (Claude and Codex are
//! paths that do not exist).

use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::{
    ProfileReply, ProfileRequest, ProposalOrigin, ProposalRecord, ProposalState, RepoProfile,
    RowEdit, RowEditState,
};

use super::service_run::Job;
use super::service_tail::TailSeam;
use super::store::{self, Stored};
use super::tests_edit::{put, review};
use super::tests_ready::{Rig, record, repo};
use crate::run::driver::unix_now;

/// One request of the service: a git call and a few small file reads on
/// `spawn_blocking`; only a failure waits this long (`tests_ready_tail.rs`'s bound).
const REQUEST_WAIT: Duration = Duration::from_secs(75);

/// A tail bound no test reaches unless a request waits on a registration that a panic
/// left behind.
const LONG_BOUND: Duration = Duration::from_secs(3600);

/// Sets the seam: the long tail bound, and `flag` on.
fn arm(rig: &Rig, flag: fn(&mut TailSeam) -> &mut bool) {
    let mut seam = crate::lock(&rig.profiles.tail_seam);
    seam.bound = Some(LONG_BOUND);
    *flag(&mut seam) = true;
}

/// Turns `flag` off again, so the next request's own work does not panic.
fn disarm(rig: &Rig, flag: fn(&mut TailSeam) -> &mut bool) {
    *flag(&mut crate::lock(&rig.profiles.tail_seam)) = false;
}

/// A job for `project` registered now, its proposal `record`.
async fn job(rig: &Rig, project: &Path, mut record: ProposalRecord) -> Job {
    let pre = rig.profiles.preflight(project.to_path_buf()).await.unwrap();
    let (generation, token) = rig.profiles.register(project).expect("nothing runs");
    record.base_sha = pre.base_sha.clone();
    Job {
        generation,
        token,
        pre,
        record,
        codex_config: Vec::new(),
        route: None,
        anyway: false,
    }
}

/// Joins `task`, which the seam must have panicked, then asserts that the panic ended
/// `project`'s registration: before the guard this fails at once.
async fn joined_panicked(rig: &Rig, project: &Path, task: tokio::task::JoinHandle<()>) {
    // The work's own worst case (a verification of `true` under `verify_timeout_secs`,
    // and its checkout's git calls), as a hang guard only.
    let verify = Duration::from_secs(rig.profiles.ctx.orchestrator.onboarding.verify_timeout_secs);
    let ended = tokio::time::timeout(verify + REQUEST_WAIT, task)
        .await
        .expect("the work ends");
    let error = ended.expect_err("the seam panics the work");
    assert!(error.is_panic(), "{error:?}");
    assert!(
        !rig.profiles.registered(project),
        "the panic left the registration"
    );
}

/// A `Detect`-origin proposal for `project` verified to `Ready` by its own background
/// task, which then panics in `after_ready`. Returns once the task has ended with the
/// panic, with the `Ready` proposal on disk.
async fn panicked_after_ready(rig: &Rig) -> PathBuf {
    arm(rig, |seam| &mut seam.panic_after_ready);
    let project = repo(rig.dir.path(), "app");
    let mut record = record(&project, ProposalState::Preparing, unix_now());
    record.profile = None;
    record.proposed = Some(RepoProfile {
        check: Some("true".into()),
        ..Default::default()
    });
    let job = job(rig, &project, record).await;
    let task = tokio::spawn(rig.profiles.clone().verify_in_background(job));
    joined_panicked(rig, &project, task).await;
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

/// `detect` answers `Done`: accepted, not refused as already running, and never parked
/// on the tail. The request's own legal worst case (`project_of`'s git call within
/// `git_timeout`, and file work), an hour below the bound. Then the detection it started
/// (its agent does not exist) is stopped.
async fn detect_is_accepted(rig: &Rig, project: &Path) {
    let limit = rig.profiles.git_timeout() + REQUEST_WAIT;
    assert!(limit < LONG_BOUND);
    let detect = ProfileRequest::Detect {
        dir: project.to_path_buf(),
        trust_project: false,
        unconfined_checks: true,
    };
    let reply = tokio::time::timeout(limit, rig.profiles.request(detect))
        .await
        .expect("detect answers at once, not after the tail bound");
    assert_eq!(parked(rig), 0, "detect waited on a dead registration");
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    reject(rig, project).await;
}

async fn reject(rig: &Rig, project: &Path) -> ProfileReply {
    let reject = ProfileRequest::Reject {
        dir: project.to_path_buf(),
    };
    rig.profiles.request(reject).await
}

/// Review I1: `detect_in_background` itself holds the guard. Its first save panics, and
/// the project takes a new detection.
#[tokio::test(flavor = "multi_thread")]
async fn a_detection_that_panics_before_ready_ends_its_registration() {
    let rig = Rig::new();
    arm(&rig, |seam| &mut seam.panic_in_gap);
    let project = repo(rig.dir.path(), "app");
    let job = job(
        &rig,
        &project,
        record(&project, ProposalState::Preparing, 1),
    )
    .await;
    let task = tokio::spawn(rig.profiles.clone().detect_in_background(job));
    joined_panicked(&rig, &project, task).await;
    assert!(
        rig.on_disk(&project).is_none(),
        "the panic came before the Scouting write"
    );

    disarm(&rig, |seam| &mut seam.panic_in_gap);
    detect_is_accepted(&rig, &project).await;
}

/// Use this after the panic stores the proposal at once.
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
}

/// A new `detect` after the panic is accepted, not refused as already running.
#[tokio::test(flavor = "multi_thread")]
async fn a_detect_after_a_detection_panicked_past_ready_is_accepted() {
    let rig = Rig::new();
    let project = panicked_after_ready(&rig).await;
    disarm(&rig, |seam| &mut seam.panic_after_ready);
    detect_is_accepted(&rig, &project).await;
}

/// Review I2: a review proposal's row edit that panics ends its registration, so the
/// project is not refused until a restart: `reject` cleans it up and a new detection is
/// accepted. Before, `reject` only cancelled the entry, and every later request found it.
#[tokio::test(flavor = "multi_thread")]
async fn a_row_edit_that_panics_ends_its_registration() {
    let rig = Rig::new();
    arm(&rig, |seam| &mut seam.panic_in_row_edit);
    let project = repo(rig.dir.path(), "app");
    let ready = review(&rig, &project, ProposalOrigin::Detect, false);
    let mut checking = ready.clone();
    checking.edit = Some(RowEdit {
        key: "check".into(),
        value: Some("true && true".into()),
        state: RowEditState::Verifying,
    });
    put(&rig, &checking);
    let job = job(&rig, &project, checking).await;
    let edited = RepoProfile {
        check: Some("true && true".into()),
        ..ready.profile.clone().unwrap()
    };
    let task = tokio::spawn(rig.profiles.clone().verify_row_edit(job, edited));
    joined_panicked(&rig, &project, task).await;

    disarm(&rig, |seam| &mut seam.panic_in_row_edit);
    let reply = reject(&rig, &project).await;
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    assert!(rig.on_disk(&project).is_none(), "the proposal is kept");
    detect_is_accepted(&rig, &project).await;
}
