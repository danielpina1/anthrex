//! Turning the visible children of a node into rows, with their box-drawing guides.
//!
//! Every level is exactly two columns wide and depth is unbounded (decisions 23
//! and 25). The bits that decide a guide column come from the *visible* tree —
//! after collapse and after filtering — so hiding the last child of a group
//! turns the `├─` above it into `└─` (decision 24).

use super::{
    NodeKey, ProjectChild, Row, RowKind, SubagentNode, WindowInfo, matches_filter,
    matches_subagent, subagent_branch_matches, subagent_forest,
};

/// The column below an ancestor that still has a later visible sibling.
const STEM: &str = "│ ";
/// The column below an ancestor that was the last of its visible siblings.
const GAP: &str = "  ";
/// The connector of a row that has a later visible sibling.
const TEE: &str = "├─";
/// The connector of the last of its visible siblings.
const ELBOW: &str = "└─";

/// The guide prefix drawn before a row's own marker.
///
/// `ancestors` holds one "has a later visible sibling" bit per ancestor level
/// below the root, outermost first, and `has_later_sibling` is the row's own
/// bit. Root rows — projects — are not drawn by this routine: they carry no
/// guides at all.
pub(super) fn guide_prefix(ancestors: &[bool], has_later_sibling: bool) -> String {
    let mut guides = String::with_capacity((ancestors.len() + 1) * STEM.len());
    for ancestor_has_later_sibling in ancestors.iter().copied() {
        guides.push_str(if ancestor_has_later_sibling {
            STEM
        } else {
            GAP
        });
    }
    guides.push_str(if has_later_sibling { TEE } else { ELBOW });
    guides
}

/// A window that survives the filter, with the work its rows need.
pub(super) struct VisibleWindow<'a> {
    pub window: &'a WindowInfo,
    pub forest: Vec<SubagentNode<'a>>,
    /// True when every sub-agent below the window is shown, because the window
    /// or its project matched the filter itself.
    pub show_all: bool,
}

/// The windows of one project that the filter keeps, in display order.
///
/// `project_matches` is true when the project's own name matched, which shows
/// every window below it whatever its own name is.
pub(super) fn visible_windows<'a>(
    members: Vec<ProjectChild<'a>>,
    filtering: bool,
    project_matches: bool,
    filter: &str,
    keep_finished_secs: u64,
) -> Vec<VisibleWindow<'a>> {
    members
        .into_iter()
        .filter_map(|member| match member {
            ProjectChild::Window(window) => {
                let window_matches = filtering && matches_filter(&window.name, filter);
                let forest = subagent_forest(&window.subagents, keep_finished_secs);
                let subagent_matches = filtering
                    && forest
                        .iter()
                        .any(|node| subagent_branch_matches(node, filter));
                if filtering && !project_matches && !window_matches && !subagent_matches {
                    return None;
                }
                Some(VisibleWindow {
                    window,
                    forest,
                    show_all: !filtering || project_matches || window_matches,
                })
            }
        })
        .collect()
}

/// What does not change as the sub-agent walk descends below one window.
pub(super) struct SubagentWalk<'a, 'f> {
    pub window: &'a WindowInfo,
    pub filter: &'f str,
    /// True when every sub-agent below the window is shown, because the window
    /// or its project matched the filter itself.
    pub show_all: bool,
}

/// Appends the rows for one sub-agent level and, recursively, everything below it.
pub(super) fn emit_subagents<'a>(
    rows: &mut Vec<Row<'a>>,
    walk: &SubagentWalk<'a, '_>,
    nodes: &[SubagentNode<'a>],
    ancestors: &mut Vec<bool>,
    depth: u16,
    ancestor_matches: bool,
) {
    let SubagentWalk {
        window,
        filter,
        show_all,
    } = *walk;
    let visible: Vec<_> = nodes
        .iter()
        .filter(|node| show_all || ancestor_matches || subagent_branch_matches(node, filter))
        .collect();
    for (index, node) in visible.iter().enumerate() {
        let has_later_sibling = index + 1 < visible.len();
        rows.push(Row {
            key: NodeKey::Subagent {
                window_id: window.id,
                id: node.info.id.clone(),
            },
            guides: guide_prefix(ancestors, has_later_sibling),
            depth,
            kind: RowKind::Subagent { info: node.info },
        });
        ancestors.push(has_later_sibling);
        let node_matches = matches_subagent(node.info, filter);
        emit_subagents(
            rows,
            walk,
            &node.children,
            ancestors,
            depth.saturating_add(1),
            ancestor_matches || node_matches,
        );
        ancestors.pop();
    }
}
