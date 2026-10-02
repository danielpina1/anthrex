//! Decision 27 (task M9.2.9): a red CI run on a stage PR's head becomes a fix task. One
//! `CiRecord` per (stage, head), a small state machine driven by the delivery pass:
//!
//! 1. **Logs**: `FailedLogs` for each failing check's GitHub Actions run (at most five),
//!    the host answering each log's last 48 KiB as text (the engine reads no file); a
//!    check with no Actions run contributes a line instead.
//! 2. **Summarising**: the `ci_summary` decider (decision 18); a red made only of
//!    cancellations and startup failures is `infra` without one.
//! 3. **Rerunning**: an `infra` red re-runs its failed jobs once (`CiRecord.reruns`,
//!    recorded in the step that issues each, so a restart never issues one twice); the
//!    next red on the same head is `unknown`.
//! 4. **Reproducing** at the PR head with 9.1's probe (`TestAt`, or a tier-2 job), holding
//!    `Run.full_op`; red with names on a tiered profile goes to 9.1's bisect
//!    (**Bisecting**), whose end comes back here (`fix.rs`).
//! 5. **Tasked** or **ToUser** (decision 27 step 6's cap, or a fix task the plan rules
//!    refuse). Pure (design decision 2).

use proto::{CiCategory, DeciderSource, PrState};

use super::super::requests::log;
use super::super::{Effect, OpKind, deciders};
use super::ci_repro::reproduce;
use super::fix;
use super::{emit, host_busy, stage_mut};
use crate::decider::ci::{category_label, safe_tests};
use crate::decider::{CiSummaryInput, DeciderRequest, Decision};
use crate::host::LogFile;
use crate::run::contract::sha7;
use crate::run::delivery::ops::HostOp;
use crate::run::delivery::snapshot::stage_count;
use crate::run::delivery::{CiPhase, CiRecord};
use crate::run::model::Run;

/// The text kept of the logs: what the decider reads (decision 18).
const TEXT_MAX: usize = crate::decider::CI_SUMMARY_INPUT_BYTES;
/// A summary line's bound, the schema's `maxLength` (the fallback's lines are the
/// log's own, so they are cut too).
const LINE_MAX: usize = 300;

/// The record of stage `n` being worked on: the newest that is not tasked or handed to
/// the user.
pub(super) fn active(run: &Run, n: u16) -> Option<usize> {
    let stage = run.delivery.stage(n)?;
    let done = |r: &CiRecord| matches!(r.phase, CiPhase::Tasked | CiPhase::ToUser);
    stage.ci.iter().rposition(|r| !done(r))
}

pub(super) fn record(run: &Run, n: u16, i: usize) -> Option<&CiRecord> {
    run.delivery.stage(n)?.ci.get(i)
}

pub(super) fn record_mut(run: &mut Run, n: u16, i: usize) -> Option<&mut CiRecord> {
    stage_mut(run, n).ci.get_mut(i)
}

/// Why the record is no longer worth working on: its PR is not open, the stage was
/// paused (decision 37: no fix task is added for it), or a newer head was pushed.
pub(super) fn stale(run: &Run, n: u16, rec: &CiRecord) -> Option<String> {
    let pr = run.delivery.pr(n)?;
    if pr.state != PrState::Open {
        let state = if pr.state == PrState::Merged {
            "merged"
        } else {
            "closed"
        };
        return Some(format!("its PR is {state}"));
    }
    if run.delivery.stage(n).is_some_and(|s| s.paused_by.is_some()) {
        return Some("the stage is paused".to_string());
    }
    (pr.pushed_head != rec.head).then(|| format!("{} was pushed since", sha7(&pr.pushed_head)))
}

/// [`stale`], or the run is ending (`finish` or a cancel): nothing more is added for
/// the red (fix round 1: no fix task, no attention line that nothing would clear).
pub(super) fn gone(run: &Run, n: u16, rec: &CiRecord) -> Option<String> {
    stale(run, n, rec)
        .or_else(|| super::super::full::ending(run).then(|| "the run is ending".to_string()))
}

/// Drops record `i` of stage `n` for `why`.
pub(super) fn drop_record(run: &mut Run, n: u16, i: usize, why: &str, now: u64) {
    let stage = stage_mut(run, n);
    if i >= stage.ci.len() {
        return;
    }
    let rec = stage.ci.remove(i);
    log(
        run,
        now,
        format!(
            "stage {n}: CI red at {} is not acted on: {why}",
            sha7(&rec.head)
        ),
    );
}

