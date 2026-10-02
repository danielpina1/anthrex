//! Decisions 23, 24 and 28 (task M9.2.8): watching an open stage PR, and keeping its
//! branch in step with the stage. Each pass, per stage with a PR and no host op in
//! flight, at most one op goes out, in this order:
//!
//! 1. an **adopt**: a view saw a remote head that is not the pushed one, so the remote
//!    branch is fetched and, when it descends from the stage head, becomes it (a
//!    user's own commit); it waits for the stage's merge queue, and the queue waits
//!    for it ([`stage_busy`]);
//! 2. a **push**: the stage head moved past the pushed head (a merged fix, a
//!    propagate), the queue is quiet, the stage is not paused or held;
//! 3. a **view**, when due: on the interval `poll_base_secs × 2^k` while open (`k`
//!    counting the views in a row that changed nothing), at the cap
//!    `max(poll_max_secs, poll_base_secs)` once merged or closed.
//!
//! A view is processed against the PR's watermark in the step that answers it
//! (`view.rs`). A push the
//! remote refuses holds its stage (ruling R-11) until `run resume`. The run's delivery
//! attention lines are [`attention`]'s. Pure (design decision 2): the clock is the
//! event's `now`.

use proto::PrState;

use super::super::merge::halt;
use super::super::requests::log;
use super::super::stages::{forget_line, highest, set_stage_head};
use super::super::{Effect, OpKind, wake};
use super::open::{REWRITTEN, queue_busy, remote_branch};
use super::{emit, failure_key, host_busy, pr, stage_mut};
use crate::host::{Adopt, FetchOutcome, PushOutcome};
use crate::run::contract::sha7;
use crate::run::delivery::PrRecord;
use crate::run::delivery::ops::HostOp;
use crate::run::delivery::snapshot::stage_count;
use crate::run::model::{Run, StageLayout};

/// Decision 11: a rate limit doubles the poll interval base up to an hour.
pub(crate) const POLL_BASE_MAX_SECS: u64 = 3_600;
/// The alert key of a lost login (decision 11).
const AUTH: &str = "auth";

/// Decision 11's interval base: `poll_base_secs` (from `poll_secs`, doubled by rate
/// limits), at least one second.
pub(super) fn base_secs(run: &Run) -> u64 {
    let d = &run.delivery;
    let base = if d.poll_base_secs == 0 {
        d.limits.poll_secs
    } else {
        d.poll_base_secs
    };
    base.max(1)
}

/// Decision 23's cap, `max(poll_max_secs, poll_base_secs)`.
pub(super) fn cap_secs(run: &Run) -> u64 {
    run.delivery.limits.poll_max_secs.max(base_secs(run))
}

/// Decision 23: the interval after `k` views in a row that changed nothing.
pub(super) fn interval(run: &Run, k: u32) -> u64 {
    let doubled = base_secs(run).saturating_mul(1u64 << k.min(20));
    doubled.min(cap_secs(run))
}

/// Decision 24: an adopt of stage `n` is in flight; 9.1's `start_merge` starts no
/// candidate or propagate for that stage until it answers.
pub(crate) fn stage_busy(run: &Run, n: u16) -> bool {
    run.pending_ops.values().any(|p| {
        matches!(&p.kind, OpKind::Host { op: HostOp::Fetch { stage: Some(s), adopt: Some(_), .. }, .. } if *s == n)
    })
}

/// Decision 37: stage `n` is paused by a lower stage's PR closed without merging; none
/// of its tasks that has not started starts.
pub(crate) fn stage_paused(run: &Run, n: u16) -> bool {
    pr(run) && run.delivery.stage(n).is_some_and(|s| s.paused_by.is_some())
}

/// Ruling R-11: a stage is held by a push the remote refused.
pub(crate) fn held(run: &Run) -> bool {
    pr(run) && run.delivery.stages.iter().any(|s| s.held.is_some())
}

