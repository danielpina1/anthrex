//! Decisions 35, 37, 43 and 44 (task M9.2.11): following the user's merges. anthrex
//! never lands anything: a view tells it a stage PR was merged or closed on GitHub, and
//! this moves anthrex's own state. Each pass, from the run alone (so a restart neither
//! loses nor repeats a step):
//!
//! - a **merged** stage (`StageDelivery.landed`): its base is fetched (decision 33), which
//!   also counts its merge commit's parents (ruling R-4, decision 44's method), and the
//!   next delivering stage absorbs the new base, is pushed, then **retargeted** onto the
//!   base branch (`gh pr edit --base`, the one allow-listed edit); with
//!   `delete_merged_branches`, its remote branch is deleted once no open PR is based on
//!   it. Only what the host reports at the merge landed (the final fix wave's I-1): a
//!   local head it does not hold (a held stage, ruling R-11, or a push answered after
//!   the merge) goes up with the next stage, and is an attention line when no stage
//!   above can carry it; a reply whose fix missed the merge is dropped with a line;
//! - a **closed** stage: its unfinished fix tasks are cancelled and every stage above it
//!   pauses (decision 37) until the PR is reopened; a paused stage whose tasks are all
//!   cancelled counts as skipped (the controller's re-plan ruling), and a task added to
//!   a skipped stage reopens it;
//! - the time an open PR waits on a person (decision 43), and each landed stage's
//!   `stage` history line, once (decision 44). Pure.

use proto::{
    HISTORY_VERSION, HistoryLine, MergeMethod, PrState, StageLine, StageOutcome, TaskState,
};

use super::super::requests::log;
use super::super::{Effect, wake};
use super::land_judge::{cancel_fixes, merged_head, missed_merge, unpushed};
use super::open::remote_branch;
use super::watch::{named, pr_mut};
use super::{emit, host_busy, stage_mut};
use crate::host::allow::is_object_id;
use crate::run::contract::sha7;
use crate::run::delivery::body::stacked_on;
use crate::run::delivery::ops::HostOp;
use crate::run::delivery::snapshot::stage_count;
use crate::run::delivery::{CiPhase, PrRecord};
use crate::run::model::Run;
use proto::{CiState, TaskOrigin};

/// Decision 37's attention line and wake note (TT §6.7, exact).
fn closed_line(n: u16) -> String {
    format!("stage {n} PR closed without merging; resume, re-plan, or cancel the rest")
}

/// The pass, every running `pr`-mode pass, before the base fetch and the pushes.
pub(super) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let count = stage_count(run);
    for n in 1..=count {
        landing(run, n, now, fx);
    }
    repause(run, now);
    for n in 1..=count {
        skips(run, n, now);
        waiting(run, n, now);
        history(run, n, now, fx);
        let retry = run.delivery.stage(n).and_then(|s| s.retry_at);
        if host_busy(run, n) || retry.is_some_and(|t| t > now) {
            continue;
        }
        if !retarget(run, n, fx) {
            delete(run, n, fx);
        }
    }
}

/// A PR state the stage has not processed yet: merged, closed, or reopened.
fn landing(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    // Milestone 9.3 (task 5 fix round 1): a widened run's stage 1 has no head until
    // `create_pass` records it; what landed is judged against that head, so it waits.
    if run.stage_head(n).is_none() {
        return;
    }
    let landed = run.delivery.stage(n).and_then(|s| s.landed);
    match (pr.state, landed) {
        (PrState::Merged, Some(PrState::Merged)) | (PrState::Closed, Some(PrState::Closed)) => {}
        (PrState::Open, None) => {}
        // Fix round 1 (I2): reopened and merged between two polls; the reopen's reset
        // first, so the closed line goes and the merge gets its own history line.
        (PrState::Merged, Some(PrState::Closed)) => {
            reopened(run, n, &pr, now);
            merged(run, n, &pr, now, fx);
        }
        (PrState::Merged, _) => merged(run, n, &pr, now, fx),
        (PrState::Closed, _) => closed(run, n, &pr, now, fx),
        (PrState::Open, Some(_)) => reopened(run, n, &pr, now),
    }
}

