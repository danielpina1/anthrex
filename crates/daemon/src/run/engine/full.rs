//! Milestone 9.1's tier 3 (task M9.1.14; decisions 17–19): the full `check`, run on a
//! stage head in the run-level scratch checkout `Run::full_path()`. It starts when the
//! merge queue has been idle for `full_idle_secs` (on the lowest stage without a green
//! one, at `FullIdle`), per stage bottom-up at completion (at `FullStage`), and from
//! 9.2 before a stage PR opens ([`request`]). At most one job per run (`Run.full_op`).
//! A green job marks its commit green (`StageFull.green_at`); a red one marks it red
//! (`red_at`), which holds completion until the stage head moves or the `finish` edit.
//! An untiered profile never reaches any of it: its completion is M8a's final check,
//! and nothing here touches its run (decision 6). Pure (design decision 2).

use proto::RunState;

use super::requests::log;
use super::{Effect, OpId, OpKind, OpResult, ScratchAt, emit_op, next_op, tiers, wake};
use crate::run::contract::sha7;
use crate::run::model::{InfraFailures, Run, StageRecord};
use crate::run::slots::Priority;
use crate::run::tiers::TierSpec;

/// Why a tier-3 job starts (decision 17): (b) the queue is idle, (c) completion, (a)
/// before a stage PR opens (9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum FullWhy {
    Idle,
    Completion,
    Deliver,
}

/// Decision 38's attention lines name at most this many tests, its wake notes fewer.
const ATTENTION_TESTS: usize = 5;
const WAKE_TESTS: usize = 3;

/// Until task M9.1.15 brings the bisect (decision 35), a red that could be bisected
/// ends as decision 38's "no culprit" with this reason. M9.1.15 replaces it with
/// `bisect::start`.
const NO_BISECT_YET: &str = "bisecting arrives with task M9.1.15";

/// Ruling C-18: after the executor's own failure number `k` on one commit, the next
/// pass may start tier 3 again after `INFRA_BACKOFF[k - 1]` seconds; at [`INFRA_MAX`]
/// failures the stage is held until `run resume`, so the last step is not reached.
const INFRA_BACKOFF: [u64; 3] = [30, 120, 600];
const INFRA_MAX: u8 = 3;
/// A "could not run" line keeps at most this many characters of its message.
const LINE_MAX: usize = 200;

/// Tier 3 runs at all: a tiered profile with a `check` (decision 20: without one the
/// run is unverified and tier 3 runs nothing).
fn active(run: &Run) -> bool {
    tiers::tiered(run) && run.profile.check.is_some()
}

/// Stage `s`'s head still needs a green tier 3: it is not the base (nothing merged,
/// as M8a's final check skips it) and has none.
fn lacks_green(run: &Run, s: &StageRecord) -> bool {
    s.head != run.base_sha && s.full.green_at.as_deref() != Some(s.head.as_str())
}

/// Stage `s`'s head is the commit its last tier 3 found red.
fn red_at_head(s: &StageRecord) -> bool {
    s.full.red_at.as_deref() == Some(s.head.as_str())
}

/// Stage `s`'s executor failures, when they are on its head (ruling C-18).
fn infra_at_head(s: &StageRecord) -> Option<&InfraFailures> {
    s.full.infra.as_ref().filter(|i| i.commit == s.head)
}

/// Stage `s`'s head failed [`INFRA_MAX`] times in a row: held until `run resume`.
fn infra_held(s: &StageRecord) -> bool {
    infra_at_head(s).is_some_and(|i| i.count >= INFRA_MAX)
}

/// Stage `s`'s head may not start tier 3 yet at `now`: held, or within its backoff.
fn infra_waiting(s: &StageRecord, now: u64) -> bool {
    infra_at_head(s).is_some_and(|i| {
        let k = usize::from(i.count.max(1) - 1).min(INFRA_BACKOFF.len() - 1);
        i.count >= INFRA_MAX || now < i.at.saturating_add(INFRA_BACKOFF[k])
    })
}

/// The first non-empty line of `text`, trimmed, at most [`LINE_MAX`] characters.
fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    line.unwrap_or_default().chars().take(LINE_MAX).collect()
}

/// A tier-3 job or a bisect is in flight: at most one per run (decision 17).
fn in_flight(run: &Run) -> bool {
    run.full_op.is_some() || run.stages.iter().any(|s| s.bisect.is_some())
}

/// A red stage no longer holds completion once the run is ending anyway: the `finish`
/// edit (decision 19), or `run cancel`, which must complete.
fn ending(run: &Run) -> bool {
    run.finish_edit || run.cancelled
}

