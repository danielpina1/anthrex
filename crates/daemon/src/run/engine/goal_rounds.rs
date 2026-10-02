//! Milestone 9.3's rounds, engine side (KG §2): round 1 for every run ([`ensure_first`],
//! task 3), when a run is settled ([`settled`], decision 9), starting a round
//! ([`iterate`], decision 10), widening a `Single` run for it ([`widen`], decision 13),
//! a dependency on an earlier round ([`dep_met`]) and the `round` history line of a
//! round that ends ([`end_round`], decision 17). Pure (design decision 2).

use proto::{
    GOAL_MAX_CHARS, HISTORY_VERSION, HistoryLine, PrState, RoundLine, RoundOrigin, RoundOutcome,
    RunState, TaskState, safe_text,
};

use super::actions::rules;
use super::requests::log;
use super::{Effect, EngineState, ReplyId, batch, delivery, history, orch, orch_window};
use crate::run::model::{Round, Run, StageLayout, Task};
use crate::run::orch::contract_rounds::{
    ITERATE_ALONE, REQUEST_TOO_LONG, round_started, round_wake,
};
use crate::run::snapshot_stages::stage_count;
use crate::run::triage::blank_goal;

/// Decision 18: a run with no round record (one started now, or one from before
/// rounds) gets round 1 ([`Round::first`]).
pub(super) fn ensure_first(run: &mut Run) {
    if run.rounds.is_empty() {
        run.rounds.push(Round::first(run));
    }
}

/// Decision 9, steps 6 and 7: a round may start. Local mode: the run is complete. `pr`
/// mode: complete and not cancelled (every PR landed), or running with every stage
/// delivered and nothing in progress.
pub(crate) fn settled(run: &Run) -> bool {
    match run.state {
        RunState::Complete => !delivery::pr(run) || !run.cancelled,
        RunState::Running => delivery::pr(run) && delivered_idle(run),
        _ => false,
    }
}

/// KG §2.3's "delivering with nothing in progress": every stage has its PR (or was
/// skipped), no task is unfinished, nothing waits to merge, propagate or absorb the
/// base, and no stage's push is held.
fn delivered_idle(run: &Run) -> bool {
    run.delivery.delivering(stage_count(run))
        && run.tasks.iter().all(|t| t.state.is_finished())
        && run.merge_queue.is_empty()
        && run.propagate_due.is_empty()
        && run.delivery.base_sync_due.is_empty()
        && run.delivery.stages.iter().all(|s| s.held.is_none())
}

/// `run iterate` (`EventKind::Iterate`): decision 10 with origin `user`.
pub(super) fn request(
    state: &mut EngineState,
    reply: ReplyId,
    (run_id, goal): (&str, &str),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let result = match state.runs.get_mut(run_id) {
        Some(run) => iterate(run, goal, RoundOrigin::User, now, fx),
        None => Err(format!("unknown run {run_id}")),
    };
    fx.push(Effect::Reply { reply, result });
}

/// Decision 10: the request is checked (blank, too long, then decision 9), the current
/// round ends `completed` if it has not ended, and the next round is recorded with the
/// request cleaned (`safe_text::multi_line`), its first stage after every stage so far
/// and D5's counters. A `Single` run is widened, the run goes back to `planning` (from
/// `complete`, or from `running` in `pr` mode) with `cancelled` and `finish_edit`
/// cleared, and the round wake waits on the run until its orchestrator is woken with
/// it; a dormant orchestrator is relaunched for it. The reply names the round.
pub(super) fn iterate(
    run: &mut Run,
    goal: &str,
    origin: RoundOrigin,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<String, String> {
    if let Some(text) = blank_goal(goal) {
        return Err(text);
    }
    if goal.chars().count() > GOAL_MAX_CHARS {
        return Err(REQUEST_TOO_LONG.to_string());
    }
    if let Some(text) = rules::iterate(run) {
        return Err(text);
    }
    ensure_first(run);
    end_round(run, RoundOutcome::Completed, now, fx);
    let n = run.round() + 1;
    let first_stage = stage_count(run).saturating_add(1);
    let request = safe_text::multi_line(goal);
    let round = Round {
        n,
        goal: request.clone(),
        origin,
        started_at: now,
        ended_at: None,
        outcome: None,
        summary: None,
        first_stage,
        windows_before: run.windows_created,
        scouts_before: u32::try_from(run.orch.run_scouts.len()).unwrap_or(u32::MAX),
    };
    run.rounds.push(round);
    // KG §2.5 (task 5): above landed PRs only, the round's first stage absorbs the
    // remote base first, so a fetch of it is due before that stage is created.
    if delivery::pr(run) && landed_below(run, first_stage) {
        run.delivery.base_fetch_due = true;
    }
    widen(run, now);
    run.state = RunState::Planning;
    run.cancelled = false;
    run.finish_edit = false;
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.plan_submitted = false;
    }
    let h4 = run.short().to_string();
    run.orch.request_wake = Some(round_wake(&h4, n, first_stage - 1, &request));
    let who = match origin {
        RoundOrigin::User => "the user",
        RoundOrigin::Orchestrator => "the orchestrator",
    };
    // The request is user text: it reaches the orchestrator only fenced, never a log
    // line (review focus 3).
    log(run, now, format!("round {n} started by {who}"));
    orch_window::relaunch(run, now, fx);
    Ok(round_started(&h4, n))
}