/// A bisect of stage `n` serves record `rec` (decision 27 step 4).
fn bisect_serves(run: &Run, n: u16, rec: &CiRecord) -> bool {
    let s = run.stage(n).and_then(|s| s.bisect.as_ref());
    s.is_some_and(|b| b.ci.as_ref() == Some(&(n, rec.key.clone())) && b.head == rec.head)
}

/// The delivery pass's CI part: per stage, the active record's next step.
pub(super) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for n in 1..=stage_count(run) {
        let Some(i) = active(run, n) else {
            continue;
        };
        if let Some(rec) = record(run, n, i).cloned() {
            step(run, n, i, rec, now, fx);
        }
    }
}

fn step(run: &mut Run, n: u16, i: usize, rec: CiRecord, now: u64, fx: &mut Vec<Effect>) {
    // A probe in flight is waited for (it holds `full_op`), and so is a bisect.
    if let Some(op) = rec.probe {
        if run.pending_ops.contains_key(&op) {
            return;
        }
        // Lost in a restart (reconciled `NotStarted`): issued again below.
        if run.full_op == Some(op) {
            run.full_op = None;
        }
        if let Some(r) = record_mut(run, n, i) {
            r.probe = None;
        }
    }
    let serving = rec.phase == CiPhase::Bisecting && bisect_serves(run, n, &rec);
    if !serving && let Some(why) = gone(run, n, &rec) {
        return drop_record(run, n, i, &why, now);
    }
    match rec.phase {
        CiPhase::Logs => logs_step(run, n, i, &rec, now, fx),
        CiPhase::Summarising if rec.decider.is_none() => summarise(run, n, i, now, fx),
        CiPhase::Rerunning => rerun_step(run, n, i, &rec, now, fx),
        CiPhase::Reproducing if now >= rec.retry_at => reproduce(run, n, i, now, fx),
        // 9.1's rebaseline ended the bisect without its end (the head moved).
        CiPhase::Bisecting if !serving => {
            let why = "the bisect ended without a result";
            fix::no_culprit_at(run, n, i, why, now, fx);
        }
        _ => {}
    }
}

/// Step 1: the next failed log, else the summary.
fn logs_step(run: &mut Run, n: u16, i: usize, rec: &CiRecord, now: u64, fx: &mut Vec<Effect>) {
    if rec.infra_only && rec.reruns.is_empty() {
        // Step 2: cancellations and startup failures only are infra, with no decider.
        log(
            run,
            now,
            format!(
                "stage {n}: CI red at {} was cancelled or did not start: infra",
                sha7(&rec.head)
            ),
        );
        if let Some(r) = record_mut(run, n, i) {
            r.category = Some(CiCategory::Infra);
        }
        return classified(run, n, i, now, fx);
    }
    let Some(ci_run) = (rec.ci_runs.iter()).find(|r| !rec.fetched.contains(r)) else {
        return summarise(run, n, i, now, fx);
    };
    let waiting = run.delivery.stage(n).and_then(|s| s.retry_at) > Some(now);
    if host_busy(run, n) || waiting {
        return;
    }
    let runs = rec.ci_runs.len().max(1) as u64;
    let max_bytes = (run.delivery.limits.ci_log_max_bytes / runs).max(1);
    let op = HostOp::FailedLogs {
        stage: n,
        ci_run: *ci_run,
        max_bytes,
    };
    emit(run, op, fx);
}

/// `FailedLogs` answered: the log's text joins the record's.
pub(super) fn logs(run: &mut Run, n: u16, ci_run: u64, file: LogFile) {
    let Some(i) = active(run, n) else {
        return;
    };
    let Some(r) = record_mut(run, n, i) else {
        return;
    };
    if r.phase != CiPhase::Logs || !r.ci_runs.contains(&ci_run) || r.fetched.contains(&ci_run) {
        return;
    }
    r.fetched.push(ci_run);
    r.log_failures = 0;
    if r.log.is_none() {
        r.log = Some(file.path);
    }
    if !r.text.is_empty() && !r.text.ends_with('\n') {
        r.text.push('\n');
    }
    r.text.push_str(&file.tail);
    let cut = r.text.len().saturating_sub(TEXT_MAX);
    let mut at = cut;
    while !r.text.is_char_boundary(at) {
        at += 1;
    }
    r.text.drain(..at);
}

