//! Milestone 9.3's rounds, engine side II (KG §2.4, §2.6): a round's plan gate
//! ([`skips_gate`], [`approved`], decision 12), its reject ([`reject_round`], decision
//! 12) and cancel ([`cancel_round`], decision 16), and its end ([`pass`], decision 17),
//! whose `round` history line `goal_rounds::end_round` writes. Pure (design decision 2).

use proto::{RoundOrigin, RoundOutcome, RunState, TaskState};

use super::complete::{cancel_task, deferred_note};
use super::goal_rounds::{end_round, landed_below};
use super::requests::log;
use super::{Effect, delivery, merge, planners, stages, wake};
use crate::run::model::Run;
use crate::run::orch::contract_rounds::{round_cancelled, round_done, round_rejected};
use crate::run::snapshot_stages::stage_count;

/// Decision 12's reason for the sub-planners and run scouts a rejected round halts.
pub const ROUND_REJECTED: &str = "the round was rejected";
/// Decision 16's reason for the sub-planners and run scouts a cancelled round halts.
pub const ROUND_CANCELLED: &str = "the round was cancelled";

/// Decision 12 (KG §2.4 step 4): a submitted plan skips the gate, and a submitted hold
/// is approved at once (`gate_holds::submitted`, fix round 1), only when the run was
/// started with approve at once and the current round is the user's; a round the
/// orchestrator started always waits for the user.
pub(super) fn skips_gate(run: &Run) -> bool {
    // Milestone 9.6 decision 6: `--yes` skips none of a design run's gates.
    run.orch.yes
        && run.design_mode != proto::DesignMode::Full
        && run
            .current_round()
            .is_none_or(|r| r.origin == RoundOrigin::User)
}

/// Fix round 1 (I1): the current round is a later one that has not ended, so a
/// reject or cancel is that round's; once it has ended (rejected, or a `pr` run back to
/// delivering), they are the whole run's.
pub(super) fn open_round(run: &Run) -> bool {
    run.current_round()
        .is_some_and(|r| r.n > 1 && r.ended_at.is_none())
}

/// The round a `run cancel` is ending (decision 16): the current round, open and
/// `cancelled`, of a run that was not cancelled as a whole. `finish_edit` is then that
/// round's, and `complete::finish_pass` cancels its tasks only (the final fix wave,
/// review A, I1).
pub(super) fn cancelling_round(run: &Run) -> Option<u32> {
    let round = run.current_round()?;
    let cancelled = round.outcome == Some(RoundOutcome::Cancelled);
    let ours = run.finish_edit && run.round_finish;
    (!run.cancelled && ours && open_round(run) && cancelled).then_some(round.n)
}

/// The plan is approved, `by` the user or `--yes`: round 1's approval is the run's and
/// fixes its layout (milestone 9.1 decision 46); a later round's leaves both as round 1
/// set them (decision 12), and its own time is the round's (milestone 9.5 ruling RE-1).
/// A decider queued at the gate waits from now (task 12 review m7).
pub(super) fn approved(run: &mut Run, by: &str, now: u64) {
    if run.round() <= 1 {
        run.approved_by = Some(by.to_string());
        run.approved_at = Some(now);
        stages::fix_layout(run, now);
    }
    // Milestone 9.5 ruling RE-1 and decision 15: a later round's estimate runs from its
    // approval, and every round's counts the time paused since.
    let (later, paused) = (run.round() > 1, run.paused_secs);
    if let Some(round) = run.rounds.last_mut() {
        round.approved_at = later.then_some(now);
        round.paused_before = paused;
    }
    for q in &mut run.decider_queue {
        q.queued_at = now;
    }
}

