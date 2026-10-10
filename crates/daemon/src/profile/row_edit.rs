//! A row edit's records (milestone 9.10 decisions 15 to 19), with no I/O: decision 17's
//! ✓/✗, the outcome a row edit's check writes, **Save anyway** on a proposal (R22), and
//! the refusal texts. `service_edit.rs` does the requests' I/O.

use std::path::Path;

use proto::{
    CommandCheck, DroppedCommand, ProfileVerification, ProposalOrigin, ProposalRecord, RepoProfile,
    RowEdit, RowEditState,
};

use super::proposal::failure;
use super::service_run::{Job, Stop};

/// What a verification found: the verified profile, the record and what it dropped.
pub(super) type Verified = Result<(RepoProfile, ProfileVerification, Vec<DroppedCommand>), Stop>;

/// Decision 3: a review proposal is any but an edit of the stored profile.
pub(super) fn is_review(record: &ProposalRecord) -> bool {
    !matches!(record.origin, ProposalOrigin::Edit { .. })
}

/// Decision 19's refusal while the row edit of `key` verifies.
pub(super) fn still_checking(key: &str) -> String {
    format!("the edit of {key} is still being checked; wait for it")
}

pub(super) fn review_waiting(project: &Path) -> String {
    format!(
        "{} has a proposal waiting for review; review it first (C-b P, or anthrex profile use or reject)",
        project.display()
    )
}

pub(super) fn no_proposal_to_edit(project: &Path) -> String {
    format!(
        "no proposal to edit for {}; run anthrex profile detect",
        project.display()
    )
}

pub(super) fn no_failed_edit(project: &Path) -> String {
    format!("no failed edit to revert for {}", project.display())
}

/// The proposal changed between the request's checks and its write.
pub(super) fn changed(project: &Path) -> String {
    format!(
        "the proposal for {} changed meanwhile; try again",
        project.display()
    )
}

/// The verification slot of a command key; `None` for a key that is not a command.
pub(super) fn command_slot<'a>(
    v: &'a mut ProfileVerification,
    key: &str,
) -> Option<&'a mut Option<CommandCheck>> {
    Some(match key {
        "setup" => &mut v.setup,
        "check" => &mut v.check,
        "single_test" => &mut v.single_test,
        "build_check" => &mut v.build_check,
        "module_graph" => &mut v.module_graph,
        "module_test" => &mut v.module_test,
        "module_tests" => &mut v.module_tests,
        "toolchain_id" => &mut v.toolchain_id,
        _ => return None,
    })
}

/// The command `profile` holds for a command key.
pub(super) fn command_of(profile: &RepoProfile, key: &str) -> Option<String> {
    match key {
        "setup" => profile.setup.clone(),
        "check" => profile.check.clone(),
        "single_test" => profile.single_test.clone(),
        "build_check" => profile.build_check.clone(),
        "module_graph" => profile.module_graph.clone(),
        "module_test" => profile.module_test.clone(),
        "module_tests" => profile.module_tests.clone(),
        "toolchain_id" => profile.toolchain_id.clone(),
        _ => None,
    }
}

/// Decision 17, pure: the edited key's check failed or timed out, or verification
/// dropped any other command. `None` is ✓.
pub(super) fn row_failure(
    key: &str,
    verification: Option<&ProfileVerification>,
    dropped: &[DroppedCommand],
) -> Option<RowEditState> {
    let check = verification.and_then(|v| command_slot(&mut v.clone(), key).and_then(Option::take));
    let reason = match (dropped.iter().find(|d| d.key == key), dropped.first()) {
        (Some(own), _) => own.reason.clone(),
        (None, Some(other)) => format!("another command stopped passing: {}", other.key),
        (None, None) => match &check {
            Some(check) if !check.ok => failure(check),
            _ => return None,
        },
    };
    let (tail, secs) = check.map_or((String::new(), 0), |c| (c.tail, c.secs));
    Some(RowEditState::Failed { reason, tail, secs })
}

/// Decision 18: the `tail` and `secs` of `record`'s ✗ row edit of `key` set to `value`.
pub(super) fn failed_edit(
    record: &ProposalRecord,
    key: &str,
    value: Option<&str>,
) -> Option<(String, u64)> {
    match &record.edit {
        Some(RowEdit {
            key: k,
            value: v,
            state: RowEditState::Failed { tail, secs, .. },
        }) if k == key && v.as_deref() == value => Some((tail.clone(), *secs)),
        _ => None,
    }
}

