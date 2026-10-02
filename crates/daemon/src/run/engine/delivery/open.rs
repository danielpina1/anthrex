//! Decisions 19 and 20 (task M9.2.7): when a stage is ready to be delivered, and how
//! its pull request opens. Stages open in order: a stage waits for every one of its
//! tasks, for its merge queue, and for the stage below to have an open (or merged) PR
//! or to have been skipped. Then tier 3 must be green on its current head (9.1's
//! `full::request`, which runs the profile's `check` for an untiered profile); a profile
//! with no `check` opens unverified. The head is pushed fast-forward to
//! `anthrex/<run>/stage-<n>` (in every layout: the push names a commit), then the PR is
//! created against the base branch or the nearest lower open stage's branch. A stage
//! that merged nothing is skipped: GitHub refuses a PR without commits. Every step is
//! re-derived from the run on each pass, so a host op a restart dropped is simply
//! emitted again (decision 10). Pure (design decision 2).

use proto::{PrState, RunState, TaskState};

use super::super::full::{self, FullWhy};
use super::super::merge::halt;
use super::super::requests::log;
use super::super::{Effect, OpKind};
use super::{emit, host_busy, stage_mut};
use crate::host::{PrRef, PushOutcome};
use crate::run::contract::sha7;
use crate::run::delivery::PrRecord;
use crate::run::delivery::body::{pr_body, pr_title, stacked_on};
use crate::run::delivery::ops::HostOp;
use crate::run::delivery::snapshot::stage_count;
use crate::run::model::Run;

/// TT §6.2's halt when the remote stage branch is not an ancestor of the pushed head.
pub(crate) const REWRITTEN: &str = "remote stage branch was rewritten by someone else";

/// `anthrex/<run>/stage-<n>`: a stage's branch on the remote (decision 20).
pub(super) fn remote_branch(run: &Run, n: u16) -> String {
    format!("anthrex/{}/stage-{n}", run.id)
}

/// Decision 19's first four conditions for stage `n`; `Err` names the first that fails,
/// as `run deliver` quotes it (Interfaces "Messages").
pub(super) fn ready(run: &Run, n: u16) -> Result<(), String> {
    if n == 0 || n > stage_count(run) {
        return Err(format!("it has no stage {n}"));
    }
    let unfinished = (run.tasks.iter())
        .filter(|t| t.stage() == n && !t.state.is_finished())
        .count();
    if unfinished > 0 {
        return Err(format!("{unfinished} of its tasks are not finished"));
    }
    if queue_busy(run, n) {
        return Err("its merge queue is busy".to_string());
    }
    // Decision 37: a stage below whose PR was closed without merging pauses this one.
    let closed = |m: u16| {
        run.delivery
            .pr(m)
            .is_some_and(|p| p.state == PrState::Closed)
    };
    if let Some(m) = (1..n).find(|&m| closed(m)) {
        return Err(format!("stage {m}'s PR was closed without merging"));
    }
    let below = n - 1;
    let landed = |m: u16| {
        run.delivery.stage(m).is_some_and(|s| s.skipped)
            || (run.delivery.pr(m))
                .is_some_and(|p| matches!(p.state, PrState::Open | PrState::Merged))
    };
    if n > 1 && !landed(below) {
        return Err(format!("stage {below}'s PR is not open"));
    }
    Ok(())
}

/// No merge-queue item targets stage `n`: no queued task of it, no due propagate or
/// base sync into it, and no candidate or propagate in flight for it.
pub(super) fn queue_busy(run: &Run, n: u16) -> bool {
    let of_stage = |id: &str| run.task(id).is_some_and(|t| t.stage() == n);
    run.merge_queue.iter().any(|id| of_stage(id))
        || run.propagate_due.contains(&n)
        || run.delivery.base_sync_due.contains_key(&n)
        || run.pending_ops.values().any(|p| match &p.kind {
            OpKind::MergeCandidate { .. } => p.task_id.as_deref().is_some_and(of_stage),
            OpKind::Propagate(spec) => spec.to == n,
            _ => false,
        })
}

/// The stage merged nothing (every task cancelled, or reported): its head is the stage
/// below's, and GitHub refuses a PR without commits (decision 19).
fn empty(run: &Run, n: u16) -> bool {
    !(run.tasks.iter()).any(|t| t.stage() == n && t.state == TaskState::Merged)
}

/// Every stage, lowest first: a ready stage is skipped when empty, else pushed once tier
/// 3 is green on its head. At most one host op per stage is in flight, and a stage whose
/// last opening op failed waits for its retry time.
pub(super) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for n in 1..=stage_count(run) {
        let done = run
            .delivery
            .stage(n)
            .is_some_and(|s| s.pr.is_some() || s.skipped);
        let waiting = run.delivery.stage(n).and_then(|s| s.retry_at) > Some(now)
            || run.delivery.stage(n).is_some_and(|s| s.held.is_some())
            || super::watch::lower_held(run, n);
        if done || waiting || host_busy(run, n) || ready(run, n).is_err() {
            continue;
        }
        // Decision 44's `time_to_open` starts when the stage is first ready.
        stage_mut(run, n).ready_at.get_or_insert(now);
        if empty(run, n) {
            stage_mut(run, n).skipped = true;
            log(run, now, format!("stage {n}: skipped (no changes)"));
            continue;
        }
        if let Some(head) = verified_head(run, n, now, fx) {
            push(run, n, head, now, fx);
        }
    }
}