/// Decision 35: the user merged stage `n`. The base is fetched (its merge commit's
/// parents counted there), the next stage synced, pushed and retargeted. What landed is
/// the head the host reports at the merge (the final fix wave's I-1), never what
/// anthrex pushed: GitHub accepts a push to a merged PR's branch, so a push answered
/// after the user's merge counts for nothing. The local head was delivered when it is
/// that head, or when a view of the open PR showed it (`PrRecord.confirmed`; a PR head
/// only fast-forwards). Otherwise it is not delivered ([`unpushed`]), and each reply
/// whose fix missed the merge is dropped with an attention line. A top stage's
/// unfinished fix tasks are cancelled ([`cancel_fixes`], milestone 9.7 decision 2).
fn merged(run: &mut Run, n: u16, pr: &PrRecord, now: u64, fx: &mut Vec<Effect>) {
    let stage = stage_mut(run, n);
    stage.landed = Some(PrState::Merged);
    stage.held = None;
    cancel_fixes(run, n, now, fx);
    let commit = pr.merge_commit.as_deref().map_or("an unknown commit", sha7);
    log(
        run,
        now,
        format!("{}: merged on the host at {commit}", named(n, pr)),
    );
    if !pr.merge_commit.as_deref().is_some_and(is_object_id) {
        // Nothing git can count the parents of (ruling R-4; fix round 1, m2): the
        // method is unknown.
        if let Some(record) = pr_mut(run, n) {
            record.merge_method = Some(MergeMethod::None);
        }
    }
    run.delivery.base_fetch_due = true;
    let head = run.stage_head(n).unwrap_or_default().to_string();
    let at = merged_head(pr);
    let delivered = head == at || pr.confirmed.as_deref() == Some(head.as_str());
    let missed = super::reply::landed(run, n, &at, delivered);
    if !delivered {
        unpushed(run, n, pr, &head, &at, now);
    }
    if !missed.is_empty() {
        missed_merge(run, n, pr, &at, &missed, now);
    }
}

/// Ruling R-4, decision 44: the base fetch counted merge commit `oid`'s parents; two or
/// more is a merge commit, one a squash or rebase. The wake note follows (decision 35).
pub(super) fn method(run: &mut Run, oid: &str, parents: Option<u32>, now: u64) {
    let found = (1..=stage_count(run)).find(|&n| {
        run.delivery.pr(n).is_some_and(|p| {
            p.state == PrState::Merged
                && p.merge_method.is_none()
                && p.merge_commit.as_deref() == Some(oid)
        })
    });
    let Some(n) = found else {
        return;
    };
    let method = match parents {
        Some(p) if p >= 2 => MergeMethod::Merge,
        Some(1) => MergeMethod::SquashOrRebase,
        _ => MergeMethod::None,
    };
    let Some(record) = pr_mut(run, n) else {
        return;
    };
    record.merge_method = Some(method);
    let number = record.number;
    let how = match method {
        MergeMethod::Merge => "merge commit",
        MergeMethod::SquashOrRebase => "squash or rebase",
        MergeMethod::None => "method unknown",
    };
    let next = (n + 1..=stage_count(run)).find(|&m| super::sync::live(run, m));
    let rest = match next {
        Some(m) => format!("stage {m} is being synced and retargeted"),
        None => "that was the last stage".to_string(),
    };
    let text = format!("stage {n} PR #{number} was merged ({how}); {rest}");
    log(run, now, text.clone());
    wake::note(run, text);
}

/// Decision 37: the user closed stage `n`'s PR without merging. Its unfinished fix
/// tasks are cancelled; every stage above pauses (`repause`); TT's attention line and
/// wake note.
fn closed(run: &mut Run, n: u16, pr: &PrRecord, now: u64, fx: &mut Vec<Effect>) {
    stage_mut(run, n).landed = Some(PrState::Closed);
    let why = format!("stage {n} PR closed");
    for i in 0..run.tasks.len() {
        let t = &run.tasks[i];
        if t.stage() == n && t.fixes.is_some() && !t.state.is_finished() {
            super::super::complete::cancel_task(run, i, &why, now, fx);
        }
    }
    log(
        run,
        now,
        format!("{}: closed without merging", named(n, pr)),
    );
    let line = closed_line(n);
    run.delivery
        .alerts
        .insert(format!("{n}/closed"), line.clone());
    log(run, now, line.clone());
    wake::note(run, line);
}