/// Decision 12: `run reject` on round 2 or later while it is open ([`open_round`]).
/// Every task of the round is cancelled, the round ends `rejected` with its history
/// line, its request and its sub-planners and run scouts are dropped, and the run goes
/// back to `complete`, or in `pr` mode to `running` unless every earlier stage has
/// landed and been processed (task 5 and its fix round 1, I1: that run was complete,
/// and is again), with no summary wake; a `pr` round's stages are skipped. Earlier
/// rounds are untouched; nothing is discarded.
pub(super) fn reject_round(run: &mut Run, now: u64, fx: &mut Vec<Effect>) -> String {
    let n = run.round();
    let why = format!("run reject (round {n})");
    for i in 0..run.tasks.len() {
        if run.tasks[i].round == n && !run.tasks[i].state.is_finished() {
            cancel_task(run, i, &why, now, fx);
        }
    }
    end_round(run, RoundOutcome::Rejected, now, fx);
    wake::clear_request(run, n);
    planners::halt_all(run, ROUND_REJECTED, now, fx);
    // Milestone 9.6 decision 29: its design agents too, and the approval before it.
    super::design_round::rejected(run, ROUND_REJECTED, fx);
    // The earlier rounds' plan stays the submitted one (`rules::promoted_unsubmitted`).
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.plan_submitted = true;
    }
    run.paused_from = None;
    // Task 5 fix round 1 (I1): back to `complete` only when the run was, every earlier
    // stage's landing processed; its round's fetch is dropped then. Otherwise
    // delivery resumes, and processes a landing seen while the round was planned.
    let first = run.current_round().map_or(1, |r| r.first_stage);
    let landed = delivery::pr(run) && landed_below(run, first);
    run.state = if delivery::pr(run) && !landed {
        RunState::Running
    } else {
        RunState::Complete
    };
    if landed {
        run.delivery.base_fetch_due = false;
    }
    // The round's stages merged nothing: they are skipped (m1), so no base sync targets
    // them and a later round's first stage sits above landed stages only.
    if delivery::pr(run) {
        skip_stages(run, first, now);
    }
    log(run, now, format!("round {n} rejected by the user"));
    let text = format!("the user rejected round {n}; the earlier rounds are unchanged");
    wake::note(run, text);
    round_rejected(run.short(), n)
}

/// Decision 16: `run cancel` on round 2 or later while it is open ([`open_round`])
/// cancels that round only. Its unfinished tasks are cancelled as `complete::cancel`
/// cancels them, its request is dropped and the earlier rounds' plan stays submitted,
/// its sub-planners and run scouts are halted, and `finish_edit` keeps anything new
/// from starting while its sessions end; the round's outcome is `cancelled`, and
/// [`pass`] ends it once the run completes. `run.cancelled` stays false and the
/// delivery keeps watching, so earlier rounds' pull requests are untouched.
pub(super) fn cancel_round(run: &mut Run, now: u64, fx: &mut Vec<Effect>) -> String {
    let n = run.round();
    if run.state == RunState::Paused {
        run.state = RunState::Running;
        run.paused_from = None;
    }
    // T15-minors (M-4), as `complete::cancel`: nothing resumes the cancelled work.
    run.restored = None;
    let why = format!("run cancel (round {n})");
    let mut merging = Vec::new();
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        if task.round == n && !task.state.is_finished() && cancel_task(run, i, &why, now, fx) {
            merging.push(run.tasks[i].id().to_string());
        }
    }
    if let Some(round) = run.rounds.last_mut().filter(|r| r.ended_at.is_none()) {
        round.outcome = Some(RoundOutcome::Cancelled);
    }
    // W1 fix round 2: a finish the user gave first stays the run's.
    run.round_finish = run.round_finish || !run.finish_edit;
    run.finish_edit = true;
    // Fix round 1 (D13, m1): a request of this round not yet delivered never is, and
    // the earlier rounds' plan stays the submitted one, as a reject leaves it.
    wake::clear_request(run, n);
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.plan_submitted = true;
    }
    planners::halt_all(run, ROUND_CANCELLED, now, fx);
    if delivery::pr(run) {
        unused_stages(run, now);
    }
    log(run, now, format!("round {n} cancelled by the user"));
    let mut text = round_cancelled(run.short(), n);
    for id in merging {
        text.push_str(&deferred_note(&id));
    }
    text
}