/// `run resume` (ruling R-11): every held stage pushes again; the stages released.
pub(crate) fn release(run: &mut Run, now: u64) -> Vec<u16> {
    let mut released = Vec::new();
    for (n, s) in (1u16..).zip(run.delivery.stages.iter_mut()) {
        if s.held.take().is_some() {
            s.retry_at = None;
            released.push(n);
        }
    }
    for n in &released {
        log(
            run,
            now,
            format!("stage {n}: the push retries (run resume)"),
        );
    }
    released
}

/// The run's delivery attention lines: a lost login, an op failing again and again
/// (decision 11), and each held stage (ruling R-11).
pub(crate) fn attention(run: &Run) -> Vec<String> {
    if !pr(run) {
        return Vec::new();
    }
    let mut lines: Vec<String> = run.delivery.alerts.values().cloned().collect();
    for (n, s) in (1u16..).zip(&run.delivery.stages) {
        if let Some(reason) = &s.held {
            lines.push(hold_line(run, n, reason));
        }
    }
    lines
}

/// Ruling R-11's attention line and wake note for a held stage.
fn hold_line(run: &Run, n: u16, reason: &str) -> String {
    format!(
        "stage {n} is held: {reason}; anthrex run resume {} pushes it again",
        run.id
    )
}

/// `stage <n> (PR #<k>)`, how the log names a stage PR.
pub(super) fn named(n: u16, pr: &PrRecord) -> String {
    format!("stage {n} (PR #{})", pr.number)
}

pub(super) fn pr_mut(run: &mut Run, n: u16) -> Option<&mut PrRecord> {
    stage_mut(run, n).pr.as_mut()
}

/// Every stage with a PR, lowest first: an adopt, else a push, else a view when due.
pub(super) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for n in 1..=stage_count(run) {
        let Some(pr) = run.delivery.pr(n) else {
            continue;
        };
        if host_busy(run, n) {
            continue;
        }
        let open = pr.state == PrState::Open;
        let due = pr.next_poll_at <= now;
        let number = pr.number;
        if open && (adopt(run, n, now, fx) || push(run, n, now, fx)) {
            continue;
        }
        if run.delivery.watching && due {
            emit(run, HostOp::ViewPr { stage: n, number }, fx);
        }
    }
}

/// Whether stage `n` may move or push: not paused, not held, the queue quiet, and not
/// waiting for a failed op's retry.
fn free(run: &Run, n: u16, now: u64) -> bool {
    let Some(s) = run.delivery.stage(n) else {
        return false;
    };
    s.paused_by.is_none()
        && s.held.is_none()
        && s.retry_at.is_none_or(|t| t <= now)
        && !queue_busy(run, n)
}

/// Decision 24: a view saw `remote_head` on the stage branch; fetch it to adopt it onto
/// the stage head (and `integration` with the highest stage of a `Multi` run).
fn adopt(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) -> bool {
    let due = run.delivery.stage(n).and_then(|s| s.remote_head.clone());
    let (Some(remote), Some(head)) = (due, run.stage_head(n).map(str::to_string)) else {
        return false;
    };
    if !free(run, n, now) {
        return false;
    }
    let branch = remote_branch(run, n);
    let multi = run.stage_layout == StageLayout::Multi;
    let op = HostOp::Fetch {
        stage: Some(n),
        branch: branch.clone(),
        into: format!("refs/anthrex/{}/remote/stage-{n}", run.id),
        adopt: Some(Adopt {
            local_ref: run.stage_branch(n),
            expected_local: head,
            also_integration: multi && n >= highest(run),
        }),
    };
    let sent = emit(run, op, fx);
    if sent {
        let text = format!("stage {n}: fetching {branch} to adopt {}", sha7(&remote));
        log(run, now, text);
    }
    sent
}