/// Decision 37's resume: the user reopened stage `n`'s PR; the stages above resume
/// (`repause`), and its landing is processed again when it lands.
fn reopened(run: &mut Run, n: u16, pr: &PrRecord, now: u64) {
    let stage = stage_mut(run, n);
    stage.landed = None;
    stage.history_written = false;
    stage.reopens = stage.reopens.saturating_add(1);
    run.delivery.alerts.remove(&format!("{n}/closed"));
    log(
        run,
        now,
        format!("{}: reopened; the stages above resume", named(n, pr)),
    );
    // The final fix wave's I-3: its CI is judged again on its head (the fixes the close
    // cancelled are not resurrected; a red raises its fix or its alert again).
    super::ci_trigger::rearm(run, n, now);
}

/// Decision 37: each stage is paused by the lowest stage below it whose PR is closed
/// (none: not paused). Logged when it changes.
fn repause(run: &mut Run, now: u64) {
    let closed = |run: &Run, k: u16| {
        run.delivery
            .pr(k)
            .is_some_and(|p| p.state == PrState::Closed)
    };
    for m in 1..=stage_count(run) {
        let by = (1..m).find(|&k| closed(run, k));
        let was = run.delivery.stage(m).and_then(|s| s.paused_by);
        if by == was {
            continue;
        }
        stage_mut(run, m).paused_by = by;
        // I-3: an unpaused stage's CI is judged again (a red seen while it was paused
        // added nothing); its review batch, which waited, closes as usual.
        if by.is_none() {
            super::ci_trigger::rearm(run, m, now);
        }
        let text = match by {
            Some(k) => format!("stage {m}: paused (stage {k} PR closed without merging)"),
            None => format!("stage {m}: no longer paused"),
        };
        log(run, now, text);
    }
}

/// The controller's re-plan ruling: a paused stage with no PR whose tasks are all
/// cancelled counts as skipped, so the run can complete; a skipped stage that gains an
/// unfinished task is no longer skipped, so it opens its own PR rather than riding in
/// the next one.
fn skips(run: &mut Run, n: u16, now: u64) {
    let Some(stage) = run.delivery.stage(n) else {
        return;
    };
    let (paused, skipped, has_pr) = (stage.paused_by.is_some(), stage.skipped, stage.pr.is_some());
    let tasks: Vec<TaskState> = (run.tasks.iter())
        .filter(|t| t.stage() == n)
        .map(|t| t.state)
        .collect();
    if skipped && tasks.iter().any(|s| !s.is_finished()) {
        let stage = stage_mut(run, n);
        stage.skipped = false;
        stage.ready_at = None;
        log(
            run,
            now,
            format!("stage {n}: a task was added, so it is no longer skipped"),
        );
    } else if paused && !skipped && !has_pr && tasks.iter().all(|s| *s == TaskState::Cancelled) {
        stage_mut(run, n).skipped = true;
        let text = format!("stage {n}: skipped (its tasks were cancelled while it was paused)");
        log(run, now, text);
    }
}

/// Stage `n` has a PR while a stage below it delivers with no PR yet (one a task
/// reopened): its pushes wait, or the lower stage's commits would ride in its PR.
pub(crate) fn lower_unopened(run: &Run, n: u16) -> bool {
    (1..n).any(|k| {
        run.delivery
            .stage(k)
            .is_none_or(|s| !s.skipped && s.pr.is_none())
    })
}