/// Milestone 9.3 decision 30: `edit_plan`'s `iterate`, alone in its call, is decision
/// 10's path with origin `orchestrator`, logged as an `iterate` edit and answered as an
/// accepted `edit_plan`; a refusal is logged too (decision 40).
pub(super) fn edit(
    run: &mut Run,
    reply: ReplyId,
    (goal, alone): (&str, bool),
    (now, base): (u64, &mut Option<Run>),
    fx: &mut Vec<Effect>,
) {
    let source = crate::run::orch::EditSource::Orchestrator;
    let edit = [proto::PlanEdit::Iterate {
        goal: goal.to_string(),
    }];
    let started = match alone {
        false => Err(ITERATE_ALONE.to_string()),
        true => iterate(run, goal, RoundOrigin::Orchestrator, now, fx),
    };
    if let Err(text) = started {
        batch::record_rejected(run, &edit, &source, text.clone(), now);
        return orch::refuse(fx, reply, text);
    }
    let outcome = crate::run::edit_log::EditOutcome::Accepted {
        recipients: Vec::new(),
    };
    crate::run::edit_log::record(run, &edit, now, &source, outcome);
    orch::accepted(run, reply, (Vec::new(), None, None), (now, base), fx)
}

/// Decision 13: a `Single` run is `Multi` from its second round on, with no stage
/// recorded, so the next running pass creates `stage-1` from `run_head` and then the
/// round's first stage from it (milestone 9.1 decision 48). The delivery records live
/// in `run.delivery.stages` and are kept.
pub(super) fn widen(run: &mut Run, now: u64) {
    if run.stage_layout != StageLayout::Single {
        return;
    }
    run.stage_layout = StageLayout::Multi;
    run.stages.clear();
    let text = format!(
        "round {} widens the run to stages: branches anthrex/{}/stage-<n>",
        run.round(),
        run.id
    );
    log(run, now, text);
}

/// KG §2.5: every stage below `n` (at least one) has landed ([`stage_landed`]); no
/// open PR is left for stage `n`'s to stack on.
pub(super) fn landed_below(run: &Run, n: u16) -> bool {
    n > 1 && (1..n).all(|m| stage_landed(run, m))
}

/// Stage `m` has landed: skipped, or its PR merged or closed and that landing processed
/// (`land::processed`, its `stage` line out; task 5 fix round 1, I1), or, with no PR,
/// it merged nothing and nothing of it is left to run (it is skipped once delivery
/// looks at it, as a rejected round's stage is).
pub(super) fn stage_landed(run: &Run, m: u16) -> bool {
    let d = &run.delivery;
    let empty = || {
        let mut of_stage = run.tasks.iter().filter(|t| t.stage() == m);
        of_stage.all(|t| t.state.is_finished() && t.state != TaskState::Merged)
    };
    match d.pr(m) {
        _ if d.stage(m).is_some_and(|s| s.skipped) => true,
        Some(pr) => pr.state != PrState::Open && delivery::land::processed(run, m),
        None => empty(),
    }
}

/// Decision 13, `pr` mode: stage `next`, the current round's first, waits while the
/// base fetch its round made due ([`iterate`]) is due or in flight, when every stage
/// below it has landed ([`landed_below`]).
pub(super) fn awaits_base(run: &Run, next: u16) -> bool {
    let first = run
        .current_round()
        .is_some_and(|r| r.n > 1 && r.first_stage == next);
    let fetch = run.delivery.base_fetch_due || delivery::sync::fetching(run);
    delivery::pr(run) && first && fetch && landed_below(run, next)
}

/// Decision 13: a dependency of `task` on a task of an earlier round is met, whatever
/// that task's state (its work is in every stage the round creates).
pub(crate) fn dep_met(_run: &Run, task: &Task, dep: &Task) -> bool {
    dep.round < task.round
}

/// Decision 17: the current round ends at `now`, once, with `outcome` unless a cancel
/// already set its own (decision 16); with history on, its `round` line
/// (`<run>/round/<n>`) counts the round's tasks, those merged, their tool calls and the
/// round's whole minutes.
pub(super) fn end_round(run: &mut Run, outcome: RoundOutcome, now: u64, fx: &mut Vec<Effect>) {
    let Some(round) = run.rounds.last_mut().filter(|r| r.ended_at.is_none()) else {
        return;
    };
    let outcome = *round.outcome.get_or_insert(outcome);
    round.ended_at = Some(now);
    let (n, origin, started_at) = (round.n, round.origin, round.started_at);
    if !crate::run::history::enabled(run) {
        return;
    }
    let of_round: Vec<&Task> = run.tasks.iter().filter(|t| t.round == n).collect();
    let count = |k: usize| u32::try_from(k).unwrap_or(u32::MAX);
    let merged = of_round
        .iter()
        .filter(|t| t.state == TaskState::Merged)
        .count();
    let calls = of_round
        .iter()
        .fold(0u32, |sum, t| sum.saturating_add(t.spent_total.tool_calls));
    let line = HistoryLine::Round(RoundLine {
        v: HISTORY_VERSION,
        record_id: format!("{}/round/{n}", run.id),
        at: now,
        run_id: run.id.clone(),
        round: n,
        origin,
        outcome,
        tasks: count(of_round.len()),
        merged: count(merged),
        calls,
        minutes: now.saturating_sub(started_at) / 60,
    });
    history::append(run, None, line, fx);
}
