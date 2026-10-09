//! Milestone 9.9 decision 18 (OFA §4.4): problems only the user can fix. One classifier
//! over host and tool text ([`marked`]) and one predicate for a halt ([`halt`]), shared by
//! the snapshot and the orchestrator's refusals (ruling R3). Pure.

use proto::RunState;

use super::actions::rules;
use crate::run::model::Run;

/// Text that names a cause the orchestrator cannot fix, matched case-insensitively.
pub(crate) const USER_ONLY_MARKERS: &[&str] = &[
    "no space left on device",
    "disk quota exceeded",
    "please tell me who you are",
    "author identity unknown",
    "committer identity unknown",
    "empty ident name",
    "gh auth login",
    "not logged in",
    "authentication failed",
    "could not read username",
    "permission denied (publickey)",
    "command not found",
];

/// Whether `text` names a problem only the user can fix.
pub(crate) fn marked(text: &str) -> bool {
    let lower = text.to_lowercase();
    USER_ONLY_MARKERS.iter().any(|m| lower.contains(m))
}

/// Whether the run is halted on something only the user can resume: `rules::resume`
/// refuses it without `--rebaseline` (a ref moved under the run), or its reason is marked.
pub(crate) fn halt(run: &Run) -> bool {
    run.state == RunState::Halted
        && (rules::resume(run, false).is_some() || run.halted_reason.as_deref().is_some_and(marked))
}

/// The orchestrator's refusal (decision 10) for `target` waiting on `text`.
pub(crate) fn refusal(target: &str, text: &str) -> String {
    let first = text.lines().next().unwrap_or_default();
    format!(
        "{target} waits on something only the user can fix, and they have been alerted: {first}"
    )
}