/// Decision 43: an open PR is waiting on a person while no fix task of its stage is
/// unfinished, no check of its head is running and no CI record is being worked on.
fn waiting(run: &mut Run, n: u16, now: u64) {
    let Some(stage) = run.delivery.stage(n) else {
        return;
    };
    let Some(pr) = stage.pr.as_ref() else {
        return;
    };
    let fixing =
        (run.tasks.iter()).any(|t| t.stage() == n && t.fixes.is_some() && !t.state.is_finished());
    let ci_running = pr.checks.iter().any(|c| c.state == CiState::Pending);
    let ci_working =
        (stage.ci.iter()).any(|r| !matches!(r.phase, CiPhase::Tasked | CiPhase::ToUser));
    let open = pr.state == PrState::Open && stage.paused_by.is_none();
    let wait = open && !fixing && !ci_running && !ci_working;
    // The time since the last pass counts when the PR was waiting then; from now on it
    // counts while it is waiting now.
    let stage = stage_mut(run, n);
    if let Some(from) = stage.wait_from {
        let secs = now.saturating_sub(from);
        stage.review_wait_secs = stage.review_wait_secs.saturating_add(secs);
    }
    stage.wait_from = wait.then_some(now);
}

/// The base stage `n`'s open PR should have now: the nearest lower open stage's
/// branch, else the base branch (decision 20).
fn wanted(run: &Run, n: u16) -> String {
    match stacked_on(run, n) {
        Some((m, _)) => remote_branch(run, m),
        None => run.base_branch.clone(),
    }
}

/// The base anthrex last gave stage `n`'s PR: its retarget, else what it opened against
/// (never the host's `base`, which GitHub may have changed itself).
fn based_on(pr: &PrRecord) -> &str {
    (pr.retargeted_to.as_deref())
        .or(pr.opened_base.as_deref())
        .unwrap_or(&pr.base)
}

/// Decision 35: an open PR whose base should change (a stage below merged, or a stage
/// below opened its own PR) is retargeted once its stage is quiet: no base fetch or
/// base sync due or in flight for it, no sync task open, and its head pushed. So the
/// order after a merge is the base fetch, the base sync, the push, then the retarget.
fn retarget(run: &mut Run, n: u16, fx: &mut Vec<Effect>) -> bool {
    let Some(pr) = run.delivery.pr(n) else {
        return false;
    };
    let Some(stage) = run.delivery.stage(n) else {
        return false;
    };
    let want = wanted(run, n);
    let quiet = !run.delivery.base_fetch_due
        && !run.delivery.base_sync_due.contains_key(&n)
        && !super::open::queue_busy(run, n)
        && !(run.tasks.iter())
            .any(|t| t.sync.is_some() && t.stage() == n && !t.state.is_finished())
        && run.stage_head(n) == Some(pr.pushed_head.as_str())
        && stage.remote_head.is_none();
    let free = stage.paused_by.is_none() && stage.held.is_none();
    if pr.state != PrState::Open || based_on(pr) == want || !quiet || !free {
        return false;
    }
    let op = HostOp::Retarget {
        stage: n,
        number: pr.number,
        base: want,
    };
    emit(run, op, fx)
}

/// The retarget answered: the PR is based on `base` now.
pub(super) fn retargeted(run: &mut Run, n: u16, base: String, now: u64) {
    let Some(record) = pr_mut(run, n) else {
        return;
    };
    record.retargeted_to = Some(base.clone());
    let text = format!("{}: retargeted onto {base}", named(n, record));
    log(run, now, text);
}

/// Decision 35: with `delete_merged_branches`, a merged stage's remote branch is
/// deleted once no open PR is based on it (anthrex never deletes a branch a PR uses).
fn delete(run: &mut Run, n: u16, fx: &mut Vec<Effect>) {
    if !run.delivery.limits.delete_merged_branches {
        return;
    }
    let Some(pr) = run.delivery.pr(n) else {
        return;
    };
    if pr.state != PrState::Merged || pr.branch_deleted {
        return;
    }
    let branch = remote_branch(run, n);
    let used = (n + 1..=stage_count(run)).any(|m| {
        run.delivery
            .pr(m)
            .is_some_and(|p| p.state == PrState::Open && based_on(p) == branch)
    });
    if !used {
        emit(run, HostOp::DeleteBranch { stage: n }, fx);
    }
}

