//! Layout is asserted by exact rectangles (decision 9): every test names the
//! canvas cell each box starts at and how wide it is, so a wrong tier width or
//! a wrong centring shows up as a number rather than as a property that several
//! layouts would satisfy.

use super::*;
use crate::tree::{NodeKey, Row, RowKind, RuntimeCounts};
use proto::{Runtime, Status, SubagentInfo, SubagentState, WindowInfo};
use ratatui::layout::Rect;
use std::path::PathBuf;

/// A tree written as `(depth, name)` pairs in visible pre-order — the order and
/// the shape the sidebar's row builder produces — owning the records the rows
/// borrow.
///
/// The rows' `guides` are left empty on purpose. Layout must read `Row::depth`;
/// a layout that recovered depth from the guide string's width would put every
/// node of every one of these trees in tier 0.
struct Tree {
    spec: Vec<(u16, String)>,
    roots: Vec<PathBuf>,
    windows: Vec<WindowInfo>,
    subagents: Vec<SubagentInfo>,
    collapsed: Vec<bool>,
}

impl Tree {
    fn new(spec: &[(u16, &str)]) -> Self {
        let mut roots = Vec::new();
        let mut windows = Vec::new();
        let mut subagents = Vec::new();
        for (index, (_, name)) in spec.iter().enumerate() {
            roots.push(PathBuf::from(format!("/r/{index}")));
            windows.push(WindowInfo {
                id: index as u32,
                name: (*name).to_owned(),
                runtime: Runtime::Claude,
                cwd: format!("/r/{index}").into(),
                project: format!("/r/{index}").into(),
                worktree: None,
                branch: None,
                status: Status::Idle,
                tool: None,
                since_secs: 0,
                last_output_secs: 0,
                session_id: None,
                model: None,
                subagents: vec![],
                exit: None,
                kind: proto::WindowKind::Pty,
                run: None,
            });
            subagents.push(SubagentInfo {
                id: index.to_string(),
                parent_id: None,
                kind: (*name).to_owned(),
                label: None,
                model: None,
                state: SubagentState::Running,
                tool: None,
                started_secs: 0,
                ended_secs: None,
                needs_permission: false,
            });
        }
        Self {
            spec: spec.iter().map(|(d, n)| (*d, (*n).to_owned())).collect(),
            roots,
            windows,
            subagents,
            collapsed: vec![false; spec.len()],
        }
    }

    fn collapse(mut self, index: usize) -> Self {
        self.collapsed[index] = true;
        self
    }

    /// The key of the node at `index` in the spec.
    fn key(&self, index: usize) -> NodeKey {
        match self.spec[index].0 {
            0 => NodeKey::Project(self.roots[index].clone()),
            1 => NodeKey::Window(self.windows[index].id),
            _ => NodeKey::Subagent {
                window_id: self.owning_window(index),
                id: self.subagents[index].id.clone(),
            },
        }
    }

    /// The id of the nearest window above `index`, which is the window a
    /// sub-agent key belongs to.
    fn owning_window(&self, index: usize) -> u32 {
        self.spec[..index]
            .iter()
            .rposition(|(depth, _)| *depth == 1)
            .map_or(0, |window| self.windows[window].id)
    }

    fn rows(&self) -> Vec<Row<'_>> {
        let mut position = 0;
        self.spec
            .iter()
            .enumerate()
            .map(|(index, (depth, name))| {
                let kind = match depth {
                    0 => RowKind::Project {
                        root: &self.roots[index],
                        name: name.clone(),
                        status: Status::Idle,
                        counts: RuntimeCounts::default(),
                        collapsed: self.collapsed[index],
                    },
                    1 => {
                        position += 1;
                        RowKind::Window {
                            info: &self.windows[index],
                            position,
                            // Irrelevant to layout; kept faithful so a reader
                            // is not misled about what a collapsed row is.
                            has_subagents: self.collapsed[index]
                                || self.spec.get(index + 1).is_some_and(|(d, _)| *d == 2),
                            collapsed: self.collapsed[index],
                        }
                    }
                    _ => RowKind::Subagent {
                        info: &self.subagents[index],
                    },
                };
                Row {
                    key: self.key(index),
                    guides: String::new(),
                    depth: *depth,
                    kind,
                }
            })
            .collect()
    }
}

