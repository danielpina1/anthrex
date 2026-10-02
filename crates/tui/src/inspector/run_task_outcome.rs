//! Milestone 9.0.7 decisions 12–14: a task's OUTCOME (the pipeline, the check, the
//! acceptance criteria, a block's text, a finished task's result) and its EVIDENCE (the
//! diff, the review, the worker's summary, the merge). Pure: it reads `App` and returns
//! sections; the panel sanitises, wraps and colours them (`Marks`).

use super::run_format::{clean, counts_text, location, most_severe, reason_text};
use super::run_task_sections::{check_line, first_line};
use super::{Marks, Section, SectionField};
use crate::app::App;
use crate::app::task_detail::DetailState;
use crate::safe_text::{multi_line, one_line};
use crate::theme::{self, Glyph};
use proto::{ReviewInfo, RunInfo, SummarySource, TaskInfo, TaskKind, TaskState, TestMode, Verdict};

/// One step of the pipeline (decision 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Passed,
    Failed,
    /// The step the task is at: its word alone, in `Working` bold.
    Current,
    NotReached,
}

fn judged(ok: Option<bool>) -> Step {
    match ok {
        Some(true) => Step::Passed,
        Some(false) => Step::Failed,
        None => Step::NotReached,
    }
}

/// The last review with a verdict.
fn last_verdict(task: &TaskInfo) -> Option<&ReviewInfo> {
    task.reviews
        .iter()
        .rev()
        .find(|review| review.verdict.is_some())
}

/// The steps that apply to `task`, each with its mark: proof only in `tdd` mode, check
/// unless the run is unverified, review only with a review route, merge unless the
/// task reports instead.
fn steps(run: &RunInfo, task: &TaskInfo) -> Vec<(&'static str, Step)> {
    let at = |state: TaskState, otherwise: Step| {
        if task.state == state {
            Step::Current
        } else {
            otherwise
        }
    };
    let mut steps = vec![(
        "done",
        at(TaskState::Working, judged(task.done_signal.map(|_| true))),
    )];
    if task.test_mode == TestMode::Tdd {
        let proof = judged(task.last_proof.as_ref().map(|proof| proof.ok));
        steps.push(("proof", at(TaskState::Proof, proof)));
    }
    if !run.unverified {
        let check = judged(task.last_check.as_ref().map(|check| check.ok));
        steps.push(("check", at(TaskState::Check, check)));
    }
    if task.review_route.is_some() {
        let review = judged(last_verdict(task).map(|review| !review.blocking));
        steps.push(("review", at(TaskState::Review, review)));
    }
    // A research or review task is reported, never merged: no merge step.
    let reports = matches!(task.kind, TaskKind::Research | TaskKind::Review);
    if !reports && task.state != TaskState::Reported {
        let merge = match task.state {
            TaskState::Merged => Step::Passed,
            TaskState::MergeQueue => Step::Current,
            _ => Step::NotReached,
        };
        steps.push(("merge", merge));
    }
    steps
}