/// Decision 17(b): no queued task and no candidate in flight.
fn queue_idle(run: &Run) -> bool {
    run.merge_queue.is_empty()
        && !run
            .pending_ops
            .values()
            .any(|p| matches!(p.kind, OpKind::MergeCandidate { .. }))
}

/// Decision 17(b), every scheduler pass of a running run: the queue-idle clock
/// (`Run.queue_idle_since`, set when the queue empties, cleared while an item is in
/// it, so a merge resets it), and after `full_idle_secs` a tier-3 job on the lowest
/// created stage whose head has no green one and is not already red.
pub(super) fn idle_pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if !active(run) {
        return;
    }
    if !queue_idle(run) {
        run.queue_idle_since = None;
        return;
    }
    let since = *run.queue_idle_since.get_or_insert(now);
    let due = since.saturating_add(run.limits.testing.full_idle_secs);
    if now < due || ending(run) || in_flight(run) {
        return;
    }
    let lowest = run
        .stages
        .iter()
        .filter(|s| lacks_green(run, s) && !red_at_head(s) && !infra_waiting(s, now))
        .map(|s| s.n)
        .min();
    if let Some(n) = lowest {
        start(run, n, FullWhy::Idle, now, fx);
    }
}

/// Decision 19, after the completion guard's `RefsOk` (tiered profiles only): `true`
/// when every created stage's head is green, or red and the run ending anyway (then
/// with M8a's `final_check_failed`); otherwise tier 3 starts on the lowest stage that
/// lacks one, or a red stage makes the run wait, and it is `false`.
pub(super) fn completion(run: &mut Run, now: u64, fx: &mut Vec<Effect>) -> bool {
    if !active(run) {
        return true;
    }
    if in_flight(run) {
        return false;
    }
    let mut stages: Vec<u16> = run.stages.iter().map(|s| s.n).collect();
    stages.sort_unstable();
    let mut red = Vec::new();
    for n in stages {
        let Some(s) = run.stage(n) else { continue };
        if !lacks_green(run, s) {
            continue;
        }
        // Ruling C-18: a held stage is passed over only by a run ending anyway.
        if infra_waiting(s, now) {
            if ending(run) && infra_held(s) {
                red.push(n);
                continue;
            }
            return false;
        }
        if !red_at_head(s) {
            start(run, n, FullWhy::Completion, now, fx);
            return false;
        }
        if !ending(run) {
            return false;
        }
        red.push(n);
    }
    if !red.is_empty() {
        run.final_check_failed = true;
        let stages: Vec<String> = red.iter().map(u16::to_string).collect();
        log(
            run,
            now,
            format!(
                "completing without a green tier 3 on stage {}",
                stages.join(", ")
            ),
        );
    }
    true
}

/// Decision 19: completion waits while a created stage's head is red, until the head
/// moves or the run ends anyway; ruling C-18: and while its head waits out an executor
/// failure's backoff, or is held after the last one (unless the run ends anyway).
/// Tiered profiles only.
pub(super) fn holds_completion(run: &Run, now: u64) -> bool {
    active(run)
        && run.stages.iter().any(|s| {
            lacks_green(run, s)
                && ((!ending(run) && red_at_head(s))
                    || (infra_waiting(s, now) && !(ending(run) && infra_held(s))))
        })
}

/// Whether a stage waits red or on an executor failure, whatever the time (the tests'
/// liveness check: such a run is not stuck).
#[cfg(test)]
pub(super) fn waits(run: &Run) -> bool {
    active(run)
        && run
            .stages
            .iter()
            .any(|s| lacks_green(run, s) && (red_at_head(s) || infra_at_head(s).is_some()))
}

/// `run resume` (ruling C-18): every stage's executor failures are forgotten, so the
/// next pass retries tier 3. `true` when a stage was held.
pub(super) fn retry(run: &mut Run, now: u64) -> bool {
    let mut held = Vec::new();
    for s in run.stages.iter_mut() {
        if infra_held(s) {
            held.push(s.n);
        }
        s.full.infra = None;
    }
    for n in &held {
        log(run, now, format!("stage {n}: tier 3 retries (run resume)"));
    }
    !held.is_empty()
}

/// Decision 17(a), 9.2's entry: tier 3 on stage `stage` at `FullStage` priority unless
/// the profile is untiered, the run is not running, the stage is not created or its
/// head is green already, or a tier-3 job or bisect is in flight. `true` when a job
/// started.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn request(
    run: &mut Run,
    stage: u16,
    why: FullWhy,
    now: u64,
    fx: &mut Vec<Effect>,
) -> bool {
    if !active(run) || run.state != RunState::Running || in_flight(run) {
        return false;
    }
    let Some(s) = run.stage(stage) else {
        return false;
    };
    if s.full.green_at.as_deref() == Some(s.head.as_str()) || infra_waiting(s, now) {
        return false;
    }
    start(run, stage, why, now, fx);
    true
}

