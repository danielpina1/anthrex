//! Milestone 9.17's snapshot additions (ruling C1): decision 10's sticky attention line
//! for a `brief` note in a run nobody plans with, and decision 20's plan frames, the
//! run's and each epic's, published only at the plan gate or while a hold awaits.
//! Pure: it reads the run, no I/O.

use proto::{HoldKind, HoldState, PlanFrameInfo, RunState, TaskNoteKind};

use super::model::Run;
use super::orch::EpicRecord;
use super::orch::plan_frame::{frame_for, info};

/// Decision 20 and ruling C7: the current round's frame and a design run's approved
/// Non-goals, while the run awaits approval or its `promotion` hold awaits.
pub(super) fn plan_frame_info(run: &Run) -> Option<PlanFrameInfo> {
    let promotion = (run.orch.gate_holds.iter())
        .any(|h| h.state == HoldState::Awaiting && matches!(h.kind, HoldKind::Promotion));
    if run.state != RunState::AwaitingApproval && !promotion {
        return None;
    }
    let goals = run
        .orch
        .design
        .as_ref()
        .map(|d| d.non_goals_section.as_str());
    info(frame_for(run, run.round()), goals)
}

/// Decision 20: `epic`'s frame, while the run awaits approval or a hold of the epic,
/// or of one of its tasks, awaits.
pub(super) fn planner_frame(run: &Run, epic: &EpicRecord) -> Option<PlanFrameInfo> {
    let of_epic = |id: &String| {
        (run.tasks.iter()).any(|t| t.id() == id && t.spec.epic.as_deref() == Some(&*epic.epic))
    };
    let held = (run.orch.gate_holds.iter()).any(|h| {
        h.state == HoldState::Awaiting
            && (matches!(&h.kind, HoldKind::Epic { epic: e } if *e == epic.epic)
                || h.tasks.iter().any(of_epic))
    });
    if run.state != RunState::AwaitingApproval && !held {
        return None;
    }
    info(epic.frame.as_ref(), None)
}

/// Decision 10 (`BRIEF_ATTENTION`): for a non-terminal run with no orchestrator record,
/// one line per task with at least one `brief` note, `<t>: the reviewer says the brief
/// is wrong (<n> notes); read them in the task's notes`. It stays while the task has
/// the notes, so later notes never push it out as they do `noted_lines`.
pub(super) fn brief_attention(run: &Run) -> Vec<String> {
    if run.state.is_terminal() || run.orch.orchestrator.is_some() {
        return Vec::new();
    }
    (run.tasks.iter())
        .filter_map(|t| {
            let notes = t.orch.worker_notes.iter();
            let n = notes.filter(|n| n.kind == TaskNoteKind::Brief).count();
            (n > 0).then(|| {
                format!(
                    "{}: the reviewer says the brief is wrong ({n} notes); read them in the task's notes",
                    t.id()
                )
            })
        })
        .collect()
}

/// Decision 26: the tasks whose `replaces` names `id`, in plan order.
pub(super) fn replaced_by(run: &Run, id: &str) -> Vec<String> {
    (run.tasks.iter())
        .filter(|t| t.replaces.iter().any(|r| r == id))
        .map(|t| t.id().to_string())
        .collect()
}

#[cfg(test)]
#[path = "snapshot_plan_tests.rs"]
mod tests;
