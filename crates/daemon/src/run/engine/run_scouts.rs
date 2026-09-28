//! Milestone 9 decision 20, engine side: run scouts. The orchestrator's `spawn_scout`
//! queues one; it takes a reader slot after the sub-planners (`planners::dispatch`), and
//! its session runs on M8b's `ScoutService` (`OpKind::StartScout`). Its report is
//! stored by the service; the engine records that it reported, and its usage. Pure
//! (design decision 2).

use super::orch::{refuse, settle};
use super::planners::wake_note;
use super::requests::log;
use super::{Effect, OpKind, OpResult, ReplyId, ScoutEnd, emit_op, next_op};
use crate::run::globs::validate_glob;
use crate::run::model::Run;
use crate::run::orch::launch::scout_spec;
use crate::run::orch::{RunScout, RunScoutState};

/// Run scouts holding reader slots: started and not ended.
pub(super) fn running(run: &Run) -> usize {
    run.orch
        .run_scouts
        .iter()
        .filter(|s| s.state == RunScoutState::Running)
        .count()
}

/// Decision 20's `spawn_scout { id, question, area, web }`: the full id is
/// `<h4>-<id>`, each at most once per run and at most `max_scouts` per run; the scout is
/// queued for a reader slot and the reply is at once, `queued` or `starting`.
pub(super) fn spawn(
    run: &mut Run,
    reply: ReplyId,
    (id, question, area, web): (&str, String, Vec<String>, bool),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let full = format!("{}-{id}", run.short());
    if run.orch.run_scouts.iter().any(|s| s.id == full) {
        let text = format!("scout {full} already exists in run {}", run.id);
        return refuse(fx, reply, text);
    }
    let n = run.orch.run_scouts.len();
    if n >= run.limits.orch.max_scouts as usize {
        let text = format!(
            "run {} already has {n} scouts, the most max_scouts allows",
            run.id
        );
        return refuse(fx, reply, text);
    }
    if let Some((glob, problem)) = area
        .iter()
        .find_map(|g| validate_glob(g).err().map(|p| (g, p)))
    {
        return refuse(
            fx,
            reply,
            format!("invalid arguments: area: {glob}: {problem}"),
        );
    }
    run.orch.run_scouts.push(RunScout {
        id: full.clone(),
        question,
        area,
        web,
        state: RunScoutState::Queued,
        queued_at: now,
        started_at: None,
        ended_at: None,
        window_id: None,
    });
    log(run, now, format!("scout {full} queued"));
    settle(run, now, fx);
    let state = match run.orch.run_scouts.iter().find(|s| s.id == full) {
        Some(s) if s.state == RunScoutState::Running => "starting",
        _ => "queued",
    };
    let text = serde_json::json!({"scout_id": full, "state": state}).to_string();
    fx.push(Effect::Reply {
        reply,
        result: Ok(text),
    });
}

/// Starts the oldest queued run scout; false when none is queued.
pub(super) fn start_next(run: &mut Run, now: u64, fx: &mut Vec<Effect>) -> bool {
    let Some(k) = run
        .orch
        .run_scouts
        .iter()
        .position(|s| s.state == RunScoutState::Queued)
    else {
        return false;
    };
    let spec = scout_spec(run, &run.orch.run_scouts[k], &run.root, &run.project);
    let scout = &mut run.orch.run_scouts[k];
    scout.state = RunScoutState::Running;
    scout.started_at = Some(now);
    let text = format!("scout {} starting", scout.id);
    log(run, now, text);
    let op = next_op(run);
    let kind = OpKind::StartScout {
        spec: Box::new(spec),
    };
    emit_op(run, op, None, kind, fx);
    true
}

/// `StartScout`'s result: the scout's window, or its failure.
pub(super) fn started(run: &mut Run, kind: &OpKind, result: OpResult, now: u64) {
    let OpKind::StartScout { spec } = kind else {
        return;
    };
    match result {
        OpResult::ScoutStarted { window_id } => {
            if let Some(s) = run.orch.run_scouts.iter_mut().find(|s| s.id == spec.id) {
                s.window_id = Some(window_id);
            }
        }
        OpResult::Failed { message } => {
            let reason = format!("the scout could not start: {message}");
            end(run, &spec.id, Err(reason), now);
        }
        _ => {}
    }
}

/// Decision 20: a run scout's session ended. A report is pushed to `Run.scout_reports`
/// (M8b decision 19's list, which the plan rules and the size check read); the usage
/// counts whatever the outcome.
pub(super) fn ended(
    run: &mut Run,
    scout_id: &str,
    outcome: ScoutEnd,
    usage: proto::TokenUsage,
    now: u64,
) {
    if !run.orch.run_scouts.iter().any(|s| s.id == scout_id) {
        return;
    }
    run.scout_usage += usage;
    let result = match outcome {
        ScoutEnd::Reported => Ok(()),
        ScoutEnd::Failed { reason } => Err(reason),
    };
    end(run, scout_id, result, now);
}

fn end(run: &mut Run, id: &str, result: Result<(), String>, now: u64) {
    let Some(scout) = run.orch.run_scouts.iter_mut().find(|s| s.id == id) else {
        return;
    };
    if !matches!(scout.state, RunScoutState::Queued | RunScoutState::Running) {
        return;
    }
    scout.ended_at = Some(now);
    let note = match result {
        Ok(()) => {
            scout.state = RunScoutState::Reported;
            if !run.scout_reports.iter().any(|r| r == id) {
                run.scout_reports.push(id.to_string());
            }
            format!("scout {id} reported")
        }
        Err(reason) => {
            let note = format!("scout {id} failed: {reason}");
            scout.state = RunScoutState::Failed { reason };
            note
        }
    };
    log(run, now, note.clone());
    wake_note(run, note);
}

/// Decision 20 after a daemon restart: a queued or running run scout is not resumed; it
/// fails with the documented reason, and nothing is killed.
pub(super) fn restore(run: &mut Run, now: u64) {
    let live: Vec<String> = run
        .orch
        .run_scouts
        .iter()
        .filter(|s| matches!(s.state, RunScoutState::Queued | RunScoutState::Running))
        .map(|s| s.id.clone())
        .collect();
    for id in live {
        let reason = "the daemon restarted during this scout".to_string();
        end(run, &id, Err(reason), now);
    }
}