/// The branch delete answered.
pub(super) fn deleted(run: &mut Run, n: u16, now: u64) {
    let branch = remote_branch(run, n);
    let Some(record) = pr_mut(run, n) else {
        return;
    };
    record.branch_deleted = true;
    log(
        run,
        now,
        format!("stage {n}: deleted {branch} on the remote"),
    );
}

/// Decision 44: stage `n`'s `stage` history line, once its landing is processed
/// (a merge once its method is known), idempotent by `record_id`.
fn history(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    let Some(stage) = run.delivery.stage(n) else {
        return;
    };
    let Some(pr) = stage.pr.as_ref() else {
        return;
    };
    let outcome = match (stage.landed, pr.merge_method) {
        _ if stage.history_written => return,
        (Some(PrState::Merged), Some(_)) => StageOutcome::Merged,
        (Some(PrState::Closed), _) => StageOutcome::Closed,
        _ => return,
    };
    append(run, n, outcome, now, fx);
}

/// Decision 44: a `pr`-mode run cancelled with stage PRs open records each of them.
pub(crate) fn cancelled(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if !super::pr(run) {
        return;
    }
    for n in 1..=stage_count(run) {
        let open = run.delivery.pr(n).is_some_and(|p| p.state == PrState::Open);
        let written = run.delivery.stage(n).is_some_and(|s| s.history_written);
        if open && !written {
            append(run, n, StageOutcome::OpenAtCancel, now, fx);
        }
    }
}

fn append(run: &mut Run, n: u16, outcome: StageOutcome, now: u64, fx: &mut Vec<Effect>) {
    stage_mut(run, n).history_written = true;
    if !crate::run::history::enabled(run) {
        return;
    }
    let (Some(stage), Some(pr)) = (run.delivery.stage(n), run.delivery.pr(n)) else {
        return;
    };
    let count = |origin: TaskOrigin| {
        let of = |t: &&crate::run::model::Task| t.stage() == n && t.origin == origin;
        u32::try_from(run.tasks.iter().filter(of).count()).unwrap_or(u32::MAX)
    };
    let merge_method = match outcome {
        StageOutcome::Merged => pr.merge_method.unwrap_or(MergeMethod::None),
        _ => MergeMethod::None,
    };
    let line = StageLine {
        v: HISTORY_VERSION,
        // B m-4: one record per landing outcome; a reopened stage's next one is its own.
        record_id: match stage.reopens {
            0 => format!("{}/stage/{n}", run.id),
            k => format!("{}/stage/{n}-r{k}", run.id),
        },
        run_id: run.id.clone(),
        stage: n,
        pr: pr.number,
        time_to_open_secs: pr
            .opened_at
            .saturating_sub(stage.ready_at.unwrap_or(pr.opened_at)),
        human_review_secs: stage.review_wait_secs,
        ci_rounds: count(TaskOrigin::Ci),
        review_rounds: stage.review_rounds,
        sync_tasks: count(TaskOrigin::Sync),
        outcome,
        merge_method,
        at: now,
    };
    super::super::history::append(run, None, HistoryLine::Stage(line), fx);
}

/// Decision 37 (ruling R-8): every landed stage is fully processed: its landing seen,
/// a merge's method known and its history line out, and no base fetch is due; and no
/// merged stage left work undelivered.
pub(super) fn settled(run: &Run) -> bool {
    let unlanded = run.delivery.alerts.keys().any(|k| k.ends_with("/unlanded"));
    !run.delivery.base_fetch_due && !unlanded && (1..=stage_count(run)).all(|n| processed(run, n))
}

/// [`settled`]'s per stage: stage `n`'s landing, if its PR merged or closed, is
/// processed (seen, and its history line out); true for an open PR or none (also
/// `goal_rounds::landed_below`, milestone 9.3).
pub(in crate::run::engine) fn processed(run: &Run, n: u16) -> bool {
    let Some(stage) = run.delivery.stage(n) else {
        return true;
    };
    match stage.pr.as_ref() {
        Some(pr) if pr.state != PrState::Open => {
            stage.landed == Some(pr.state) && stage.history_written
        }
        _ => true,
    }
}