/// The rectangle the layout gave the node at `index` in `tree`'s spec.
fn rect(layout: &Layout, tree: &Tree, index: usize) -> Rect {
    layout
        .node(&tree.key(index))
        .unwrap_or_else(|| panic!("node {index} should be placed"))
        .rect
}

#[test]
fn a_single_project_is_one_box_at_the_origin() {
    // "storefront" is 10 columns, plus the glyph and its space is 12, plus two
    // borders and a space of padding each side is 16.
    let tree = Tree::new(&[(0, "storefront")]);
    let layout = layout(&tree.rows());

    assert_eq!(layout.nodes.len(), 1);
    assert_eq!(rect(&layout, &tree, 0), Rect::new(0, 0, 16, 3));
    assert_eq!(layout.nodes[0].depth, 0);
    assert_eq!(layout.size, (16, 3));
    assert!(layout.edges.is_empty());
}

#[test]
fn tiers_share_a_width_and_differ_from_each_other() {
    // Tier 0's widest content is "storefront": 10 + 2 = 12, so its width is 16.
    // Tier 1's widest is "1 api-worker": 12 + 2 = 14, so its width is 18.
    // "web" and "2 ui" are narrower and take their tier's width all the same.
    let tree = Tree::new(&[(0, "storefront"), (1, "api-worker"), (1, "ui"), (0, "web")]);
    let layout = layout(&tree.rows());

    assert_eq!(rect(&layout, &tree, 0).width, 16);
    assert_eq!(rect(&layout, &tree, 3).width, 16);
    assert_eq!(rect(&layout, &tree, 1).width, 18);
    assert_eq!(rect(&layout, &tree, 2).width, 18);
}

#[test]
fn a_narrow_tier_is_held_at_the_floor() {
    // "ab" wants 2 + 2 + 4 = 8 and "1 c" wants 3 + 2 + 4 = 9; both tiers are
    // held at 12, and tier 1 therefore starts at 12 + 3.
    let tree = Tree::new(&[(0, "ab"), (1, "c")]);
    let layout = layout(&tree.rows());

    assert_eq!(MIN_NODE_WIDTH, 12);
    assert_eq!(rect(&layout, &tree, 0), Rect::new(0, 0, 12, 3));
    assert_eq!(rect(&layout, &tree, 1), Rect::new(15, 0, 12, 3));
}

#[test]
fn a_wide_tier_is_capped() {
    // 45 columns of name want 51; the tier is capped at 30.
    let tree = Tree::new(&[(0, "a-project-name-far-past-the-thirty-column-cap")]);
    let layout = layout(&tree.rows());

    assert_eq!(MAX_NODE_WIDTH, 30);
    assert_eq!(rect(&layout, &tree, 0), Rect::new(0, 0, 30, 3));
}

#[test]
fn x_accumulates_tier_widths_and_gaps() {
    // Widths 16, 18, 13. Tier 1 starts at 16 + 3 = 19 and tier 2 at
    // 16 + 3 + 18 + 3 = 40 — not at 2 x 19 = 38, which is what a
    // depth-times-a-constant placement would produce.
    let tree = Tree::new(&[(0, "storefront"), (1, "api-worker"), (2, "Explore")]);
    let layout = layout(&tree.rows());

    assert_eq!(rect(&layout, &tree, 0), Rect::new(0, 0, 16, 3));
    assert_eq!(rect(&layout, &tree, 1), Rect::new(19, 0, 18, 3));
    assert_eq!(rect(&layout, &tree, 2), Rect::new(40, 0, 13, 3));
}

#[test]
fn a_leaf_takes_the_next_free_row() {
    // Tier 1's widest is "3 three": 7 + 2 + 4 = 13. Each leaf advances the
    // next free row by NODE_HEIGHT + ROW_GAP = 4.
    let tree = Tree::new(&[(0, "storefront"), (1, "one"), (1, "two"), (1, "three")]);
    let layout = layout(&tree.rows());

    assert_eq!(rect(&layout, &tree, 1), Rect::new(19, 0, 13, 3));
    assert_eq!(rect(&layout, &tree, 2), Rect::new(19, 4, 13, 3));
    assert_eq!(rect(&layout, &tree, 3), Rect::new(19, 8, 13, 3));
}

