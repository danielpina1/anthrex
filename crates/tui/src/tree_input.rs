//! Pure tree-mode state transitions and input handling.

use crate::app::{App, Effect, TreeInput, nav_rows_of};
use crate::keymap::Command;
use crate::tree::{self, NodeKey};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use unicode_segmentation::UnicodeSegmentation;

impl App {
    pub fn rows(&self) -> Vec<tree::Row<'_>> {
        tree::build_with_runs(&self.windows, &self.runs.runs, &self.tree)
    }

    pub fn set_tree_viewports(&mut self, sidebar_rows: u16, overview_rows: u16) {
        let changed =
            self.tree.sidebar.height != sidebar_rows || self.tree.overview.height != overview_rows;
        self.tree.sidebar.height = sidebar_rows;
        self.tree.overview.height = overview_rows;
        if changed {
            self.reveal_tree_anchor();
        }
    }

    /// The renderer calls this with the overview's own area after every draw,
    /// the way `set_tree_viewports` reports the list heights.
    pub fn set_graph_viewport(&mut self, main: Rect) {
        self.graph_main = Some(main);
        self.settle_graph_viewport();
    }

    /// Re-splits the last frame's overview area. The split is the frame's own, the run
    /// view's tall panel included, or the reveal would aim at a viewport the frame
    /// does not have; opening and leaving the run view call it at once, so the keys
    /// that follow before the next frame reveal into the right canvas.
    pub(crate) fn settle_graph_viewport(&mut self) {
        let Some(main) = self.graph_main else {
            return;
        };
        let (canvas, _) =
            crate::ui::overview::areas(main, self.inspector_visible, self.run_view.is_some());
        if self.graph_area != canvas {
            self.graph_area = canvas;
            self.reveal_graph_selection();
        }
    }

    pub(crate) fn reveal_tree_anchor(&mut self) {
        let rows = self.rows();
        let index = if self.tree_input.is_some() {
            self.tree.selected_index(&rows)
        } else {
            self.focused_window().and_then(|window| {
                tree::row_index(&rows, &NodeKey::Window(window.id))
                    .or_else(|| tree::row_index(&rows, &NodeKey::Project(window.project.clone())))
            })
        };
        let len = rows.len();
        if let Some(index) = index
            && self.tree.sidebar.height > 0
        {
            // `self.tree.overview` is not revealed: the overview is a graph
            // now and pans through `graph_pan`, so nothing reads that
            // one-dimensional viewport (see the brief's implementation notes).
            self.tree.sidebar.reveal(index);
        }
        self.tree.sidebar.scroll(0, len);
        self.tree.overview.scroll(0, len);
        self.reveal_graph_selection();
    }

    /// The two-dimensional form of `reveal_tree_anchor`'s rule: if the
    /// selected node's rectangle is not wholly inside the viewport, the pan
    /// moves by the smallest amount on each axis that puts it inside
    /// (decision 15).
    ///
    /// The rectangle and the canvas size exist only once the layout has run,
    /// so the reveal has to happen where a layout is in hand. It cannot be the
    /// renderer, which takes `&App` and would also have to re-reveal on every
    /// frame — undoing the wheel and a drag, which decision 16 leaves as the
    /// only ways to look away from the selection. So it happens here, beside
    /// the one-dimensional rule it generalises, on the same three edges:
    /// a change of selection, rows or focus.
    pub(crate) fn reveal_graph_selection(&mut self) {
        if !self.overview || self.graph_area.is_empty() {
            return;
        }
        let layout = crate::graph::layout(&self.nav_rows());
        let selected = self
            .tree
            .selected
            .as_ref()
            .and_then(|key| layout.node(key))
            .map(|node| node.rect);
        self.graph_pan = match selected {
            Some(rect) => self.graph_pan.revealing(rect, layout.size, self.graph_area),
            None => self.graph_pan.clamped(layout.size, self.graph_area),
        };
    }

    pub fn enter_tree(&mut self) {
        self.sidebar_visible = true;
        self.enter_tree_navigation();
    }

    pub(crate) fn enter_overview(&mut self) {
        self.overview = true;
        self.enter_tree_navigation();
    }

    fn enter_tree_navigation(&mut self) {
        self.tree_input = Some(TreeInput::Navigate);
        self.keymap.set_tree_mode(true);

        let rows = tree::build_with_runs(&self.windows, &self.runs.runs, &self.tree);
        let selected = self
            .focused
            .map(NodeKey::Window)
            .filter(|key| tree::row_index(&rows, key).is_some())
            .or_else(|| rows.first().map(|row| row.key.clone()));
        self.tree.selected = None;
        if let Some(selected) = selected {
            self.tree.select(&rows, selected);
        }
        self.reveal_tree_anchor();
    }

    pub fn exit_tree(&mut self) {
        self.tree_input = None;
        self.run_view = None;
        self.keymap.set_tree_mode(false);
        self.tree.selected = None;
        self.tree.filter.clear();
        self.overview = false;
        self.reveal_tree_anchor();
    }

    pub(crate) fn run_tree_command(&mut self, command: Command) -> Vec<Effect> {
        match command {
            Command::ToggleTree => {
                if self.tree_input.is_some() {
                    self.exit_tree();
                } else {
                    self.enter_tree();
                }
            }
            Command::ToggleOverview => {
                if self.overview {
                    self.exit_tree();
                } else {
                    self.enter_overview();
                }
            }
            Command::NarrowSidebar => {
                self.sidebar_visible = true;
                self.sidebar_width = self
                    .sidebar_width
                    .saturating_sub(crate::ui::SIDEBAR_WIDTH_STEP)
                    .max(crate::ui::MIN_SIDEBAR_WIDTH);
            }
            Command::WidenSidebar => {
                self.sidebar_visible = true;
                self.sidebar_width = self
                    .sidebar_width
                    .saturating_add(crate::ui::SIDEBAR_WIDTH_STEP)
                    .min(crate::ui::MAX_SIDEBAR_WIDTH);
            }
            _ => unreachable!("non-tree command routed to tree input"),
        }
        vec![]
    }

    pub(crate) fn on_tree_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if self.tree_input == Some(TreeInput::Filter) {
            self.on_filter_key(key);
            return vec![];
        }
        if let Some(effects) = self.on_run_view_key(key) {
            return effects;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_tree_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_tree_selection(-1),
            KeyCode::Char('h') | KeyCode::Left => self.select_tree_parent(),
            // Milestone 8c decision 22: `l` on a run's node in the project overview
            // opens the run view, the same as Enter.
            KeyCode::Char('l') | KeyCode::Right => match self.project_run_selected() {
                Some(id) if self.overview => self.open_run_view(id),
                _ => self.select_first_visible_child(),
            },
            KeyCode::Enter => {
                if let Some(selected) = self.tree.selected.clone() {
                    return self.activate_tree_node(selected);
                }
            }
            // Decision 22 (Risks 2): a run's node in the project tree is a leaf, and it
            // shares its key with the run view's root, which never folds (review m1).
            KeyCode::Char(' ') if matches!(self.tree.selected, Some(NodeKey::Run(_))) => {}
            KeyCode::Char(' ') => {
                if let Some(selected) = self.tree.selected.clone() {
                    self.toggle_tree_node(&selected);
                }
            }
            // Only in the overview, which is the one place the panel is drawn,
            // and the choice is kept for the session (decision 7). The canvas
            // changes height under it, and `set_graph_viewport` settles that
            // before the next frame, so the selection is revealed into the rows
            // the toggle gave back.
            KeyCode::Char('i') if self.overview => {
                self.inspector_visible = !self.inspector_visible;
            }
            KeyCode::Char('/') => self.tree_input = Some(TreeInput::Filter),
            KeyCode::Esc => self.exit_tree(),
            _ => {}
        }
        vec![]
    }

    pub(crate) fn activate_tree_node(&mut self, key: NodeKey) -> Vec<Effect> {
        match key {
            key @ NodeKey::Project(_) => {
                self.toggle_tree_node(&key);
                vec![]
            }
            // Milestone 8c decisions 24 and 26: a headless window, and any sub-agent in
            // the run view (whose rows hold no `Window`), open the owning window's
            // conversation; only a PTY window is focused.
            NodeKey::Window(id) if self.is_headless(id) => self.open_conversation(id),
            NodeKey::Subagent { window_id: id, .. }
                if self.is_headless(id) || self.run_view.is_some() =>
            {
                self.open_conversation(id)
            }
            NodeKey::Window(id) | NodeKey::Subagent { window_id: id, .. } => {
                let effects = self.focus(id);
                self.exit_tree();
                effects
            }
            key @ (NodeKey::Run(_)
            | NodeKey::Planner { .. }
            | NodeKey::Scout { .. }
            | NodeKey::Task { .. }
            | NodeKey::Stage { .. }
            | NodeKey::AgentRound { .. }) => self.activate_run_node(key),
        }
    }

    /// The selected run's id while the selection is a project tree's `Run` node (not
    /// the run view's root, which has the same key).
    fn project_run_selected(&self) -> Option<String> {
        match &self.tree.selected {
            Some(NodeKey::Run(id)) if self.run_view.is_none() => Some(id.clone()),
            _ => None,
        }
    }

    fn move_tree_selection(&mut self, delta: isize) {
        let rows = nav_rows_of(
            &self.windows,
            &self.runs.runs,
            &self.tree,
            self.run_view.as_ref(),
        );
        self.tree.move_selection(&rows, delta);
        self.reveal_tree_anchor();
    }

    /// `h` and `Left`: select the nearest preceding row one level up, the row's parent
    /// in the visible pre-order list. A no-op at a root, which has no
    /// shallower row before it (decision 17).
    fn select_tree_parent(&mut self) {
        let rows = nav_rows_of(
            &self.windows,
            &self.runs.runs,
            &self.tree,
            self.run_view.as_ref(),
        );
        let Some(index) = self.tree.selected_index(&rows) else {
            return;
        };
        let depth = rows[index].depth;
        if depth == 0 {
            return;
        }
        if let Some(parent) = rows[..index].iter().rev().find(|row| row.depth < depth) {
            self.tree.select(&rows, parent.key.clone());
            self.reveal_tree_anchor();
        }
    }

    /// `l` and `Right`: select the row right after the selected one if it is one level
    /// deeper, the first visible child in the pre-order list. A no-op at a
    /// leaf, whether it has no children or is collapsed — either way the next
    /// row is not a child (decision 17, decision 7).
    fn select_first_visible_child(&mut self) {
        let rows = nav_rows_of(
            &self.windows,
            &self.runs.runs,
            &self.tree,
            self.run_view.as_ref(),
        );
        let Some(index) = self.tree.selected_index(&rows) else {
            return;
        };
        let depth = rows[index].depth;
        if let Some(child) = rows.get(index + 1)
            && child.depth == depth + 1
        {
            self.tree.select(&rows, child.key.clone());
            self.reveal_tree_anchor();
        }
    }

    pub(crate) fn toggle_tree_node(&mut self, key: &NodeKey) {
        if self.tree.toggle(key) {
            let rows = nav_rows_of(
                &self.windows,
                &self.runs.runs,
                &self.tree,
                self.run_view.as_ref(),
            );
            self.tree.repair_selection(&rows);
            self.reveal_tree_anchor();
        }
    }

    fn on_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c)
                if !c.is_control()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.tree.filter.push(c);
            }
            KeyCode::Backspace => {
                if let Some((index, _)) = self.tree.filter.grapheme_indices(true).next_back() {
                    self.tree.filter.truncate(index);
                }
            }
            KeyCode::Enter => {
                self.tree_input = Some(TreeInput::Navigate);
                return;
            }
            KeyCode::Esc => {
                self.tree.filter.clear();
                self.tree_input = Some(TreeInput::Navigate);
            }
            _ => return,
        }
        self.repair_filtered_selection();
    }

    fn repair_filtered_selection(&mut self) {
        let rows = nav_rows_of(
            &self.windows,
            &self.runs.runs,
            &self.tree,
            self.run_view.as_ref(),
        );
        if self.tree.selected_index(&rows).is_none() {
            let first = rows
                .iter()
                .find(|row| matches!(row.key, NodeKey::Window(_)))
                .or_else(|| rows.first());
            if let Some(row) = first {
                self.tree.select(&rows, row.key.clone());
            }
        }
        self.tree.repair_selection(&rows);
        self.reveal_tree_anchor();
    }

    pub(crate) fn on_tree_paste(&mut self, text: String) -> Vec<Effect> {
        if self.tree_input == Some(TreeInput::Filter) {
            self.tree
                .filter
                .extend(text.chars().filter(|c| *c != '\r' && *c != '\n'));
            self.repair_filtered_selection();
        }
        vec![]
    }
}