/// Decision 17, every step for every run: a round after the first that has not ended
/// ends once the run is `complete` (M8a's completion, whose summary wake asks for the
/// round's summary), or, in `pr` mode, once it is delivered while the run keeps
/// delivering ([`delivered`]), with the note asking for its summary. It ends
/// `completed` unless a cancel made it `cancelled`, and `finish_edit`, when the cancel
/// set it (`round_finish`), is cleared. The round's `ended_at` is set in the step that asks for its summary,
/// so a summary written after that note is the round's (`orch::write_summary`). Round 1
/// ends when a second round starts (`goal_rounds::iterate`, decision 10). A run
/// discarded or failed first ends it `cancelled` (milestone 9.5 decision 34).
pub(super) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if !open_round(run) {
        return;
    }
    // Milestone 9.5 decision 34: a run that ended without completing ends its round
    // `cancelled`, before `history::pass` writes the run's own line.
    if matches!(run.state, RunState::Discarded | RunState::Failed) {
        let (n, state) = (run.round(), run.state.label());
        log(
            run,
            now,
            format!("round {n} ended cancelled: the run is {state}"),
        );
        return end_round(run, RoundOutcome::Cancelled, now, fx);
    }
    let pr_end = run.state == RunState::Running && delivered(run);
    if run.state != RunState::Complete && !pr_end {
        return;
    }
    end_round(run, RoundOutcome::Completed, now, fx);
    // W1 fix round 2: only the round cancel's finish ends with the round; the user's
    // run-wide `finish` stays.
    if run.round_finish {
        run.finish_edit = false;
    }
    run.round_finish = false;
    if pr_end {
        let n = run.round();
        let cancelled =
            run.current_round().and_then(|r| r.outcome) == Some(RoundOutcome::Cancelled);
        let text = match cancelled {
            true => format!("round {n} ended cancelled"),
            false => format!("round {n} is done"),
        };
        log(run, now, text);
        wake::note(run, round_done(n));
    }
}

/// Decision 17's `pr` end of the current round: a `pr` run every task of which is
/// finished, every stage of the round has its PR (in any state) or was skipped, no
/// merge, stage creation, propagate or base sync is queued, due or in flight, and no
/// stage is held (task 5 fix round 1, m5).
pub(super) fn delivered(run: &Run) -> bool {
    let Some(first) = run.current_round().map(|r| r.first_stage) else {
        return false;
    };
    let d = &run.delivery;
    let covered = |n: u16| d.stage(n).is_some_and(|s| s.skipped) || d.pr(n).is_some();
    delivery::pr(run)
        && run.tasks.iter().all(|t| t.state.is_finished())
        && (first..=stage_count(run)).all(covered)
        && run.merge_queue.is_empty()
        && run.propagate_due.is_empty()
        && d.base_sync_due.is_empty()
        && d.stages.iter().all(|s| s.held.is_none())
        && !merge::merging(run)
}

/// The final fix wave (review A, C1): once a `pr` round's unfinished tasks are
/// cancelled, each of its stages that has no PR and nothing merged or still merging
/// (a deferred merge keeps its stage) is skipped, as a reject skips them. Above landed
/// PRs with every stage of the round skipped, the round's base fetch is dropped: no
/// stage is left to absorb the base, and a sync due into a stage never created would
/// keep the round from ending.
fn unused_stages(run: &mut Run, now: u64) {
    let first = run.current_round().map_or(1, |r| r.first_stage);
    for n in first..=stage_count(run) {
        let d = &run.delivery;
        let used = (run.tasks.iter())
            .any(|t| t.stage() == n && (!t.state.is_finished() || t.state == TaskState::Merged));
        if used || d.pr(n).is_some() || d.stage(n).is_some_and(|s| s.skipped) {
            continue;
        }
        delivery::stage_mut(run, n).skipped = true;
        log(run, now, format!("stage {n}: skipped (no changes)"));
    }
    let skipped = |n: u16| run.delivery.stage(n).is_some_and(|s| s.skipped);
    if landed_below(run, first) && (first..=stage_count(run)).all(skipped) {
        run.delivery.base_fetch_due = false;
        let d = &mut run.delivery;
        d.base_sync_due.retain(|k, _| *k < first);
    }
}

/// Every stage from `first` without a PR is skipped (a rejected round's, whose tasks
/// all ended cancelled), logged as `open.rs` logs a skip.
fn skip_stages(run: &mut Run, first: u16, now: u64) {
    for n in first..=stage_count(run) {
        if run.delivery.pr(n).is_some() || run.delivery.stage(n).is_some_and(|s| s.skipped) {
            continue;
        }
        delivery::stage_mut(run, n).skipped = true;
        log(run, now, format!("stage {n}: skipped (no changes)"));
    }
}