/// Decision 18 on a proposal (R22): `edited` written into the proposal, and the key's ✗
/// check rebuilt from the row edit's `tail` and `secs`; the key leaves `dropped`.
pub(super) fn saved_anyway(
    record: &mut ProposalRecord,
    edited: RepoProfile,
    key: &str,
    (tail, secs): (String, u64),
    (now, confined): (u64, bool),
) {
    if let Some(command) = command_of(&edited, key) {
        let v = record.verification.get_or_insert(ProfileVerification {
            at: now,
            confined,
            setup: None,
            check: None,
            single_test: None,
            build_check: None,
            module_graph: None,
            module_test: None,
            module_tests: None,
            toolchain_id: None,
        });
        if let Some(slot) = command_slot(v, key) {
            *slot = Some(CommandCheck {
                command,
                ok: false,
                code: None,
                timed_out: false,
                secs,
                tail,
            });
        }
    }
    record.profile = Some(edited);
    record.dropped.retain(|d| d.key != key);
    record.edit = None;
}

/// Decision 15 on the stored profile, at `Ready`: ✓ clears the row edit, ✗ (decision 17;
/// `failed`, a verification that could not run) marks it `Failed`.
pub(super) fn settle_row_edit(record: &mut ProposalRecord, failed: Option<String>) {
    let Some(key) = record.edit.as_ref().map(|edit| edit.key.clone()) else {
        return;
    };
    let state = match failed {
        Some(reason) => Some(RowEditState::Failed {
            reason,
            tail: String::new(),
            secs: 0,
        }),
        None => row_failure(&key, record.verification.as_ref(), &record.dropped),
    };
    match (state, record.edit.as_mut()) {
        (Some(state), Some(edit)) => edit.state = state,
        _ => record.edit = None,
    }
}

/// Decision 15 on a proposal: the record `verify_row_edit` writes, `None` when rejected.
pub(super) fn proposal_outcome(
    job: &Job,
    edited: RepoProfile,
    verified: Verified,
    (now, confined): (u64, bool),
) -> Option<ProposalRecord> {
    let mut record = job.record.clone();
    let key = record.edit.as_ref()?.key.clone();
    record.updated_at = now;
    let failed = match &verified {
        Err(Stop::Cancelled) => return None,
        Err(Stop::Failed(reason)) => Some(RowEditState::Failed {
            reason: reason.clone(),
            tail: String::new(),
            secs: 0,
        }),
        Ok((_, verification, dropped)) => row_failure(&key, Some(verification), dropped),
    };
    match (verified, failed) {
        (Ok((profile, verification, dropped)), None) => {
            record.profile = Some(profile);
            record.verification = Some(verification);
            record.dropped = dropped;
            record.edit = None;
        }
        // Decision 18: a first `--anyway` applies whatever the check found.
        (_, Some(RowEditState::Failed { tail, secs, .. })) if job.anyway => {
            saved_anyway(&mut record, edited, &key, (tail, secs), (now, confined))
        }
        (_, state) => {
            if let (Some(edit), Some(state)) = (record.edit.as_mut(), state) {
                edit.state = state;
            }
        }
    }
    Some(record)
}

/// Fix round I1 (decision 15): why a held ✗ of the stored profile is not stored, and the
/// way out, in the screen and as a command to paste.
pub(super) fn held_failed(key: &str, value: Option<&str>) -> String {
    let value = match value {
        Some(value) => shell_word(value),
        None => "--unset".to_string(),
    };
    format!(
        "the edit of {key} failed its check; save it anyway (s, or anthrex profile edit \
         {key} {value} --anyway) or revert it (r, or anthrex profile reject)"
    )
}

/// `record`'s refusal of a store (`confirm`, `use_ready`): an edit of the stored profile
/// whose row edit is held ✗. A review proposal's ✗ keeps its old value, so it may be
/// used.
pub(super) fn refuse_held(record: &ProposalRecord) -> Option<String> {
    match &record.edit {
        Some(RowEdit {
            key,
            value,
            state: RowEditState::Failed { .. },
        }) if !is_review(record) => Some(held_failed(key, value.as_deref())),
        _ => None,
    }
}

/// `text` as one POSIX shell word: as is when it holds only safe characters, else in
/// single quotes.
fn shell_word(text: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_-./=:,@+%".contains(c);
    if !text.is_empty() && text.chars().all(safe) {
        return text.to_string();
    }
    format!("'{}'", text.replace('\'', "'\\''"))
}
