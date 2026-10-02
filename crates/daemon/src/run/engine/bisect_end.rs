//! A bisect's end (decisions 38 and 57, ruling C-27 (3)): its history line, its
//! attention line, and the end a rebaseline gives it. A child of `bisect`.

use super::super::history::BisectResult;
use super::super::requests::log;
use super::super::{Effect, full};
use super::stage_mut;
use crate::run::model::{BisectRecord, Run};

/// Ruling C-27 (3): a rebaseline moved stage `n`'s head, so its bisect (if any) ends
/// `rebaselined`: its history line is written as any ended bisect's is, its pending
/// probe is dropped (a late result finds no bisect and no pending op, and is ignored),
/// and nothing is marked red and nobody is woken: tier 3 is due for the new head.
pub(in crate::run::engine) fn rebaselined(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    let Some(b) = stage_mut(run, n).and_then(|s| s.bisect.take()) else {
        return;
    };
    if let Some((op, _)) = b.probe {
        run.pending_ops.remove(&op);
        if run.full_op == Some(op) {
            run.full_op = None;
        }
    }
    record(run, n, &b, BisectResult::None(REBASELINED), now, fx);
    log(run, now, format!("stage {n}: bisect ended: {REBASELINED}"));
}

/// Ruling C-27 (3): the reason of a bisect a rebaseline ended.
pub(crate) const REBASELINED: &str = "rebaselined";

/// Decision 38: the bisect of stage `n` ends without a culprit, for `reason`.
pub(super) fn end(run: &mut Run, n: u16, reason: String, now: u64, fx: &mut Vec<Effect>) {
    let Some(b) = stage_mut(run, n).and_then(|s| s.bisect.take()) else {
        return;
    };
    if run
        .full_op
        .is_some_and(|op| b.probe.is_some_and(|(p, _)| p == op))
    {
        run.full_op = None;
    }
    record(run, n, &b, BisectResult::None(&reason), now, fx);
    end_with(run, n, &b, reason, now, fx);
}

/// Decision 38's end without a culprit; for a CI red (milestone 9.2 decision 27), the
/// delivery's stage fix instead of 9.1's attention line and wake note.
pub(super) fn end_with(
    run: &mut Run,
    n: u16,
    b: &BisectRecord,
    reason: String,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    log(
        run,
        now,
        format!("stage {n}: bisect ended without a culprit: {reason}"),
    );
    if b.ci.is_some() {
        return super::super::delivery::ci_no_culprit(run, n, b, &reason, now, fx);
    }
    full::no_culprit(run, n, &b.head, &b.tests, &reason);
}

/// Decision 57: the ended bisect's `bisect` history line, numbered in its stage.
pub(super) fn record(
    run: &mut Run,
    n: u16,
    b: &BisectRecord,
    result: BisectResult<'_>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(s) = stage_mut(run, n) else {
        return;
    };
    s.full.bisects = s.full.bisects.saturating_add(1);
    let seq = s.full.bisects;
    let (culprit, fix_task, reason) = result.parts();
    s.full.ended.push(crate::run::model::BisectEnd {
        head: b.head.clone(),
        range: u32::try_from(b.candidates.len()).unwrap_or(u32::MAX),
        probes: b.probes,
        culprit: culprit.map(str::to_string),
        fix_task: fix_task.map(str::to_string),
        reason: reason.map(str::to_string),
        at: now,
    });
    super::super::history::bisect_ended(run, (n, seq), b, result, now, fx);
}