/// Decision 13: `done ✓ › proof ✓ › check ✓ › review › merge ◌`, the current step's
/// word alone; folded to ASCII when `ascii`.
pub(crate) fn pipeline_text(run: &RunInfo, task: &TaskInfo, ascii: bool) -> String {
    let mark = |step| match step {
        Step::Passed => Some(Glyph::Passed),
        Step::Failed => Some(Glyph::Failed),
        Step::NotReached => Some(Glyph::NotStarted),
        Step::Current => None,
    };
    let separator = format!(" {} ", theme::glyph(Glyph::Separator, ascii));
    steps(run, task)
        .into_iter()
        .map(|(word, step)| match mark(step) {
            Some(g) => format!("{word} {}", theme::glyph(g, ascii)),
            None => word.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(&separator)
}

/// Decision 14: every criterion `✓` when `review`, the task's latest, approves (an
/// `Approve` verdict that does not block) and the task was not merged without approval;
/// otherwise every one `◌`. `✗` is never drawn: no message carries a per-criterion
/// verdict. Each criterion is one line, so a line break in it cannot forge a row.
pub(crate) fn criteria_rows<S: AsRef<str>>(
    criteria: &[S],
    review: Option<&ReviewInfo>,
    merged_without_approval: Option<&str>,
) -> Vec<String> {
    let approved = review
        .is_some_and(|review| review.verdict == Some(Verdict::Approve) && !review.blocking)
        && merged_without_approval.is_none();
    let mark = theme::glyph(
        if approved {
            Glyph::Passed
        } else {
            Glyph::NotStarted
        },
        false,
    );
    criteria
        .iter()
        .map(|criterion| format!("{mark} {}", one_line(criterion.as_ref())))
        .collect()
}

/// Decision 12's review row: `r<n> ✓ approve · <line>`, `r<n> ✗ changes · <counts>:
/// <worst finding>` and the summary's first line on the next row, `in review · r<n>`
/// while a round runs, `not yet` before the first verdict, `none` without a review
/// route.
pub(crate) fn review_row(task: &TaskInfo) -> String {
    if task.review_route.is_none() {
        return "none".to_owned();
    }
    if task.state == TaskState::Review {
        return format!("in review · r{}", task.reviews.len());
    }
    let Some(review) = last_verdict(task) else {
        return "not yet".to_owned();
    };
    // The mark is whether the verdict blocks, as the pipeline's review step reads it;
    // the word is the verdict's own.
    let mark = if review.blocking { "✗" } else { "✓" };
    let summary = first_line(&review.summary).map(one_line);
    let (word, detail) = if review.verdict == Some(Verdict::Approve) {
        ("approve", summary)
    } else {
        let worst = most_severe(&review.findings).map(|worst| {
            let text = format!("\"{}\"", clean(&worst.text));
            let place = location(worst).map_or(text.clone(), |place| format!("{place} {text}"));
            let head = format!("{}: {place}", counts_text(&review.findings));
            // The summary line the 9.0.5 panel's `verdict` showed, on the next row.
            match &summary {
                Some(line) => format!("{head}\n{line}"),
                None => head,
            }
        });
        ("changes", worst.or(summary))
    };
    match detail {
        Some(detail) => format!("r{} {mark} {word} · {detail}", review.round),
        None => format!("r{} {mark} {word}", review.round),
    }
}

/// `+<a> −<r> · <f> files`, then the test and its red sha (with the proof's mark) when
/// there are.
pub(super) fn diff_text(task: &TaskInfo) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(diff) = &task.diff {
        let files = if diff.files == 1 { "file" } else { "files" };
        parts.push(format!(
            "+{} −{} · {} {files}",
            diff.added, diff.removed, diff.files
        ));
    }
    if let (Some(test), Some(red)) = (&task.test, &task.red) {
        let red7: String = red.chars().take(7).collect();
        let mut part = format!("test `{}` red {}", clean(test), clean(&red7));
        if let Some(proof) = &task.last_proof {
            part.push_str(if proof.ok { " ✓" } else { " ✗" });
        }
        parts.push(part);
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn row(label: &'static str, value: impl Into<String>, marks: Marks) -> SectionField {
    SectionField {
        label,
        note: None,
        value: value.into(),
        collapse: false,
        marks,
    }
}

/// EVIDENCE: `diff`, `review`, `summary` with its source note, `merged`.
pub(super) fn evidence(run: &RunInfo, task: &TaskInfo, app: &App) -> Section {
    let mut fields = Vec::new();
    if let Some(diff) = diff_text(task) {
        fields.push(row("diff", diff, Marks::None));
    }
    // Only the row's first line is the client's: the summary under it is agent text.
    fields.push(row("review", review_row(task), Marks::First));
    if let Some(DetailState::Ready(detail)) = app.task_detail_for(&run.run_id, &task.id)
        && let Some(summary) = &detail.worker_summary
    {
        let note = match detail.summary_source {
            Some(SummarySource::TaskDone) => Some("task_done"),
            Some(SummarySource::LastMessage) => Some("last message"),
            None => None,
        };
        fields.push(SectionField {
            note,
            ..row("summary", summary.clone(), Marks::None)
        });
    }
    if let Some(commit) = &task.merge_commit {
        let mut text: String = one_line(commit).chars().take(7).collect();
        if let Some(reason) = &task.merged_without_approval {
            text.push_str(&format!(" · without approval: {}", one_line(reason)));
        }
        fields.push(row("merged", text, Marks::None));
    }
    Section {
        title: "EVIDENCE",
        fields,
    }
}

/// OUTCOME: `pipeline`, `check`, `accept`, `blocked` (a blocked task's reason word and
/// every line of its text), `result` (ruling D-2: a finished task's outcome line).
pub(super) fn outcome(run: &RunInfo, task: &TaskInfo, app: &App, evidence: &Section) -> Section {
    let mut fields = vec![
        row("pipeline", pipeline_text(run, task, false), Marks::Pipeline),
        row("check", check_line(run, task), Marks::Lead),
    ];
    // Ruling R-13: a `pr` run's task names its stage's PR and what it fixes.
    for (label, value) in super::run_stage_pr::task_rows(run, task) {
        fields.push(row(label, value, Marks::None));
    }
    let criteria = match app.task_detail_for(&run.run_id, &task.id) {
        Some(DetailState::Ready(detail)) if detail.acceptance.is_empty() => None,
        Some(DetailState::Ready(detail)) => Some(
            criteria_rows(
                &detail.acceptance,
                task.reviews.last(),
                task.merged_without_approval.as_deref(),
            )
            .join("\n"),
        ),
        Some(DetailState::Failed(_)) => None,
        Some(DetailState::InFlight(_)) | None => Some("loading…".to_owned()),
    };
    if let Some(criteria) = criteria {
        fields.push(row("accept", criteria, Marks::Lead));
    }
    if task.state == TaskState::Blocked
        && let Some(block) = &task.block
    {
        let word = reason_text(block.reason);
        let text = if block.text.trim().is_empty() {
            word.to_owned()
        } else {
            format!("{word}: {}", block.text)
        };
        fields.push(row("blocked", text, Marks::None));
    }
    if let Some(line) = result_line(task, evidence) {
        fields.push(row("result", line, Marks::None));
    }
    Section {
        title: "OUTCOME",
        fields,
    }
}

/// Ruling D-2: a merged or reported task's outcome, so it shows without scrolling. The
/// first line of the worker's summary, else of the review's verdict, the diff or the
/// merge (the detail may not have landed); `None` while the task is unfinished.
fn result_line(task: &TaskInfo, evidence: &Section) -> Option<String> {
    if !matches!(task.state, TaskState::Merged | TaskState::Reported) {
        return None;
    }
    let value = |label: &str| {
        evidence
            .fields
            .iter()
            .find(|field| field.label == label)
            .map(|field| field.value.as_str())
    };
    let verdict = last_verdict(task).and(value("review"));
    let first = value("summary")
        .or(verdict)
        .or(value("diff"))
        .or(value("merged"))?;
    first_line(&multi_line(first)).map(one_line)
}
