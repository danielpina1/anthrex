//! Milestone 9.3 decision 15 (KG §2.4 step 3): in a round, earlier rounds are done and
//! read-only. A batch may add tasks (and split children) only in the current round's
//! stages, and may not name a task of an earlier round in an edit of it. A new task may
//! still depend on one (decision 13). Engine-made fix tasks do not go through edits.
//! Pure (design decision 2).

use proto::{MessageTarget, PlanEdit};

use super::model::Run;
use super::orch::contract_rounds;
use super::plan::PlanError;

/// The first edit of `edits` that reaches into an earlier round of `run`: an
/// `add_task` (or a `split_task` child, or an `amend_task`'s new stage) in a stage below
/// the current round's first, or an edit naming a task of an earlier round. Every source is refused alike, with KG
/// §2.4's text naming the stage and its round.
pub(super) fn earlier_round(run: &Run, edits: &[PlanEdit]) -> Option<PlanError> {
    if run.round() < 2 {
        return None;
    }
    let first = run.current_round().map_or(1, |r| r.first_stage);
    let refuse = |id: Option<&str>, stage: u16| {
        let text = contract_rounds::earlier_round(stage, run.round_of_stage(stage));
        Some(PlanError::new(id, "", "44", text))
    };
    let earlier_task = |id: &str| {
        run.task(id)
            .filter(|t| t.round < run.round())
            .map(|t| t.stage())
    };
    edits.iter().find_map(|edit| {
        let named: Vec<&str> = match edit {
            PlanEdit::AddTask { task } if task.stage < first => {
                return refuse(Some(&task.id), task.stage);
            }
            // Fix round 1 (I3): nor is a task moved into an earlier round's stage.
            PlanEdit::AmendTask {
                task_id,
                stage: Some(stage),
                ..
            } if *stage < first => return refuse(Some(task_id), *stage),
            PlanEdit::SplitTask { task_id, into } => {
                if let Some(child) = into.iter().find(|c| c.stage > 1 && c.stage < first) {
                    return refuse(Some(&child.id), child.stage);
                }
                vec![task_id]
            }
            PlanEdit::CancelTask { task_id }
            | PlanEdit::Refresh { task_id }
            | PlanEdit::AmendTask { task_id, .. }
            | PlanEdit::AddDep { task_id, .. }
            | PlanEdit::Answer { task_id, .. } => vec![task_id],
            PlanEdit::Message {
                to: MessageTarget::Tasks(ids),
                ..
            } => ids.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        };
        named
            .into_iter()
            .find_map(|id| earlier_task(id).and_then(|stage| refuse(Some(id), stage)))
    })
}
