//! Milestone 9.10 task M9.10.6: row edits (decisions 15 to 22). An edited command is
//! checked once, then saved, saved anyway or reverted, on the stored profile or on a
//! review proposal. Real verification commands in a scratch checkout, a real service on
//! a scratch data directory and a real git repository; no agent runs (Claude and Codex
//! are paths that do not exist).

use std::path::Path;
use std::time::{Duration, Instant};

use proto::{
    CommandCheck, DeliveryMode, DeliveryProfile, DroppedCommand, ProfileMeta, ProfileReply,
    ProfileRequest, ProfileVerification, ProposalOrigin, ProposalRecord, ProposalState,
    RepoProfile, RowEdit, RowEditState,
};

use super::store::{self, Stored};
use super::tests_queue_service::done;
use super::tests_ready::{Rig, record, repo};

/// How long a test waits for a row edit's verification: `PROFILE_WAIT`'s shape
/// (`cli/tests/support/run_adapt.rs`, `docs/timing-budgets.md`). The commands are
/// `true`, `false` or a 2 s sleep; the rest is the verification checkout's git steps,
/// each bounded by `git_timeout_secs` (60 s by default), of which a verification runs a
/// handful. Only a failure waits this long; the poll returns as soon as the state is seen.
const PROFILE_WAIT: Duration = Duration::from_secs(300);
/// Between two looks.
const POLL: Duration = Duration::from_millis(20);

pub(super) fn edit(project: &Path, key: &str, value: Option<&str>) -> ProfileRequest {
    ProfileRequest::Edit {
        dir: project.to_path_buf(),
        key: key.to_string(),
        value: value.map(str::to_string),
        // The TUI always sends `yes: true` (decision 16).
        yes: true,
        // Off macOS no sandbox confines a check; these tests are not about that.
        unconfined_checks: !cfg!(target_os = "macos"),
        anyway: false,
        on_proposal: false,
    }
}

/// `request` with `anyway` (which implies `yes`) and/or `on_proposal` set.
pub(super) fn with(
    mut request: ProfileRequest,
    anyway_: bool,
    on_proposal_: bool,
) -> ProfileRequest {
    if let ProfileRequest::Edit {
        anyway,
        on_proposal,
        ..
    } = &mut request
    {
        (*anyway, *on_proposal) = (anyway_, on_proposal_);
    }
    request
}