/// `FailedLogs` of `ci_run` failed (fix round 1): after
/// [`FAILURES_BEFORE_ATTENTION`] in a row the run is given up and the summary is made
/// without its log, so the record never stalls in `Logs`.
///
/// [`FAILURES_BEFORE_ATTENTION`]: crate::run::delivery::FAILURES_BEFORE_ATTENTION
pub(super) fn logs_failed(run: &mut Run, n: u16, ci_run: u64, now: u64) {
    let max = crate::run::delivery::FAILURES_BEFORE_ATTENTION;
    let Some(r) = active(run, n).and_then(|i| record_mut(run, n, i)) else {
        return;
    };
    if r.phase != CiPhase::Logs || r.fetched.contains(&ci_run) {
        return;
    }
    r.log_failures = r.log_failures.saturating_add(1);
    if r.log_failures < max {
        return;
    }
    r.log_failures = 0;
    r.fetched.push(ci_run);
    let text = format!(
        "stage {n}: the failed log of CI run {ci_run} could not be fetched after {max} tries; summarising without it"
    );
    log(run, now, text);
}

/// The record's log as the decider and the fix task's quote read it: the failed logs,
/// then the lines of checks with no Actions run.
pub(super) fn log_text(rec: &CiRecord) -> String {
    let mut text = rec.text.clone();
    for line in &rec.external {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(line);
    }
    text
}

/// Step 2: the `ci_summary` decider, in a reader slot like every decider.
fn summarise(run: &mut Run, n: u16, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let (Some(rec), Some(pr)) = (record(run, n, i), run.delivery.pr(n)) else {
        return;
    };
    let input = CiSummaryInput {
        stage: n,
        pr: pr.number,
        checks: rec.checks.clone(),
        log_path: rec.log.clone().unwrap_or_default(),
        log: log_text(rec),
    };
    let id = deciders::queue(run, Vec::new(), DeciderRequest::CiSummary(input), now);
    if let Some(r) = record_mut(run, n, i) {
        r.phase = CiPhase::Summarising;
        r.decider = Some(id);
    }
    fx.extend(deciders::dispatch(run, now));
}

/// The `ci_summary` decider's answer (decision 18): the summary, the safe test names
/// (each unsafe one dropped, the count logged), the category.
pub(crate) fn summarised(
    run: &mut Run,
    decider_id: u64,
    decision: &Decision,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let crate::decider::DeciderAnswer::CiSummary {
        lines,
        failing_tests,
        category,
    } = &decision.answer
    else {
        return;
    };
    let found = (1..=stage_count(run)).find_map(|n| {
        let s = run.delivery.stage(n)?;
        let i =
            s.ci.iter()
                .position(|r| r.decider == Some(decider_id) && r.phase == CiPhase::Summarising)?;
        Some((n, i))
    });
    let Some((n, i)) = found else {
        return;
    };
    let Some(rec) = record(run, n, i).cloned() else {
        return;
    };
    if let Some(why) = gone(run, n, &rec) {
        return drop_record(run, n, i, &why, now);
    }
    let (names, dropped) = safe_tests(failing_tests);
    let source = match (decision.source, decision.fallback_reason.as_deref()) {
        (DeciderSource::Decider, _) => "decider".to_string(),
        (_, Some(reason)) => format!("fallback ({reason})"),
        (_, None) => "fallback".to_string(),
    };
    if let Some(r) = record_mut(run, n, i) {
        // Fix round 1 (decision 22): every kept line is one line, at most 300 chars.
        r.lines = (lines.iter())
            .map(|l| {
                proto::safe_text::one_line(l)
                    .chars()
                    .take(LINE_MAX)
                    .collect()
            })
            .collect();
        r.failing_tests = names;
        r.category = Some(*category);
        r.source = Some(source);
    }
    if dropped > 0 {
        let text = format!(
            "stage {n}: the CI summary's failing tests: {dropped} dropped (not test names)"
        );
        log(run, now, text);
    }
    classified(run, n, i, now, fx);
}

/// Decision 27 step 6's key: the sorted failing tests, or the category.
fn key_of(rec: &CiRecord) -> String {
    if rec.failing_tests.is_empty() {
        let category = rec.category.unwrap_or(CiCategory::Unknown);
        return category_label(category).to_string();
    }
    let mut names = rec.failing_tests.clone();
    names.sort();
    names.join(", ")
}

/// The ci fix tasks of stage `n` with key `key`.
fn fixes_of(run: &Run, n: u16, key: &str) -> usize {
    (run.tasks.iter())
        .filter(|t| matches!(&t.fixes, Some(crate::run::model::FixOf::Ci { stage, key: k, .. }) if *stage == n && k == key))
        .count()
}