/// Decision 18: `Tier { tier: 3 }` on stage `n`'s head, in `.full`, prepared at that
/// commit with the setup marker as `<task>.proof` is; decision 24's class by `why`.
fn start(run: &mut Run, n: u16, why: FullWhy, now: u64, fx: &mut Vec<Effect>) {
    let Some(head) = run.stage(n).map(|s| s.head.clone()) else {
        return;
    };
    let mut job = tiers::spec(run, 3, n, run.full_path());
    job.scratch = Some(ScratchAt {
        root: run.root.clone(),
        commit: head.clone(),
        setup: run.profile.setup.clone(),
    });
    job.head = head.clone();
    let (priority, because) = match why {
        FullWhy::Idle => (Priority::FullIdle, "the merge queue is idle"),
        FullWhy::Completion => (Priority::FullStage, "completion"),
        FullWhy::Deliver => (Priority::FullStage, "delivery"),
    };
    job.priority = priority;
    let op = next_op(run);
    run.full_op = Some(op);
    emit_op(run, op, None, OpKind::Tier(Box::new(job)), fx);
    log(
        run,
        now,
        format!("stage {n}: running tier 3 at {} ({because})", sha7(&head)),
    );
}

fn stage_mut(run: &mut Run, n: u16) -> Option<&mut StageRecord> {
    run.stages.iter_mut().find(|s| s.n == n)
}

fn first(tests: &[String], k: usize) -> String {
    let shown: Vec<&str> = tests.iter().take(k).map(String::as_str).collect();
    shown.join(", ")
}

/// A tier-3 job's result (decision 18). It applies to the commit the job ran on,
/// whatever the stage head is now (decision 17).
pub(super) fn full_done(
    run: &mut Run,
    op: OpId,
    spec: &TierSpec,
    result: OpResult,
    now: u64,
    _fx: &mut Vec<Effect>,
) {
    if run.full_op == Some(op) {
        run.full_op = None;
    }
    let (n, commit) = (spec.stage, spec.head.clone());
    if let OpResult::Tier(outcome) = &result {
        tiers::run_facts(run, outcome, now);
    }
    if run.stage(n).is_none() {
        return;
    }
    match result {
        OpResult::Tier(outcome) => {
            // Decision 21's lines, less the affected set (always the full suite).
            for line in tiers::lines(&outcome).into_iter().skip(1) {
                log(run, now, line);
            }
            let mut record = tiers::record(&outcome, now);
            // Ruling C-18: the commit the record is about.
            record.commit = commit.clone();
            let failing = record.failing.clone();
            if let Some(s) = stage_mut(run, n) {
                s.full.last = Some(record);
                s.full.infra = None;
            }
            if outcome.ok {
                green(run, n, &commit, outcome.secs, now);
            } else {
                red(run, n, &commit, &failing, now);
            }
        }
        OpResult::SetupFailed { output } => setup_failed(run, n, &commit, &output, now),
        OpResult::Failed { message } => infra_failed(run, n, &commit, &message, now),
        _ => {}
    }
}

fn green(run: &mut Run, n: u16, commit: &str, secs: u64, now: u64) {
    if let Some(s) = stage_mut(run, n) {
        s.full.green_at = Some(commit.to_string());
        if s.full.red_at.as_deref() == Some(commit) {
            s.full.red_at = None;
        }
        s.full.note = None;
    }
    log(
        run,
        now,
        format!("stage {n}: tier 3 green at {} ({secs}s)", sha7(commit)),
    );
}

/// Decision 35's checks, made here so 9.2 can bisect a CI failure without them: the
/// cap first, then a bisect's needs; each failing check is decision 38's end, with its
/// attention line and wake note, and the stage waits red.
fn red(run: &mut Run, n: u16, commit: &str, failing: &[String], now: u64) {
    log(
        run,
        now,
        format!("stage {n}: tier 3 red ({})", first(failing, WAKE_TESTS)),
    );
    let fixes = run.stage(n).map_or(0, |s| s.full.bisect_fixes);
    let (attention, wake_text) = if fixes >= run.limits.testing.bisect_fix_max {
        let text = format!(
            "stage {n}: tier 3 still red after {fixes} fix tasks: {}; fix it with a task, or end the run with the finish edit",
            first(failing, ATTENTION_TESTS)
        );
        (text.clone(), text)
    } else {
        let reason = if failing.is_empty() {
            "no failing test names to bisect with"
        } else if run.profile.single_test.is_none() {
            "the profile has no single_test to bisect with"
        } else {
            NO_BISECT_YET
        };
        (
            format!(
                "tier 3 red, no single culprit: {} (stage {n}: {reason})",
                first(failing, ATTENTION_TESTS)
            ),
            format!(
                "stage {n} tier 3 red, no single culprit: {}; plan a fix",
                first(failing, WAKE_TESTS)
            ),
        )
    };
    mark_red(run, n, commit, attention);
    wake_on_head(run, n, commit, wake_text);
}

