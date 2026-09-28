//! Milestone 9 decision 24 (task M9.4): the field rules of research and review tasks,
//! for every source, split out of `validate.rs`. M8a decision 6's refusal of the two
//! kinds is gone. Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (M8a design decision 2).

use proto::{PlanTask, TaskKind};

use super::plan::PlanError;

/// A research or review task runs with test mode `none`; a given reason is kept.
pub const READER_TEST_MODE_NOTE: &str =
    "test mode none: research and review tasks change nothing (rule 8)";

/// Research and review tasks read and report; they change no file.
pub(super) fn is_reader(kind: TaskKind) -> bool {
    matches!(kind, TaskKind::Research | TaskKind::Review)
}

/// A research or review task owns nothing; a review task, and only a review task, has
/// a well-formed `review_target`.
pub(super) fn check_reader_fields(spec: &PlanTask, errors: &mut Vec<PlanError>) {
    let e = |field: &str, message: String| PlanError::new(Some(&spec.id), field, "5.2", message);
    if is_reader(spec.kind) && !spec.owns.is_empty() {
        errors.push(e(
            "owns",
            "research and review tasks change nothing; leave owns empty (rule 5.2)".into(),
        ));
    }
    match (&spec.review_target, spec.kind) {
        (None, TaskKind::Review) => errors.push(e(
            "review_target",
            "required for a review task (rule 5.2)".into(),
        )),
        (Some(_), kind) if kind != TaskKind::Review => errors.push(e(
            "review_target",
            "only review tasks have a review target".into(),
        )),
        (Some(t), _) if !is_review_target(t) => errors.push(e(
            "review_target",
            format!("{t} is not a revision or a range <a>..<b>"),
        )),
        _ => {}
    }
}

/// One revision, or `<a>..<b>`: each part is 1–200 of `[A-Za-z0-9._/@^~-]` and does not
/// start with `-`. Whether it resolves is checked at dispatch (decision 36).
fn is_review_target(target: &str) -> bool {
    let part = |p: &str| {
        (1..=200).contains(&p.len())
            && !p.starts_with('-')
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._/@^~-".contains(&b))
    };
    match target.split_once("..") {
        Some((a, b)) => part(a) && part(b) && !b.contains(".."),
        None => part(target),
    }
}