pub(super) fn refused(reply: ProfileReply) -> String {
    match reply {
        ProfileReply::Refused { message } => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

pub(super) fn revert(project: &Path) -> ProfileRequest {
    ProfileRequest::RevertEdit {
        dir: project.to_path_buf(),
    }
}

/// Polls `check` every `POLL` until it holds, at most `PROFILE_WAIT`.
pub(super) async fn wait(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + PROFILE_WAIT;
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(POLL).await;
    }
}

/// Waits until `project`'s work has ended (it can be registered again), so whatever it
/// was going to write is written.
pub(super) async fn work_ended(rig: &Rig, project: &Path) {
    wait("the work to end", || {
        rig.profiles
            .register(project)
            .is_some_and(|(generation, _)| {
                rig.profiles.unregister(project, generation);
                true
            })
    })
    .await;
}

/// How long a held ✗ is watched for a store that must not come: the store would be
/// one `confirm_record` (a few small file writes) in the same task, before the work
/// ends, so milliseconds; `work_ended` already waited past it.
const HELD_WINDOW: Duration = Duration::from_millis(300);

pub(super) fn stored(rig: &Rig, project: &Path) -> Option<(RepoProfile, ProfileMeta)> {
    match store::load(&rig.repo_dir(project)) {
        Stored::Found { profile, meta, .. } => Some((profile, meta)),
        _ => None,
    }
}

pub(super) fn stored_check(rig: &Rig, project: &Path) -> Option<String> {
    stored(rig, project).and_then(|(profile, _)| profile.check)
}

pub(super) fn edit_state(rig: &Rig, project: &Path) -> Option<RowEditState> {
    rig.on_disk(project)?.edit.map(|edit| edit.state)
}

pub(super) fn failed(state: Option<RowEditState>) -> bool {
    matches!(state, Some(RowEditState::Failed { .. }))
}

pub(super) fn check_of(command: &str, ok: bool) -> CommandCheck {
    CommandCheck {
        command: command.to_string(),
        ok,
        code: Some(if ok { 0 } else { 1 }),
        timed_out: false,
        secs: 0,
        tail: String::new(),
    }
}

/// A `Ready` review proposal on disk and in the table, as a detection leaves it:
/// `check = "true"` verified, a `proposed` that differs from its `profile`, and a
/// `[delivery]` table that a row edit must keep.
pub(super) fn review(
    rig: &Rig,
    project: &Path,
    origin: ProposalOrigin,
    dropped: bool,
) -> ProposalRecord {
    let mut ready = record(project, ProposalState::Ready, 1_790_000_000);
    ready.origin = origin;
    ready.profile = Some(RepoProfile {
        check: Some("true".into()),
        delivery: Some(DeliveryProfile {
            mode: DeliveryMode::Pr,
            remote: "origin".into(),
        }),
        ..Default::default()
    });
    ready.proposed = Some(RepoProfile {
        check: Some("true".into()),
        source: vec!["src/**".into(), "lib/[".into()],
        ..Default::default()
    });
    ready.verification = Some(ProfileVerification {
        at: 1_790_000_000,
        confined: false,
        setup: None,
        check: Some(check_of("true", true)),
        single_test: None,
        build_check: None,
        module_graph: None,
        module_test: None,
        module_tests: None,
        toolchain_id: None,
    });
    if dropped {
        ready.dropped = vec![DroppedCommand {
            key: "build_check".into(),
            command: "make quick".into(),
            reason: "exit 2 after 1s".into(),
            tail: "no rule".into(),
        }];
    }
    put(rig, &ready);
    ready
}

pub(super) fn put(rig: &Rig, record: &ProposalRecord) {
    let dir = rig.repo_dir(&record.project);
    std::fs::create_dir_all(&dir).unwrap();
    store::save_proposal(&dir, record).unwrap();
    rig.profiles.note_proposal(&record.project, Some(record));
}

/// `profile edit check false` on the stored profile, waited until its ✗ is recorded.
pub(super) async fn failed_check_edit(rig: &Rig, project: &Path) -> ProposalRecord {
    rig.store_profile(project);
    let message = done(
        rig.profiles
            .request(edit(project, "check", Some("false")))
            .await,
    );
    assert_eq!(
        message,
        "proposed: check = false; it is stored as soon as verification passes (anthrex profile status)"
    );
    wait("the edit's ✗", || failed(edit_state(rig, project))).await;
    rig.on_disk(project).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_passing_command_edit_is_stored() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    done(
        rig.profiles
            .request(edit(&project, "check", Some("true && true")))
            .await,
    );
    // The store writes the profile, then deletes the proposal.
    wait("the edit's outcome", || {
        rig.on_disk(&project)
            .is_none_or(|record| failed(record.edit.map(|e| e.state)))
    })
    .await;
    assert_eq!(rig.on_disk(&project), None, "no proposal is left");
    assert_eq!(
        stored_check(&rig, &project).as_deref(),
        Some("true && true")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_command_edit_is_held_failed() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let held = failed_check_edit(&rig, &project).await;
    assert_eq!(held.state, ProposalState::Ready);
    assert_eq!(
        held.origin,
        ProposalOrigin::Edit {
            keys: vec!["check".into()]
        }
    );
    let edit = held.edit.expect("the row edit is kept");
    assert_eq!(
        (edit.key.as_str(), edit.value.as_deref()),
        ("check", Some("false"))
    );
    match edit.state {
        RowEditState::Failed { reason, .. } => {
            assert!(reason.starts_with("exit 1 after "), "{reason}")
        }
        other => panic!("{other:?}"),
    }
    // Fix round M4: held, not stored, for as long as the work could still store it.
    work_ended(&rig, &project).await;
    let deadline = Instant::now() + HELD_WINDOW;
    while Instant::now() < deadline {
        assert_eq!(stored_check(&rig, &project).as_deref(), Some("true"));
        assert!(failed(edit_state(&rig, &project)), "the ✗ stays held");
        tokio::time::sleep(POLL).await;
    }
    assert!(rig.profiles.ready_proposals().1.is_empty(), "not an alert");
}

/// Fix round I1 (decision 15): a held ✗ of the stored profile is never stored, not by
/// `profile confirm`/`use` nor by a `--yes` goal's use; the refusal names the way out.
/// The edit's own reply no longer sends the user to `confirm` unconditionally.
#[tokio::test(flavor = "multi_thread")]
async fn a_held_failed_edit_is_never_confirmed() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    let mut request = edit(&project, "check", Some("false"));
    if let ProfileRequest::Edit { yes, .. } = &mut request {
        *yes = false;
    }
    let message = done(rig.profiles.request(request).await);
    assert_eq!(
        message,
        "proposed: check = false; verifying (anthrex profile status); once it passes, confirm \
         with anthrex profile confirm; if it fails, save it anyway with --anyway or discard \
         it with anthrex profile reject"
    );
    wait("the edit's ✗", || failed(edit_state(&rig, &project))).await;
    work_ended(&rig, &project).await;
    let held = rig.on_disk(&project).unwrap();
    let way_out = "the edit of check failed its check; save it anyway (s, or anthrex profile \
                   edit check false --anyway) or revert it (r, or anthrex profile reject)";
    let confirm = ProfileRequest::Confirm {
        dir: project.clone(),
        shown: None,
    };
    assert_eq!(refused(rig.profiles.request(confirm).await), way_out);
    assert_eq!(
        rig.profiles.use_ready(&project).await,
        Err(way_out.to_string())
    );
    assert_eq!(stored_check(&rig, &project).as_deref(), Some("true"));
    assert_eq!(rig.on_disk(&project), Some(held));
}

/// Fix round I1: the way out quotes a value the shell would split, and names `--unset`.
#[test]
fn the_way_out_is_a_command_to_paste() {
    use super::row_edit::held_failed;
    assert_eq!(
        held_failed("check", Some("sh it's.sh")),
        "the edit of check failed its check; save it anyway (s, or anthrex profile edit \
         check 'sh it'\\''s.sh' --anyway) or revert it (r, or anthrex profile reject)"
    );
    assert_eq!(
        held_failed("setup", None),
        "the edit of setup failed its check; save it anyway (s, or anthrex profile edit \
         setup --unset --anyway) or revert it (r, or anthrex profile reject)"
    );
}

/// Fix round M5: an edit of the stored profile the restart interrupted is failed whole,
/// its row edit too; it can be reverted, and it is never confirmed.
#[tokio::test(flavor = "multi_thread")]
async fn restore_fails_an_interrupted_stored_profile_edit() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    let mut checking = record(&project, ProposalState::Verifying, 1_790_000_000);
    checking.origin = ProposalOrigin::Edit {
        keys: vec!["check".into()],
    };
    checking.profile = None;
    checking.proposed = Some(RepoProfile {
        check: Some("true && true".into()),
        ..Default::default()
    });
    checking.edit = Some(RowEdit {
        key: "check".into(),
        value: Some("true && true".into()),
        state: RowEditState::Verifying,
    });
    put(&rig, &checking);
    let profiles = super::tests_ready::service(rig.dir.path());
    profiles.restore().await;
    let restored = rig.on_disk(&project).unwrap();
    assert_eq!(
        restored.state,
        ProposalState::Failed {
            reason: super::service::RESTART_REASON.to_string()
        }
    );
    assert_eq!(
        restored.edit.map(|e| e.state),
        Some(RowEditState::Failed {
            reason: super::service::EDIT_RESTART_REASON.to_string(),
            tail: String::new(),
            secs: 0,
        })
    );
    let confirm = ProfileRequest::Confirm {
        dir: project.clone(),
        shown: None,
    };
    assert_eq!(
        refused(profiles.request(confirm).await),
        super::row_edit::held_failed("check", Some("true && true"))
    );
    let message = done(profiles.request(revert(&project)).await);
    assert_eq!(message, "reverted check; the profile is unchanged");
    assert_eq!(rig.on_disk(&project), None);
    assert_eq!(stored_check(&rig, &project).as_deref(), Some("true"));
}