/// Decision 19's last condition: stage `n`'s head when tier 3 is green on it, or when
/// the profile has no `check` (the run is unverified, and the PR body says so). Else
/// tier 3 is requested on it, unless it is red there: 9.1's bisect or a fix task moves
/// the head first.
fn verified_head(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) -> Option<String> {
    let head = run.stage_head(n)?.to_string();
    if run.profile.check.is_none() {
        return Some(head);
    }
    let full = &run.stage(n)?.full;
    if full.green_at.as_deref() == Some(head.as_str()) {
        return Some(head);
    }
    if full.red_at.as_deref() != Some(head.as_str()) {
        request_tier3(run, n, now, fx);
    }
    None
}

/// Decision 19: tier 3 on stage `n`'s head at "before a PR opens" priority (9.1's
/// `FullStage`), unless it is green already or a tier-3 job or bisect is in flight.
pub(super) fn request_tier3(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    full::request(run, n, FullWhy::Deliver, now, fx);
}

/// Decision 20, first step: the head to `refs/heads/anthrex/<run>/stage-<n>`,
/// fast-forward only (the executor never forces).
fn push(run: &mut Run, n: u16, head: String, now: u64, fx: &mut Vec<Effect>) {
    let to = remote_branch(run, n);
    let text = format!("stage {n}: pushing {} to {to}", sha7(&head));
    if emit(
        run,
        HostOp::Push {
            stage: n,
            sha: head,
        },
        fx,
    ) {
        log(run, now, text);
    }
}

/// A push of stage `n`'s head `sha`, before its PR exists. Pushed (or already there):
/// the PR is created, unless the run stopped, the head moved, or the stage is no longer
/// ready meanwhile (the next pass pushes again once it is; review m1). A
/// non-fast-forward halts (decision 13; nothing is ever forced); a push the remote
/// refused (ruling R-11: protection, a ruleset, a hook) holds the stage (`watch.rs`).
pub(super) fn pushed(
    run: &mut Run,
    n: u16,
    sha: String,
    outcome: PushOutcome,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let to = remote_branch(run, n);
    match outcome {
        PushOutcome::Pushed | PushOutcome::UpToDate => {
            let stage = stage_mut(run, n);
            stage.pushed = Some(sha.clone());
            stage.held = None;
            let current = run.stage_head(n) == Some(sha.as_str());
            let stopped = run.state != RunState::Running || run.cancelled;
            if stopped || !current || ready(run, n).is_err() {
                return;
            }
            let base = match stacked_on(run, n) {
                Some((m, _)) => remote_branch(run, m),
                None => run.base_branch.clone(),
            };
            let (title, body) = (pr_title(run, n), pr_body(run, n));
            let op = HostOp::OpenPr {
                stage: n,
                base,
                head: to,
                title,
                body,
            };
            emit(run, op, fx);
        }
        PushOutcome::Rejected { reason } => {
            let remote = run.delivery.repo.as_ref().map_or("", |r| r.remote.as_str());
            let line = format!(
                "stage {n}: {remote} refused to fast-forward {to} to {} ({reason})",
                sha7(&sha)
            );
            log(run, now, line);
            halt(run, REWRITTEN.to_string(), now);
        }
        PushOutcome::Refused { reason } => super::watch::refused(run, n, reason, now),
    }
}

/// Decision 20, last step: the PR is recorded on stage `n`, with the head the opening
/// push sent. A PR found already open for the head (a restart between its creation and
/// this record, decision 10) is adopted as it is.
pub(super) fn opened(run: &mut Run, n: u16, base: String, pr: PrRef, now: u64) {
    if run.delivery.pr(n).is_some() {
        return;
    }
    let head = run.stage_head(n).unwrap_or_default().to_string();
    let poll = now.saturating_add(super::watch::base_secs(run));
    let stage = stage_mut(run, n);
    let pushed_head = stage.pushed.take().unwrap_or(head);
    stage.retry_at = None;
    stage.pr = Some(PrRecord {
        number: pr.number,
        url: pr.url.clone(),
        base: base.clone(),
        opened_at: now,
        pushed_head,
        state: pr.state,
        merged_at: None,
        merge_commit: None,
        merge_method: None,
        next_poll_at: poll,
        unchanged_views: 0,
        last_view_at: None,
        watermark: Default::default(),
        checks: Vec::new(),
        retargeted_to: None,
        opened_base: Some(base.clone()),
        branch_deleted: false,
    });
    let text = match pr.existed {
        true => format!(
            "stage {n}: PR #{} already existed; adopted: {}",
            pr.number, pr.url
        ),
        false => format!("stage {n}: PR #{} opened: {}", pr.number, pr.url),
    };
    log(run, now, text);
}
