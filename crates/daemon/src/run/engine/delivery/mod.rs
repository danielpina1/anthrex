//! Milestone 9.2's delivery, engine side (design decision 1: pure, like everything under
//! `engine/`). Task M9.2.7 builds the delivery pass the scheduler runs on every running
//! `pr`-mode run ([`pass`]), the routing of host ops' answers ([`host_done`]), the
//! `run deliver` and `run watch` requests ([`request`], decision 25), completion's extra
//! condition ([`may_complete`], decision 37), the refusals of `run accept` and `run
//! discard` (decisions 38–39, ruling R-1) and the outcome of a cancelled run (decision
//! 39). Opening a stage's pull request is `open.rs` (decisions 19–20). A `local`-mode
//! run never reaches any of it: it emits no host op.

use proto::{DeliveryMode, FinishAction, PrState, RunState};

use super::merge::halt;
use super::requests::log;
use super::{Effect, EngineState, OpKind, OpResult, ReplyId, emit_op, next_op};
use crate::host::HostError;
use crate::run::delivery::StageDelivery;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::model::Run;

mod open;

/// Decision 25: `run deliver` and `run watch`, as the driver hands them to the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryRequest {
    Deliver {
        reply: ReplyId,
        run_id: String,
        stage: u16,
    },
    Watch {
        reply: ReplyId,
        run_id: String,
        on: bool,
    },
}

impl DeliveryRequest {
    /// The request waiting for its answer.
    pub fn reply(&self) -> ReplyId {
        match self {
            DeliveryRequest::Deliver { reply, .. } | DeliveryRequest::Watch { reply, .. } => *reply,
        }
    }
}

/// The run is delivered by pull request.
pub(crate) fn pr(run: &Run) -> bool {
    run.delivery.mode == DeliveryMode::Pr
}

/// Stage `n`'s delivery record, created (with every one below it) when missing.
pub(super) fn stage_mut(run: &mut Run, n: u16) -> &mut StageDelivery {
    let i = usize::from(n.max(1)) - 1;
    let stages = &mut run.delivery.stages;
    if stages.len() <= i {
        stages.resize_with(i + 1, StageDelivery::default);
    }
    &mut stages[i]
}

/// The stage a host op is about; `None` for a run-level one (`Permission`, a base fetch).
pub(crate) fn stage_of(op: &HostOp) -> Option<u16> {
    match op {
        HostOp::Push { stage, .. }
        | HostOp::OpenPr { stage, .. }
        | HostOp::ViewPr { stage, .. }
        | HostOp::FailedLogs { stage, .. }
        | HostOp::RerunFailed { stage, .. }
        | HostOp::Reply { stage, .. }
        | HostOp::Retarget { stage, .. }
        | HostOp::DeleteBranch { stage } => Some(*stage),
        HostOp::Fetch { stage, .. } => *stage,
        HostOp::Permission { .. } => None,
    }
}

/// A host op's name in log lines and in `RunDelivery.failures` (decision 11).
pub(crate) fn op_name(op: &HostOp) -> &'static str {
    match op {
        HostOp::Push { .. } => "push",
        HostOp::Fetch { .. } => "fetch",
        HostOp::OpenPr { .. } => "open_pr",
        HostOp::ViewPr { .. } => "view_pr",
        HostOp::FailedLogs { .. } => "failed_logs",
        HostOp::RerunFailed { .. } => "rerun_failed",
        HostOp::Reply { .. } => "reply",
        HostOp::Retarget { .. } => "retarget",
        HostOp::Permission { .. } => "permission",
        HostOp::DeleteBranch { .. } => "delete_branch",
    }
}

/// `"<stage>/<op>"` (`"run/<op>"` for a run-level op), decision 11's failure key.
fn failure_key(op: &HostOp) -> String {
    let stage = stage_of(op).map_or_else(|| "run".to_string(), |n| n.to_string());
    format!("{stage}/{}", op_name(op))
}

/// Decision 8: at most one host op per stage is in flight.
pub(crate) fn host_busy(run: &Run, n: u16) -> bool {
    run.pending_ops
        .values()
        .any(|p| matches!(&p.kind, OpKind::Host { op, .. } if stage_of(op) == Some(n)))
}