#[test]
fn a_parent_centres_on_its_children() {
    // Three children at 0, 4 and 8 span rows 0 to 10, eleven rows; the parent's
    // three rows centre at (11 - 3) / 2 = 4 rows below the first child's top.
    let three = Tree::new(&[(0, "storefront"), (1, "one"), (1, "two"), (1, "three")]);
    let of_three = layout(&three.rows());
    assert_eq!(rect(&of_three, &three, 0), Rect::new(0, 4, 16, 3));

    // Two children at 0 and 4 span rows 0 to 6, seven rows; (7 - 3) / 2 = 2.
    let two = Tree::new(&[(0, "storefront"), (1, "one"), (1, "two")]);
    let of_two = layout(&two.rows());
    assert_eq!(rect(&of_two, &two, 0), Rect::new(0, 2, 16, 3));

    // "Rounded down" only bites when the span is even, which needs one child
    // on an odd row. "win-a" is centred on sub-agents that are themselves
    // centred: its two sub-agents land on rows 2 and 12, spanning 13 rows, so
    // "win-a" takes row 2 + (13 - 3) / 2 = 7. "win-b" is a leaf and takes the
    // next free row, 20. The project then spans rows 7 to 22 — sixteen rows,
    // an even span — and takes row 7 + (16 - 3) / 2 = 13, not 14.
    let odd = Tree::new(&[
        (0, "proj"),
        (1, "win-a"),
        (2, "s-one"),
        (3, "leaf-a"),
        (3, "leaf-b"),
        (2, "s-two"),
        (3, "leaf-c"),
        (3, "leaf-d"),
        (3, "leaf-e"),
        (1, "win-b"),
    ]);
    let of_odd = layout(&odd.rows());
    assert_eq!(rect(&of_odd, &odd, 1), Rect::new(15, 7, 13, 3));
    assert_eq!(rect(&of_odd, &odd, 9), Rect::new(15, 20, 13, 3));
    assert_eq!(rect(&of_odd, &odd, 0), Rect::new(0, 13, 12, 3));
}

#[test]
fn a_collapsed_node_is_a_leaf() {
    // The window is collapsed, so its sub-agents are not in the row list at
    // all: the next window still starts four rows below it, the parent centres
    // on two children rather than on a taller span, and the collapsed window
    // owns no edge.
    let tree = Tree::new(&[(0, "storefront"), (1, "api-worker"), (1, "ui")]).collapse(1);
    let layout = layout(&tree.rows());

    assert_eq!(layout.nodes.len(), 3);
    assert_eq!(rect(&layout, &tree, 1), Rect::new(19, 0, 18, 3));
    assert_eq!(rect(&layout, &tree, 2), Rect::new(19, 4, 18, 3));
    assert_eq!(rect(&layout, &tree, 0), Rect::new(0, 2, 16, 3));
    assert_eq!(
        layout.edges,
        vec![Edge {
            parent: tree.key(0),
            children: vec![tree.key(1), tree.key(2)],
        }]
    );
}

#[test]
fn a_six_level_chain_places_six_tiers() {
    // Widths 19, 12, 22, 12, 13, 12; each tier starts three columns past the
    // right edge of the one before it. Every box is an only child, so every
    // parent centres on it and the whole chain sits on row 0.
    let tree = Tree::new(&[
        (0, "project-alpha"),
        (1, "win"),
        (2, "explore-the-code"),
        (3, "bb"),
        (4, "plan-it"),
        (5, "dd"),
    ]);
    let layout = layout(&tree.rows());

    assert_eq!(rect(&layout, &tree, 0), Rect::new(0, 0, 19, 3));
    assert_eq!(rect(&layout, &tree, 1), Rect::new(22, 0, 12, 3));
    assert_eq!(rect(&layout, &tree, 2), Rect::new(37, 0, 22, 3));
    assert_eq!(rect(&layout, &tree, 3), Rect::new(62, 0, 12, 3));
    assert_eq!(rect(&layout, &tree, 4), Rect::new(77, 0, 13, 3));
    assert_eq!(rect(&layout, &tree, 5), Rect::new(93, 0, 12, 3));
    assert_eq!(
        layout
            .nodes
            .iter()
            .map(|node| node.depth)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5]
    );
    assert_eq!(layout.size, (105, 3));
}