/// Decision 28: the stage head moved past the pushed head; push it (tier 2 passed in
/// the merge queue; on an open PR, CI runs the full suite).
fn push(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) -> bool {
    let Some(head) = run.stage_head(n).map(str::to_string) else {
        return false;
    };
    let moved = run.delivery.pr(n).is_some_and(|p| p.pushed_head != head);
    let adopting = run
        .delivery
        .stage(n)
        .is_some_and(|s| s.remote_head.is_some());
    if !moved || adopting || !free(run, n, now) {
        return false;
    }
    let text = format!(
        "stage {n}: pushing {} to {}",
        sha7(&head),
        remote_branch(run, n)
    );
    let sent = emit(
        run,
        HostOp::Push {
            stage: n,
            sha: head,
        },
        fx,
    );
    if sent {
        log(run, now, text);
    }
    sent
}

/// A push of an open PR's stage answered (decision 28): the PR now carries `sha`.
pub(super) fn pushed(run: &mut Run, n: u16, sha: String, outcome: PushOutcome, now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    match outcome {
        PushOutcome::Pushed | PushOutcome::UpToDate => {
            let base = base_secs(run);
            stage_mut(run, n).held = None;
            if let Some(record) = pr_mut(run, n) {
                record.pushed_head = sha.clone();
                // A new head is a change: CI starts on it, so the next view comes soon.
                record.unchanged_views = 0;
                record.next_poll_at = record.next_poll_at.min(now.saturating_add(base));
            }
            log(
                run,
                now,
                format!("{}: pushed {}", named(n, &pr), sha7(&sha)),
            );
        }
        PushOutcome::Rejected { reason } => {
            let remote = run.delivery.repo.as_ref().map_or("", |r| r.remote.as_str());
            let to = remote_branch(run, n);
            let line = format!(
                "{}: {remote} refused to fast-forward {to} to {} ({reason})",
                named(n, &pr),
                sha7(&sha)
            );
            log(run, now, line);
            rewritten(run, n, now);
        }
        PushOutcome::Refused { reason } => refused(run, n, reason, now),
    }
}

/// Ruling R-11: the remote refused stage `n`'s push (protection, a ruleset, a hook).
/// The stage is held, with an attention line and a wake note; the run's other work
/// goes on, and `run resume` (or the next successful push) releases it.
pub(super) fn refused(run: &mut Run, n: u16, reason: String, now: u64) {
    let line = hold_line(run, n, &reason);
    stage_mut(run, n).held = Some(reason);
    log(run, now, line.clone());
    wake::note(run, line);
}

/// TT §6.2's halt: the remote stage branch was rewritten. `run resume` polls again
/// (a view first, so a PR closed meanwhile is seen) and halts again while it is so.
fn rewritten(run: &mut Run, n: u16, now: u64) {
    halt(run, REWRITTEN.to_string(), now);
    run.halt_retryable = true;
    stage_mut(run, n).remote_head = None;
    if let Some(record) = pr_mut(run, n) {
        record.next_poll_at = now;
    }
}