/// Emits `op` against the run's frozen repository; `false` (nothing emitted) when the
/// run has none, which only a `local`-mode run lacks.
pub(super) fn emit(run: &mut Run, op: HostOp, fx: &mut Vec<Effect>) -> bool {
    let Some(repo) = run.delivery.repo.clone() else {
        return false;
    };
    let id = next_op(run);
    emit_op(run, id, None, OpKind::Host { repo, op }, fx);
    true
}

/// The delivery pass, every scheduler pass of a running run (`dispatch::schedule`):
/// nothing in `local` mode, nor once the run is cancelled (decision 39: watching stops,
/// nothing more opens or is pushed).
pub(super) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if !pr(run) || run.cancelled || run.state != RunState::Running {
        return;
    }
    open::pass(run, now, fx);
}

/// A host op's answer (decision 8). An error is decision 11's; a `Push` or an `OpenPr`
/// of a stage with no PR yet is opening it (`open.rs`). Every other answer is a later
/// task's (watching, CI, reviews, landing).
pub(super) fn host_done(
    run: &mut Run,
    op: HostOp,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let OpResult::Host(result) = result else {
        return;
    };
    if let HostResult::Error(error) = result {
        return failed(run, &op, error, now);
    }
    run.delivery.failures.remove(&failure_key(&op));
    match (op, result) {
        (HostOp::Push { stage, sha }, HostResult::Pushed(outcome))
            if run.delivery.pr(stage).is_none() =>
        {
            open::pushed(run, stage, sha, outcome, now, fx)
        }
        (HostOp::OpenPr { stage, base, .. }, HostResult::PrOpened(pr)) => {
            open::opened(run, stage, base, pr, now)
        }
        _ => {}
    }
}

/// Decision 11: `Forbidden` is a bug, and the run halts on it; any other error is a
/// line on the run's log and a retry when next due (for opening: after the poll
/// interval, so a lasting failure is not retried every second).
fn failed(run: &mut Run, op: &HostOp, error: HostError, now: u64) {
    if let HostError::Forbidden(text) = &error {
        let argv = text.strip_prefix("anthrex never runs: ").unwrap_or(text);
        return halt(
            run,
            format!("anthrex refused its own host command: {argv}"),
            now,
        );
    }
    *run.delivery.failures.entry(failure_key(op)).or_default() += 1;
    let text: String = error.text().chars().take(200).collect();
    let Some(n) = stage_of(op) else {
        return log(run, now, format!("{} failed: {text}", op_name(op)));
    };
    log(
        run,
        now,
        format!("stage {n}: {} failed: {text}", op_name(op)),
    );
    let wait = retry_secs(run);
    stage_mut(run, n).retry_at = Some(now.saturating_add(wait));
}

/// The interval a failed opening op waits: the poll interval base (decision 11), at
/// least `poll_secs` and one second.
fn retry_secs(run: &Run) -> u64 {
    let d = &run.delivery;
    d.poll_base_secs.max(d.limits.poll_secs).max(1)
}

/// Decision 37 (ruling R-8): a `pr`-mode run completes once every stage PR is merged or
/// closed on the host (a skipped stage counts as done); a cancelled run does not wait
/// (decision 39). Always true in `local` mode.
pub(super) fn may_complete(run: &Run) -> bool {
    if !pr(run) || run.cancelled {
        return true;
    }
    let count = crate::run::delivery::snapshot::stage_count(run);
    (1..=count).all(|n| {
        let skipped = run.delivery.stage(n).is_some_and(|s| s.skipped);
        let landed = |state| matches!(state, PrState::Merged | PrState::Closed);
        skipped || run.delivery.pr(n).is_some_and(|p| landed(p.state))
    })
}

/// Decision 39: a cancelled `pr`-mode run completes with the PRs it leaves open,
/// `cancelled; <k> pull requests left open: #<a>, #<b>` (one: `1 pull request`), or
/// `cancelled` with none open. `None` for any other run.
pub(crate) fn cancel_outcome(run: &Run) -> Option<String> {
    if !pr(run) || !run.cancelled {
        return None;
    }
    let open: Vec<String> = (run.delivery.stages.iter())
        .filter_map(|s| s.pr.as_ref())
        .filter(|p| p.state == PrState::Open)
        .map(|p| format!("#{}", p.number))
        .collect();
    Some(match open.len() {
        0 => "cancelled".to_string(),
        1 => format!("cancelled; 1 pull request left open: {}", open[0]),
        k => format!(
            "cancelled; {k} pull requests left open: {}",
            open.join(", ")
        ),
    })
}