#[test]
fn several_roots_stack_in_tier_zero() {
    let tree = Tree::new(&[(0, "alpha"), (0, "beta"), (0, "gamma")]);
    let layout = layout(&tree.rows());

    assert_eq!(rect(&layout, &tree, 0), Rect::new(0, 0, 12, 3));
    assert_eq!(rect(&layout, &tree, 1), Rect::new(0, 4, 12, 3));
    assert_eq!(rect(&layout, &tree, 2), Rect::new(0, 8, 12, 3));
    assert_eq!(layout.size, (12, 11));
    assert!(layout.edges.is_empty());
}

#[test]
fn size_is_the_bounding_box() {
    // The widest tier is not the last one to be placed and the lowest box is
    // not the last either: "ui" is the bottom of the canvas at rows 8 to 10,
    // and tier 2 at x 40 with width 13 is the right edge at column 53.
    let tree = Tree::new(&[
        (0, "storefront"),
        (1, "api-worker"),
        (2, "Explore"),
        (2, "Plan"),
        (1, "ui"),
    ]);
    let layout = layout(&tree.rows());

    assert_eq!(rect(&layout, &tree, 2), Rect::new(40, 0, 13, 3));
    assert_eq!(rect(&layout, &tree, 3), Rect::new(40, 4, 13, 3));
    assert_eq!(rect(&layout, &tree, 1), Rect::new(19, 2, 18, 3));
    assert_eq!(rect(&layout, &tree, 4), Rect::new(19, 8, 18, 3));
    assert_eq!(rect(&layout, &tree, 0), Rect::new(0, 5, 16, 3));
    assert_eq!(layout.size, (53, 11));
}

#[test]
fn wide_characters_count_as_two_columns() {
    // Nine CJK characters are eighteen columns, so the tier wants
    // 18 + 2 + 4 = 24. Counting characters would give 15 and counting UTF-8
    // bytes would give 30, the cap.
    let tree = Tree::new(&[(0, "日本語プロジェクト")]);
    let layout = layout(&tree.rows());

    assert_eq!(rect(&layout, &tree, 0), Rect::new(0, 0, 24, 3));
    assert_eq!(layout.size, (24, 3));
}

#[test]
fn every_status_glyph_is_one_column() {
    // `GLYPH_COLUMNS` budgets one column for the glyph and one for the space
    // after it. A two-column glyph would leave every box in the tree one
    // column short of its content. Sub-agent boxes draw `subagent_glyph`,
    // not `status_glyph` directly, so it is pinned too (deferred from the
    // previous task's review): the assumption held unguarded until now.
    for status in [
        Status::Starting,
        Status::Working,
        Status::Idle,
        Status::Attention,
        Status::Done,
        Status::Exited,
    ] {
        for frame in 0..crate::theme::SPINNER.len() {
            let glyph = crate::theme::status_glyph(status, frame);
            assert_eq!(
                unicode_width::UnicodeWidthStr::width(glyph),
                1,
                "glyph {glyph} for {status:?}"
            );
        }
    }

    for needs_permission in [false, true] {
        for state in [
            SubagentState::Running,
            SubagentState::Done,
            SubagentState::Failed,
        ] {
            let info = SubagentInfo {
                id: "s".into(),
                parent_id: None,
                kind: "Explore".into(),
                label: None,
                model: None,
                state,
                tool: None,
                started_secs: 0,
                ended_secs: None,
                needs_permission,
            };
            for frame in 0..crate::theme::SPINNER.len() {
                let glyph = crate::theme::subagent_glyph(&info, frame);
                assert_eq!(
                    unicode_width::UnicodeWidthStr::width(glyph),
                    1,
                    "subagent glyph {glyph} for state {state:?}, needs_permission {needs_permission}"
                );
            }
        }
    }
}