/// Decision 24: what the adopt's fetch found.
pub(super) fn fetched(
    run: &mut Run,
    n: u16,
    outcome: FetchOutcome,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let branch = remote_branch(run, n);
    let head = run.stage_head(n).unwrap_or_default().to_string();
    let line = match outcome {
        FetchOutcome::Adopted { sha } => {
            stage_mut(run, n).remote_head = None;
            if let Some(record) = pr_mut(run, n) {
                record.pushed_head = sha.clone();
            }
            if sha == head {
                return;
            }
            // 9.1's one writer of stage heads: stage n + 1 is due its propagate. Ruling
            // R-9: a commit the engine never wrote on the line is its floor.
            set_stage_head(run, n, &sha);
            forget_line(run, n, now, fx);
            format!("{}: adopted {} from {branch}", named(n, &pr), sha7(&sha))
        }
        FetchOutcome::NotDescendant { remote } if remote == pr.pushed_head => {
            // The view was behind the branch: it holds what anthrex pushed.
            stage_mut(run, n).remote_head = None;
            format!("{}: {branch} is still at {}", named(n, &pr), sha7(&remote))
        }
        FetchOutcome::NotDescendant { remote } => {
            let name = run.delivery.repo.as_ref().map_or("", |r| r.remote.as_str());
            let line = format!(
                "{}: {name} has {}, which does not contain {}",
                named(n, &pr),
                sha7(&remote),
                sha7(&head)
            );
            log(run, now, line);
            return rewritten(run, n, now);
        }
        FetchOutcome::LocalMoved { local } => {
            // The stage moved meanwhile; the next pass adopts from where it is now.
            format!(
                "{}: {} moved to {}; adopting again",
                named(n, &pr),
                run.stage_branch(n),
                sha7(&local)
            )
        }
        FetchOutcome::Missing => {
            stage_mut(run, n).remote_head = None;
            format!("{}: {branch} is gone from the remote", named(n, &pr))
        }
        FetchOutcome::Fetched { .. } => {
            stage_mut(run, n).remote_head = None;
            return;
        }
    };
    log(run, now, line);
}

/// A host op succeeded: its failures in a row end (decision 11), a lost login is back,
/// and a rate-limited interval base returns to `poll_secs`.
pub(super) fn succeeded(run: &mut Run, op: &HostOp) {
    let key = failure_key(op);
    let d = &mut run.delivery;
    d.failures.remove(&key);
    d.alerts.remove(&key);
    d.alerts.remove(AUTH);
    if d.poll_base_secs != d.limits.poll_secs && d.poll_base_secs != 0 {
        d.poll_base_secs = d.limits.poll_secs;
    }
}

/// Decision 11: a rate limit doubles the interval base, up to an hour.
pub(super) fn rate_limited(run: &mut Run) {
    let base = base_secs(run).saturating_mul(2).min(POLL_BASE_MAX_SECS);
    run.delivery.poll_base_secs = base.max(run.delivery.limits.poll_secs);
}

/// Decision 11: `gh` lost its login; an attention line until the next success.
pub(super) fn auth_lost(run: &mut Run) {
    let host = (run.delivery.repo.as_ref()).map_or("github.com", |r| r.host.as_str());
    let line = format!("gh is no longer logged in to {host}; run gh auth login");
    run.delivery.alerts.insert(AUTH.to_string(), line);
}

/// Decision 11: after [`FAILURES_BEFORE_ATTENTION`] failures in a row of one op on one
/// stage, `PR #<pr>: <op> keeps failing: <error>`.
///
/// [`FAILURES_BEFORE_ATTENTION`]: crate::run::delivery::FAILURES_BEFORE_ATTENTION
pub(super) fn keeps_failing(run: &mut Run, op: &HostOp, name: &str, error: &str) {
    let key = failure_key(op);
    let failures = run.delivery.failures.get(&key).copied().unwrap_or(0);
    if failures < crate::run::delivery::FAILURES_BEFORE_ATTENTION {
        return;
    }
    let stage = super::stage_of(op);
    let what = match stage.and_then(|n| run.delivery.pr(n)) {
        Some(pr) => format!("PR #{}", pr.number),
        None => stage.map_or_else(|| "the run".to_string(), |n| format!("stage {n}")),
    };
    let line = format!("{what}: {name} keeps failing: {error}");
    run.delivery.alerts.insert(key, line);
}

/// A view failed: polling goes on at the backed-off interval (a rate limit has already
/// doubled the base, so it does not back off twice).
pub(super) fn view_failed(run: &mut Run, n: u16, rate_limited: bool, now: u64) {
    let k = run.delivery.pr(n).map_or(0, |p| p.unchanged_views);
    let k = if rate_limited { k } else { k.saturating_add(1) };
    let wait = interval(run, k);
    if let Some(record) = pr_mut(run, n) {
        record.unchanged_views = k;
        record.next_poll_at = now.saturating_add(wait);
    }
}
