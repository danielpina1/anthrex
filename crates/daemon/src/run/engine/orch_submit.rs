//! Decision 27's `submit`, the engine side of `edit_plan { submit: true }` and of the
//! user's `run edit --submit`: when a plan may be submitted, and what a submit does to
//! the run. Moved out of `orch.rs` (milestone 9.6 task M9.6.7, move only). Pure (design
//! decision 2).

use proto::RunState;

use super::super::requests::log;
use super::super::{gate_holds, goal_rounds_end};
use crate::run::model::Run;

/// Decision 27's `submit`. In `planning` the plan must hold an unfinished task and no
/// sub-planner may be live; the run then waits at the gate, or runs at once when it was
/// started with `--yes`. In `awaiting_approval` nothing changes. On a promoted running
/// run the plan is submitted and hold `promotion` awaits the user, created empty when
/// nothing was added. A run being discarded or accepted takes no submit. Otherwise it is
/// ignored. `who` submits: the orchestrator, or the user's `run edit` (decision 13,
/// `requests::edit`, which admits `planning` only).
pub(in crate::run::engine) fn submit_plan(
    run: &mut Run,
    who: &str,
    now: u64,
) -> Result<(), String> {
    submit_refusal(run).map_or(Ok(()), Err)?;
    match run.state {
        RunState::Planning => {
            set_submitted(run);
            // Milestone 9.3 decision 12: a round the orchestrator started never skips it.
            if goal_rounds_end::skips_gate(run) {
                run.state = RunState::Running;
                goal_rounds_end::approved(run, "--yes", now);
                log(
                    run,
                    now,
                    format!("{who} submitted the plan; approved by --yes"),
                );
            } else {
                run.state = RunState::AwaitingApproval;
                log(
                    run,
                    now,
                    format!("{who} submitted the plan; awaiting approval"),
                );
            }
        }
        RunState::Running if run.orch.orchestrator.is_some() => {
            set_submitted(run);
            if gate_holds::submit_promotion(run, now) {
                log(run, now, "the orchestrator submitted its additions");
            }
            // The epic rounds its own additions opened, whose epics no sub-planner is
            // planning (M9.7 second review, items 8 and 10).
            gate_holds::submit_epic_rounds(run, now);
        }
        _ => {}
    }
    Ok(())
}

/// Why [`submit_plan`] refuses `run` now, if it does (milestone 9.0.6 decision 42's pure
/// twin, which `actions::check` asks).
pub(in crate::run::engine) fn submit_refusal(run: &Run) -> Option<String> {
    // A discard or accept in flight takes no submit (M9.7 second review, ruling 4).
    if let Some(how) = super::super::dispatch::finishing_as(run) {
        return Some(format!("run {} is being {how}", run.id));
    }
    match run.state {
        RunState::Planning if run.tasks.iter().all(|t| t.state.is_finished()) => {
            Some("the plan has no tasks yet; add tasks before submitting".into())
        }
        RunState::Planning => planners_finished(run).err(),
        // While the promotion window is open, its submit waits for the sub-planners
        // too, so the user sees the promotion's whole plan (M9.7 second review, rulings
        // 8 and 9).
        RunState::Running if run.orch.orchestrator.is_some() && gate_holds::promotion_open(run) => {
            planners_finished(run).err()
        }
        _ => None,
    }
}

/// Decision 27: no sub-planner is queued or planning.
pub(in crate::run::engine) fn planners_finished(run: &Run) -> Result<(), String> {
    match run.orch.epics.iter().find(|e| e.phase.is_live()) {
        Some(e) => Err(format!(
            "sub-planner {} is still planning; submit when every sub-planner has finished",
            e.epic
        )),
        None => Ok(()),
    }
}

fn set_submitted(run: &mut Run) {
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.plan_submitted = true;
    }
}
