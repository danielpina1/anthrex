//! Milestone 9.3 decision 32 (KG §6): a run of more than one round hangs each round's
//! stages, and the tasks of a round with no stage listed yet, under a separator row
//! `round <r> · <goal head>` (`NodeKey::Round`, folding like a stage). Rows of an
//! earlier round than the run's current one are drawn muted, as is an idle
//! orchestrator's row in the project tree. Pure.

use super::run_rows::{Node, node};
use super::{NodeKey, RowKind};
use proto::{RoundInfo, RunInfo};
use std::collections::HashSet;

/// The root's children of a staged run: `children` (its scouts and the entries no stage
/// took) then `stages`, as before 9.3 for a one-round run. With several rounds, every
/// stage and every stageless entry goes under its round's separator, in round order,
/// after the scouts; a node whose round the snapshot does not list stays at the root.
pub(super) fn group<'a>(
    run: &'a RunInfo,
    mut children: Vec<Node<'a>>,
    stages: Vec<Node<'a>>,
) -> Vec<Node<'a>> {
    if run.rounds.len() < 2 {
        children.extend(stages);
        return children;
    }
    let mut seen = HashSet::new();
    let mut rounds: Vec<Node<'a>> = (run.rounds.iter())
        .filter(|round| seen.insert(round.n))
        .map(|round| {
            let key = NodeKey::Round {
                run: run.run_id.clone(),
                n: round.n,
            };
            node(key, RowKind::Round { run, round })
        })
        .collect();
    let mut root = Vec::new();
    for child in children.into_iter().chain(stages) {
        let parent = node_round(run, &child).and_then(|n| {
            (rounds.iter_mut())
                .find(|r| matches!(r.row.kind, RowKind::Round { round, .. } if round.n == n))
        });
        match parent {
            Some(parent) => parent.children.push(child),
            None => root.push(child),
        }
    }
    root.extend(rounds);
    root
}

/// The round a root child belongs to: a stage's, a task's, a planner's first task's;
/// `None` for a scout, which belongs to the run.
fn node_round(run: &RunInfo, child: &Node<'_>) -> Option<u32> {
    match &child.row.kind {
        RowKind::Stage { stage, .. } => Some(stage.round),
        RowKind::Task { task, .. } => Some(task.round),
        RowKind::Planner { .. } => {
            Some(
                child
                    .children
                    .first()
                    .map_or(run.round, |first| match &first.row.kind {
                        RowKind::Task { task, .. } => task.round,
                        _ => run.round,
                    }),
            )
        }
        _ => None,
    }
}

/// The separator's text, before it is cleaned and cut: `round <r> · <goal head>`.
pub fn round_text(round: &RoundInfo) -> String {
    format!("round {} · {}", round.n, round.goal_head)
}

/// Whether a run-view row belongs to a round before the run's current one (a run of
/// one round has none).
pub fn earlier_round(kind: &RowKind<'_>) -> bool {
    let (run, n) = match kind {
        RowKind::Round { run, round } => (*run, round.n),
        RowKind::Stage { run, stage } => (*run, stage.round),
        RowKind::Task { run, task } | RowKind::AgentRound { run, task, .. } => (*run, task.round),
        RowKind::Planner { run, planner } => {
            let first = (run.tasks.iter()).find(|t| t.epic.as_deref() == Some(&planner.epic));
            (*run, first.map_or(run.round, |task| task.round))
        }
        _ => return false,
    };
    run.rounds.len() > 1 && n < run.round
}

/// Whether a row is drawn muted whole (decision 32): an earlier round's, or an idle
/// orchestrator's.
pub fn muted_row(kind: &RowKind<'_>) -> bool {
    earlier_round(kind) || matches!(kind, RowKind::IdleOrchestrator { .. })
}

#[cfg(test)]
#[path = "round_rows_tests.rs"]
mod tests;