#[tokio::test(flavor = "multi_thread")]
async fn save_anyway_stores_the_failed_edit_without_verifying_again() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    failed_check_edit(&rig, &project).await;
    let anyway = with(edit(&project, "check", Some("false")), true, false);
    let message = done(rig.profiles.request(anyway).await);
    assert_eq!(
        message,
        "proposed: check = false; stored although its check failed"
    );
    // At once: no verification ran, so nothing was left `Verifying`.
    assert_eq!(rig.on_disk(&project), None);
    let (profile, meta) = stored(&rig, &project).unwrap();
    assert_eq!(profile.check.as_deref(), Some("false"));
    let check = meta.verification.and_then(|v| v.check).expect("its ✗ kept");
    assert_eq!((check.command.as_str(), check.ok), ("false", false));
}

#[tokio::test(flavor = "multi_thread")]
async fn revert_discards_a_failed_edit() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    failed_check_edit(&rig, &project).await;
    let message = done(rig.profiles.request(revert(&project)).await);
    assert_eq!(message, "reverted check; the profile is unchanged");
    assert_eq!(rig.on_disk(&project), None);
    assert_eq!(stored_check(&rig, &project).as_deref(), Some("true"));
}

#[tokio::test(flavor = "multi_thread")]
async fn revert_is_refused_while_checking() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let mut checking = review(&rig, &project, ProposalOrigin::Detect, false);
    checking.edit = Some(RowEdit {
        key: "check".into(),
        value: Some("true && true".into()),
        state: RowEditState::Verifying,
    });
    put(&rig, &checking);
    assert_eq!(
        refused(rig.profiles.request(revert(&project)).await),
        "the edit of check is still being checked; wait for it"
    );
    assert_eq!(rig.on_disk(&project), Some(checking));
}