/// Decisions 38–39 and ruling R-1: `run accept` is refused in `pr` mode, and `run
/// discard` until the run is complete (then it removes local worktrees and branches
/// only). `None`: not refused for its delivery.
pub(crate) fn finish_refusal(run: &Run, action: FinishAction) -> Option<String> {
    let id = &run.id;
    match action {
        _ if !pr(run) => None,
        FinishAction::Accept => Some(format!(
            "run {id} is delivered by pull request; merge its pull requests on GitHub (anthrex never merges)"
        )),
        FinishAction::Discard => (run.state != RunState::Complete).then(|| {
            format!(
                "run {id} is delivered by pull request; its branches back its pull requests, so discard is refused (anthrex run cancel stops its agents)"
            )
        }),
    }
}

/// Decision 25: `run deliver` and `run watch`, each answered at once.
pub(super) fn request(
    state: &mut EngineState,
    request: DeliveryRequest,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let reply = request.reply();
    let result = match request {
        DeliveryRequest::Deliver { run_id, stage, .. } => deliver(state, &run_id, stage, now, fx),
        DeliveryRequest::Watch { run_id, on, .. } => watch(state, &run_id, on, now),
    };
    fx.push(Effect::Reply { reply, result });
}

/// Both requests' refusals: a `local`-mode run, then an ended one, then a cancelled one.
fn refusal(run: &Run) -> Option<String> {
    let id = &run.id;
    if !pr(run) {
        return Some(format!(
            "run {id} delivers locally; run deliver and run watch apply to pr mode"
        ));
    }
    if run.state.is_terminal() || run.state == RunState::Complete {
        return Some(format!("run {id} is {}", run.state.label()));
    }
    run.cancelled.then(|| format!("run {id} was cancelled"))
}

/// Decision 25's `run deliver`: tier 3 on the stage head now, and its PR as soon as
/// decision 19 holds; refused with decision 19's first failing condition.
fn deliver(
    state: &mut EngineState,
    run_id: &str,
    n: u16,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<String, String> {
    let run = state
        .runs
        .get_mut(run_id)
        .ok_or_else(|| format!("unknown run {run_id}"))?;
    if let Some(text) = refusal(run) {
        return Err(text);
    }
    if run.state != RunState::Running {
        return Err(format!("run {run_id} is {}", run.state.label()));
    }
    if let Some(pr) = run.delivery.pr(n) {
        return match pr.state {
            PrState::Open => Ok(format!("stage {n}'s PR is already open: {}", pr.url)),
            PrState::Merged => Err(format!("stage {n}'s PR is already merged: {}", pr.url)),
            PrState::Closed => Err(format!(
                "stage {n}'s PR was closed without merging: {}",
                pr.url
            )),
        };
    }
    if run.delivery.stage(n).is_some_and(|s| s.skipped) {
        return Err(format!(
            "stage {n} has no changes, so it opens no pull request"
        ));
    }
    open::ready(run, n).map_err(|reason| format!("stage {n} is not ready: {reason}"))?;
    open::request_tier3(run, n, now, fx);
    Ok(format!(
        "stage {n}: tier 3 requested; its PR opens when tier 3 is green"
    ))
}

/// Decision 25's `run watch --on|--off`: off, no PR is polled; on, every open PR is due.
fn watch(state: &mut EngineState, run_id: &str, on: bool, now: u64) -> Result<String, String> {
    let run = state
        .runs
        .get_mut(run_id)
        .ok_or_else(|| format!("unknown run {run_id}"))?;
    if let Some(text) = refusal(run) {
        return Err(text);
    }
    run.delivery.watching = on;
    if !on {
        return Ok(format!(
            "stopped watching run {run_id}'s pull requests; anthrex run watch {run_id} --on resumes"
        ));
    }
    let open = run.delivery.stages.iter_mut().filter_map(|s| s.pr.as_mut());
    for pr in open.filter(|p| p.state == PrState::Open) {
        pr.next_poll_at = now;
    }
    Ok(format!("watching run {run_id}'s pull requests"))
}
