//! Milestone 9.17 decisions 13 to 18 (pre-flight rulings C5, C6): the plan frame at
//! submission. An orchestrator's submit that acts and goes 9.5's way, and every
//! `submit_epic`, carries `decided` and `out_of_scope`; the engine refuses one that
//! lacks either key and stores the frame per round (`RunOrch.plan_frames`) and per epic
//! (`EpicRecord.frame`). A design-flow submit, a user's submit and a plan file never
//! need one (decisions 16, 17). Pure (design decision 2).

use proto::{DecidedItem, RunState};

use super::{design, gate_holds};
use crate::run::model::Run;
use crate::run::orch::plan_frame::{PlanFrame, frame_for};

/// Decision 14: an orchestrator's submit without the frame.
pub(crate) const PLAN_FRAME_MISSING: &str = "submit needs decided (each question the goal left open, with the answer you chose) and out_of_scope (what this run will not do); send both with submit, as empty lists if there are none";
/// Decision 14: a sub-planner's `submit_epic` without the frame.
pub(crate) const EPIC_FRAME_MISSING: &str = "submit_epic needs decided (each question your epic's brief left open, with the answer you chose) and out_of_scope (what your epic will not do); send both, as empty lists if there are none";
/// Decision 14: either key on an `edit_plan` without `submit: true`.
pub(crate) const FRAME_NEEDS_SUBMIT: &str =
    "decided and out_of_scope come only with submit set to true";

/// The two keys of one call, each `None` when absent.
pub(super) type Frame = (Option<Vec<DecidedItem>>, Option<Vec<String>>);

/// Whether either key is present.
pub(super) fn any(frame: &Frame) -> bool {
    frame.0.is_some() || frame.1.is_some()
}

/// Decision 14 and ruling C6, for an orchestrator's `edit_plan`: either key without
/// `submit` is refused; a submit carries both keys or neither; and a submit that acts
/// (in `planning`, or on a promoted run whose promotion is open) and goes 9.5's way
/// needs both, unless its round already has a frame, which a submit without keys keeps.
/// A submit at the gate, an ignored one, or a design-flow one needs none.
pub(super) fn check_edit_plan(run: &Run, submit: bool, frame: &Frame) -> Result<(), &'static str> {
    if !submit {
        return if any(frame) {
            Err(FRAME_NEEDS_SUBMIT)
        } else {
            Ok(())
        };
    }
    match frame {
        (Some(_), Some(_)) => Ok(()),
        (Some(_), None) | (None, Some(_)) => Err(PLAN_FRAME_MISSING),
        (None, None) => {
            let needs = acts(run)
                && design::plan::submits_as_9_5(run)
                && frame_for(run, run.round()).is_none();
            if needs {
                Err(PLAN_FRAME_MISSING)
            } else {
                Ok(())
            }
        }
    }
}

/// `submit_plan`'s two acting arms (`orch_submit.rs`): a planning run, and a promoted
/// running run whose promotion is open.
fn acts(run: &Run) -> bool {
    match run.state {
        RunState::Planning => true,
        RunState::Running => run.orch.orchestrator.is_some() && gate_holds::promotion_open(run),
        _ => false,
    }
}

/// Decision 18: an accepted submit carrying both keys replaces its round's frame; one
/// without them keeps it, and so does one that repeats it (a resubmit at the gate
/// changes nothing, not even the frame's time).
pub(super) fn store_run(run: &mut Run, frame: Frame, now: u64) {
    let round = run.round();
    let Some(new) = cleaned(round, frame, now) else {
        return;
    };
    let same = |f: &PlanFrame| f.decided == new.decided && f.out_of_scope == new.out_of_scope;
    if frame_for(run, round).is_some_and(same) {
        return;
    }
    let frames = &mut run.orch.plan_frames;
    frames.retain(|f| f.round != round);
    frames.push(new);
    frames.sort_by_key(|f| f.round);
}

/// Decision 14 for `submit_epic`: both keys, or [`EPIC_FRAME_MISSING`].
pub(super) fn check_epic(frame: &Frame) -> Result<(), &'static str> {
    match frame {
        (Some(_), Some(_)) => Ok(()),
        _ => Err(EPIC_FRAME_MISSING),
    }
}

/// Decision 18: each accepted `submit_epic` (a re-plan's included) replaces epic `k`'s
/// frame.
pub(super) fn store_epic(run: &mut Run, k: usize, frame: Frame, now: u64) {
    let round = run.round();
    if let Some(new) = cleaned(round, frame, now) {
        run.orch.epics[k].frame = Some(new);
    }
}

/// The frame of `round`, every text on one line, its control characters dropped;
/// `None` unless both keys were sent.
fn cleaned(round: u32, (decided, out_of_scope): Frame, at: u64) -> Option<PlanFrame> {
    let (decided, out_of_scope) = (decided?, out_of_scope?);
    let decided = decided
        .into_iter()
        .map(|d| DecidedItem {
            question: one_line(&d.question),
            answer: one_line(&d.answer),
        })
        .collect();
    let out_of_scope = out_of_scope.iter().map(|t| one_line(t)).collect();
    Some(PlanFrame {
        round,
        decided,
        out_of_scope,
        at,
    })
}

/// A line break or tab becomes a space, any other control character is dropped, and
/// the ends are trimmed.
fn one_line(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .collect();
    crate::run::messages::one_line(&kept).trim().to_string()
}