/// Ruling C-18: a result on a commit that is no longer the stage head is recorded, but
/// wakes nobody: the next pass judges the new head.
fn wake_on_head(run: &mut Run, n: u16, commit: &str, text: String) {
    if run.stage_head(n) == Some(commit) {
        wake::note(run, text);
    }
}

/// The job's setup failed in `.full`: the commit is red with the reason, it leaves no
/// record of tests, and the run waits as for a red suite (invented, decision 19 names
/// no such case; M8a's final check completes anyway).
fn setup_failed(run: &mut Run, n: u16, commit: &str, output: &str, now: u64) {
    log(
        run,
        now,
        format!("stage {n}: could not run tier 3: setup failed:\n{output}"),
    );
    let text = format!(
        "stage {n}: could not run tier 3: setup failed: {}",
        first_line(output)
    );
    if let Some(s) = stage_mut(run, n) {
        s.full.last = None;
        s.full.infra = None;
    }
    mark_red(run, n, commit, text.clone());
    wake_on_head(run, n, commit, text);
}

/// Ruling C-18: the executor itself failed (not the suite, not its setup). Nothing is
/// red and nobody is woken: the next pass retries after a backoff, and after
/// [`INFRA_MAX`] failures in a row on one commit the stage is held for `run resume`.
fn infra_failed(run: &mut Run, n: u16, commit: &str, message: &str, now: u64) {
    log(
        run,
        now,
        format!("stage {n}: could not run tier 3: {message}"),
    );
    let line = first_line(message);
    let Some(s) = stage_mut(run, n) else { return };
    let count = match &s.full.infra {
        Some(i) if i.commit == commit => i.count.saturating_add(1),
        _ => 1,
    };
    s.full.infra = Some(InfraFailures {
        commit: commit.to_string(),
        count,
        at: now,
        line,
    });
    if count >= INFRA_MAX {
        let text = format!("stage {n}: tier 3 held after {count} failures; run resume retries");
        log(run, now, text);
    }
}

fn mark_red(run: &mut Run, n: u16, commit: &str, note: String) {
    if let Some(s) = stage_mut(run, n) {
        s.full.red_at = Some(commit.to_string());
        s.full.note = Some(note);
    }
}

/// The run's tier-3 attention lines: while it runs, each stage red on its head with its
/// note (decision 38's line); once it completed red (`final_check_failed`), decision
/// 19's `tier 3 red on stage <n>: <tests>` in place of M8a's final-check line.
pub(crate) fn attention(run: &Run) -> Vec<String> {
    let mut lines = red_lines(run);
    if run.final_check_failed || !run.state.is_terminal() {
        // Ruling C-18: a stage held after the executor's failures.
        for s in run
            .stages
            .iter()
            .filter(|s| lacks_green(run, s) && infra_held(s))
        {
            if let Some(i) = infra_at_head(s) {
                lines.push(format!(
                    "stage {}: could not run tier 3 ({}); anthrex run resume retries",
                    s.n, i.line
                ));
            }
        }
    }
    lines
}

fn red_lines(run: &Run) -> Vec<String> {
    let red = run
        .stages
        .iter()
        .filter(|s| red_at_head(s) && lacks_green(run, s));
    if run.final_check_failed {
        return red
            .map(|s| {
                // Ruling C-18: only a record of the red commit itself names tests.
                let failing = s
                    .full
                    .last
                    .as_ref()
                    .filter(|t| !t.ok && s.full.red_at.as_deref() == Some(t.commit.as_str()))
                    .map(|t| t.failing.clone())
                    .unwrap_or_default();
                match (&s.full.note, failing.is_empty()) {
                    (Some(note), true) => note.clone(),
                    _ => format!(
                        "tier 3 red on stage {}: {}",
                        s.n,
                        first(&failing, ATTENTION_TESTS)
                    ),
                }
            })
            .collect();
    }
    if run.state.is_terminal() {
        return Vec::new();
    }
    red.filter_map(|s| s.full.note.clone()).collect()
}
