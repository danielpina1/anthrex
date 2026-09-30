//! Milestone 9.1 decisions 40–42: test-weakening signals in the engine. A claim's
//! `VerifyDone` asks for them only for a tiered profile with `test_paths` or
//! `skip_markers` (decision 6); a deleted test file its `owns` does not name exactly
//! bounces at the done gate (decision 41); every accepted claim's signals are kept on
//! the task, numbered `W1…` in the reviewer prompt, and a review must answer each
//! (decision 42). Pure (design decision 2).

use proto::{Finding, Severity, Verdict};

use crate::run::contract::{
    deleted_test_file_message, shown, signal_unjustified, signals_block, signals_unanswered,
};
use crate::run::globs::names_literally;
use crate::run::model::{Run, Task};
use crate::run::tiers::{ClaimSignals, Signal, SignalsSpec};

/// What task `i`'s `VerifyDone` reads: `None` for an untiered profile or one with
/// neither `test_paths` nor `skip_markers` (decisions 6 and 40).
pub(super) fn spec(run: &Run) -> Option<SignalsSpec> {
    let tiers = &run.profile.tiers;
    let wanted =
        tiers.is_tiered() && !(tiers.test_paths.is_empty() && tiers.skip_markers.is_empty());
    wanted.then(|| SignalsSpec {
        test_paths: tiers.test_paths.clone(),
        skip_markers: tiers.skip_markers.clone(),
    })
}

/// Decision 41: the rung-1 bounce text when a deleted test file is not named exactly
/// by task `i`'s `owns` (M8a's `names_literally`, the protected-file rule); `None`
/// when there is none. The caller bounces through `ladder::gate_failure(.., Done, ..)`.
pub(super) fn bounce(run: &Run, i: usize, signals: &ClaimSignals) -> Option<String> {
    let task = &run.tasks[i];
    let caught: Vec<String> = signals
        .list
        .iter()
        .filter_map(|s| match s {
            Signal::DeletedTestFile { path } if !names_literally(&task.spec.owns, path) => {
                Some(path.clone())
            }
            _ => None,
        })
        .collect();
    if caught.is_empty() {
        return None;
    }
    // Ruling C-20: restore from the commit the diff was read from.
    let base = match signals.base.is_empty() {
        true => run.head_for(task),
        false => signals.base.as_str(),
    };
    Some(deleted_test_file_message(&caught, base))
}

/// Decision 42: an accepted claim's signals replace the task's.
pub(super) fn keep(task: &mut Task, signals: ClaimSignals) {
    task.signals = signals.list;
    task.signals_more = signals.more;
    task.signal_refusals = 0;
}

/// `W<n>`, 1-based.
fn id(n: usize) -> String {
    format!("W{}", n + 1)
}

/// A signal's path and line (none for a deleted file; no path for `DiffTooLarge`).
fn place(signal: &Signal) -> (&str, Option<u32>) {
    match signal {
        Signal::DeletedTestFile { path } | Signal::TestCodeRemoved { path, .. } => (path, None),
        Signal::DiffTooLarge => ("", None),
        Signal::SkipMarker { path, line, .. } | Signal::AssertionLoss { path, line, .. } => {
            (path, Some(*line))
        }
    }
}

/// One `- W<n> …` line of the reviewer block (Interfaces "Prompts (exact)"). The path
/// and marker are shown through `contract::shown`: they come from the worker's diff.
fn line(n: usize, signal: &Signal) -> String {
    let id = id(n);
    match signal {
        Signal::DeletedTestFile { path } => format!("- {id} {}: deleted test file", shown(path)),
        Signal::SkipMarker { path, line, marker } => format!(
            "- {id} {}:{line}: added skip marker {}",
            shown(path),
            shown(marker)
        ),
        Signal::AssertionLoss {
            path,
            line,
            removed,
            added,
        } => format!(
            "- {id} {}:{line}: {removed} assertion lines removed, {added} added",
            shown(path)
        ),
        // Ruling C-20 (invented texts).
        Signal::TestCodeRemoved {
            path,
            asserts_removed,
        } => format!(
            "- {id} {}: #[cfg(test)] code removed, with {asserts_removed} assertion lines",
            shown(path)
        ),
        Signal::DiffTooLarge => format!(
            "- {id} the diff is too large to read for test changes; check the test files yourself"
        ),
    }
}

/// Decision 42's `Test changes to justify:` block of task `task`'s reviewer prompt;
/// empty when the task has no signal.
pub(crate) fn reviewer_block(task: &Task) -> String {
    let lines: Vec<String> = task
        .signals
        .iter()
        .enumerate()
        .map(|(n, s)| line(n, s))
        .collect();
    signals_block(&lines, task.signals_more)
}

/// Whether `text` starts with `id` as a whole id (`W1` is not answered by `W12 …`).
fn answers(text: &str, id: &str) -> bool {
    text.trim_start()
        .strip_prefix(id)
        .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// Decision 42's check of a `submit_review`: every `W<n>` of the task must start a
/// finding's text. The first submission that leaves one out is refused (`Err`, the
/// exact text); a second is accepted, and one `important` finding is added per missing
/// id, which turns the verdict to `changes`.
pub(super) fn check_review(
    task: &mut Task,
    verdict: &mut Verdict,
    findings: &mut Vec<Finding>,
) -> Result<(), String> {
    let missing: Vec<usize> = (0..task.signals.len())
        .filter(|&n| !findings.iter().any(|f| answers(&f.text, &id(n))))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    if task.signal_refusals == 0 {
        task.signal_refusals = 1;
        let ids: Vec<String> = missing.iter().map(|&n| id(n)).collect();
        return Err(signals_unanswered(&ids));
    }
    for n in missing {
        let (path, line) = place(&task.signals[n]);
        findings.push(Finding {
            severity: Severity::Important,
            file: (!path.is_empty()).then(|| shown(path)),
            line,
            input: None,
            text: signal_unjustified(&id(n), path, line),
        });
    }
    *verdict = Verdict::Changes;
    Ok(())
}

/// A new reviewer round must be refused afresh.
pub(super) fn new_review(task: &mut Task) {
    task.signal_refusals = 0;
}