#[tokio::test(flavor = "multi_thread")]
async fn revert_without_a_failed_edit_is_refused() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let expected = format!("no failed edit to revert for {}", project.display());
    assert_eq!(
        refused(rig.profiles.request(revert(&project)).await),
        expected
    );
    let ready = review(&rig, &project, ProposalOrigin::Detect, false);
    assert_eq!(
        refused(rig.profiles.request(revert(&project)).await),
        expected
    );
    assert_eq!(rig.on_disk(&project), Some(ready));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_command_edit_is_stored_at_once() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    let message = done(
        rig.profiles
            .request(edit(&project, "source", Some("[\"src/**\"]")))
            .await,
    );
    assert_eq!(
        message,
        "proposed: source = [\"src/**\"]; stored (it needed no verification)"
    );
    assert_eq!(rig.on_disk(&project), None);
    assert_eq!(stored(&rig, &project).unwrap().0.source, ["src/**"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stored_profile_edit_is_refused_while_a_review_proposal_waits() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    let auto = ProposalOrigin::Auto {
        stale: vec!["Cargo.toml".into()],
    };
    review(&rig, &project, auto, false);
    let file = rig.repo_dir(&project).join(store::PROPOSAL_FILE);
    let before = std::fs::read(&file).unwrap();
    assert_eq!(
        refused(
            rig.profiles
                .request(edit(&project, "check", Some("true && true")))
                .await
        ),
        format!(
            "{} has a proposal waiting for review; review it first (C-b P, or anthrex profile use or reject)",
            project.display()
        )
    );
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_passing_edit_without_yes_stays_a_proposal() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    let mut request = edit(&project, "check", Some("true && true"));
    if let ProfileRequest::Edit { yes, .. } = &mut request {
        *yes = false;
    }
    done(rig.profiles.request(request).await);
    wait("the edit's check", || {
        rig.on_disk(&project)
            .is_some_and(|r| r.state == ProposalState::Ready)
    })
    .await;
    let record = rig.on_disk(&project).unwrap();
    assert_eq!(
        record.origin,
        ProposalOrigin::Edit {
            keys: vec!["check".into()]
        }
    );
    assert_eq!(record.edit, None);
    assert_eq!(stored_check(&rig, &project).as_deref(), Some("true"));
}

/// `profile edit check false` is the command `false` (the brief's own value), while a
/// value that fits neither as typed nor as text keeps the first error.
#[test]
fn a_command_that_reads_as_a_toml_value_is_text() {
    let stored = RepoProfile::default();
    let (edited, reverify) = super::proposal::apply_edit(&stored, "check", Some("false")).unwrap();
    assert_eq!((edited.check.as_deref(), reverify), (Some("false"), true));
    let (edited, _) = super::proposal::apply_edit(&stored, "check", Some("\"false\"")).unwrap();
    assert_eq!(edited.check.as_deref(), Some("false"));
    let refused = super::proposal::apply_edit(&stored, "check_timeout_secs", Some("true"));
    assert_eq!(
        refused.unwrap_err(),
        "check_timeout_secs: invalid type: boolean `true`, expected u64"
    );
    // Fix round M1: only a boolean or a number is retried as text.
    let (edited, _) = super::proposal::apply_edit(&stored, "check", Some("3")).unwrap();
    assert_eq!(edited.check.as_deref(), Some("3"));
    for value in ["[\"cargo\", \"test\"]", "{ a = 1 }"] {
        let refused = super::proposal::apply_edit(&stored, "check", Some(value));
        assert!(refused.is_err(), "{value}: {refused:?}");
    }
}