/// The summary is in: the cap (step 6), else an infra re-run (step 3), else the
/// reproduction (step 4).
fn classified(run: &mut Run, n: u16, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let Some(rec) = record(run, n, i) else {
        return;
    };
    let mut category = rec.category.unwrap_or(CiCategory::Unknown);
    let rerun_due = rec.ci_runs.iter().any(|r| !rec.reruns.contains(r));
    if category == CiCategory::Infra && !rerun_due {
        // Step 3: a red after the re-run (or with nothing to re-run) is unknown.
        category = CiCategory::Unknown;
    }
    let Some(r) = record_mut(run, n, i) else {
        return;
    };
    r.category = Some(category);
    r.key = key_of(r);
    let key = r.key.clone();
    let made = fixes_of(run, n, &key);
    let max = run.delivery.limits.ci_fix_max as usize;
    if made >= max {
        return fix::capped(run, n, i, made, now);
    }
    let phase = if category == CiCategory::Infra {
        CiPhase::Rerunning
    } else {
        CiPhase::Reproducing
    };
    if let Some(r) = record_mut(run, n, i) {
        r.phase = phase;
    }
    match phase {
        CiPhase::Rerunning => {
            let Some(rec) = record(run, n, i).cloned() else {
                return;
            };
            let head = sha7(&rec.head);
            let text =
                format!("stage {n}: CI red at {head} is infra; re-running its failed jobs once");
            log(run, now, text);
            rerun_step(run, n, i, &rec, now, fx);
        }
        _ => reproduce(run, n, i, now, fx),
    }
}

/// Step 3: one `RerunFailed` per Actions run, each recorded in `reruns` in the step
/// that issues it. A re-run whose answer a restart lost is never issued again: the red
/// is then `unknown` (the controller's ruling).
fn rerun_step(run: &mut Run, n: u16, i: usize, rec: &CiRecord, now: u64, fx: &mut Vec<Effect>) {
    let in_flight = run.pending_ops.values().any(|p| {
        matches!(&p.kind, OpKind::Host { op: HostOp::RerunFailed { stage, .. }, .. } if *stage == n)
    });
    if let Some(ci_run) = rec.ci_runs.iter().find(|r| !rec.reruns.contains(r)) {
        let waiting = run.delivery.stage(n).and_then(|s| s.retry_at) > Some(now);
        if host_busy(run, n) || waiting {
            return;
        }
        let ci_run = *ci_run;
        if let Some(r) = record_mut(run, n, i) {
            r.reruns.push(ci_run);
        }
        emit(run, HostOp::RerunFailed { stage: n, ci_run }, fx);
        return;
    }
    let lost: Vec<&u64> = (rec.reruns.iter())
        .filter(|r| !rec.reruns_answered.contains(r))
        .collect();
    if !lost.is_empty() && !in_flight {
        let why = if lost.iter().all(|r| rec.rerun_timeouts.contains(r)) {
            "a re-run timed out twice"
        } else {
            "a re-run's answer was lost in a restart"
        };
        let text = format!(
            "stage {n}: {why}; CI red at {} is unknown, not re-run again",
            sha7(&rec.head)
        );
        log(run, now, text);
        classified(run, n, i, now, fx);
    }
}

/// `RerunFailed` answered.
pub(super) fn rerun_done(run: &mut Run, n: u16, ci_run: u64, now: u64) {
    let Some(r) = active(run, n).and_then(|i| record_mut(run, n, i)) else {
        return;
    };
    if !r.reruns_answered.contains(&ci_run) {
        r.reruns_answered.push(ci_run);
    }
    let text = format!("stage {n}: re-ran the failed jobs of CI run {ci_run}");
    log(run, now, text);
}

/// `RerunFailed` failed: issued again, except that a re-run that timed out (GitHub may
/// have started it) is issued again once; after a second timeout it stays issued and
/// unanswered, and `rerun_step` handles the red as `unknown` (fix round 1).
pub(super) fn rerun_failed(run: &mut Run, n: u16, ci_run: u64, timed_out: bool) {
    let Some(r) = active(run, n).and_then(|i| record_mut(run, n, i)) else {
        return;
    };
    if timed_out && r.rerun_timeouts.contains(&ci_run) {
        return;
    }
    if timed_out {
        r.rerun_timeouts.push(ci_run);
    }
    r.reruns.retain(|x| *x != ci_run);
}
