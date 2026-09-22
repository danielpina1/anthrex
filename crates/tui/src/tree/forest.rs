//! Turning a window's flat `subagents` list into the forest `tree/rows.rs` walks:
//! cycle/orphan handling, sorting siblings, and hiding old finished sub-agents while
//! reattaching their children to the nearest still-shown ancestor. Split out of
//! `tree.rs` (task M6.10's file-size finding B, `AGENTS.md` hard rule 8) — the same
//! `mod rows;` split `tree.rs` already had for turning that forest into rows.

use proto::{SubagentInfo, SubagentState};
use std::collections::{HashMap, HashSet};

pub struct SubagentNode<'a> {
    pub info: &'a SubagentInfo,
    pub children: Vec<SubagentNode<'a>>,
}

pub fn subagent_forest(
    subagents: &[SubagentInfo],
    keep_finished_secs: u64,
) -> Vec<SubagentNode<'_>> {
    let index_by_id: HashMap<&str, usize> = subagents
        .iter()
        .enumerate()
        .map(|(index, info)| (info.id.as_str(), index))
        .collect();
    let parent_indices: Vec<_> = subagents
        .iter()
        .map(|info| {
            info.parent_id
                .as_deref()
                .and_then(|parent_id| index_by_id.get(parent_id).copied())
        })
        .collect();

    let mut cycle_members = HashSet::new();
    for start in 0..subagents.len() {
        let mut path = Vec::new();
        let mut path_positions = HashMap::new();
        let mut current = Some(start);
        while let Some(index) = current {
            if let Some(cycle_start) = path_positions.get(&index).copied() {
                cycle_members.extend(path[cycle_start..].iter().copied());
                break;
            }
            path_positions.insert(index, path.len());
            path.push(index);
            current = parent_indices[index];
        }
    }

    let mut roots = Vec::new();
    let mut children = vec![Vec::new(); subagents.len()];
    for (index, parent) in parent_indices.into_iter().enumerate() {
        if cycle_members.contains(&index) {
            roots.push(index);
        } else if let Some(parent) = parent {
            children[parent].push(index);
        } else {
            roots.push(index);
        }
    }
    sort_subagents(&mut roots, subagents);
    for siblings in &mut children {
        sort_subagents(siblings, subagents);
    }
    build_visible_forest(&roots, subagents, &children, keep_finished_secs)
}

/// Task M6.9: a `Done` or `Failed` sub-agent whose `ended_secs` is older than
/// `keep_finished_secs` gets no row of its own.
fn is_hidden_finished(info: &SubagentInfo, keep_finished_secs: u64) -> bool {
    info.state != SubagentState::Running
        && info
            .ended_secs
            .is_some_and(|secs| secs > keep_finished_secs)
}

/// Builds the forest for one level of `indices` (siblings, by index into `subagents`),
/// recursing into `children` first so a hidden node's own descendants are already
/// resolved before the decision to hide it is made.
///
/// A hidden node's resolved children are spliced into the list at the position the
/// hidden node itself held — the same reattach-to-the-nearest-shown-ancestor rule
/// `subagent_forest`'s cycle/orphan handling above already gives a sub-agent whose
/// `parent_id` cannot be found (it becomes a root instead of vanishing); here the
/// ancestor being skipped is known and hidden by age rather than missing, but a hidden
/// node's children must survive it exactly the same way.
fn build_visible_forest<'a>(
    indices: &[usize],
    subagents: &'a [SubagentInfo],
    children: &[Vec<usize>],
    keep_finished_secs: u64,
) -> Vec<SubagentNode<'a>> {
    let mut result = Vec::new();
    for &index in indices {
        let kids = build_visible_forest(&children[index], subagents, children, keep_finished_secs);
        if is_hidden_finished(&subagents[index], keep_finished_secs) {
            result.extend(kids);
        } else {
            result.push(SubagentNode {
                info: &subagents[index],
                children: kids,
            });
        }
    }
    result
}

fn sort_subagents(indices: &mut [usize], subagents: &[SubagentInfo]) {
    indices.sort_by(|left, right| {
        subagents[*right]
            .started_secs
            .cmp(&subagents[*left].started_secs)
            .then_with(|| subagents[*left].id.cmp(&subagents[*right].id))
    });
}
