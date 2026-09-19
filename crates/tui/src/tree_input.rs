//! Pure tree-mode state transitions and input handling.

use crate::app::{App, Effect, TreeInput};
use crate::keymap::Command;
use crate::tree::{self, NodeKey};
use crossterm::event::{KeyCode, KeyEvent};

impl App {
    pub fn rows(&self) -> Vec<tree::Row<'_>> {
        tree::build(&self.windows, &self.tree)
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

    pub fn on_click(&mut self, column: u16, row: u16, layout: &crate::ui::Layout) -> Vec<Effect> {
        if self.modal.is_some() || !self.sidebar_visible {
            return vec![];
        }
        let rows = self.rows();
        let geometry =
            crate::ui::tree_view::geometry(layout.sidebar_list, rows.len(), self.tree.sidebar.top);
        let Some(index) = geometry.index_at(column, row) else {
            return vec![];
        };
        match rows[index].key.clone() {
            NodeKey::Project(root) => {
                self.tree.toggle(&NodeKey::Project(root));
                self.reveal_tree_anchor();
                vec![]
            }
            NodeKey::Window(id) | NodeKey::Subagent { window_id: id, .. } => self.focus(id),
        }
    }

    pub(crate) fn reveal_tree_anchor(&mut self) {
        let rows = self.rows();
        let index = self.focused_window().and_then(|window| {
            tree::row_index(&rows, &NodeKey::Window(window.id))
                .or_else(|| tree::row_index(&rows, &NodeKey::Project(window.project.clone())))
        });
        let len = rows.len();
        if let Some(index) = index {
            if self.tree.sidebar.height > 0 {
                self.tree.sidebar.reveal(index);
            }
            if self.tree.overview.height > 0 {
                self.tree.overview.reveal(index);
            }
        }
        self.tree.sidebar.scroll(0, len);
        self.tree.overview.scroll(0, len);
    }

    pub fn enter_tree(&mut self) {
        self.sidebar_visible = true;
        self.tree_input = Some(TreeInput::Navigate);
        self.keymap.set_tree_mode(true);

        let rows = tree::build(&self.windows, &self.tree);
        let selected = self
            .focused
            .map(NodeKey::Window)
            .filter(|key| tree::row_index(&rows, key).is_some())
            .or_else(|| rows.first().map(|row| row.key.clone()));
        self.tree.selected = None;
        if let Some(selected) = selected {
            self.tree.select(&rows, selected);
        }
    }

    pub fn exit_tree(&mut self) {
        self.tree_input = None;
        self.keymap.set_tree_mode(false);
        self.tree.selected = None;
        self.tree.filter.clear();
        self.overview = false;
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
                    self.enter_tree();
                    self.overview = true;
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
        if key.code == KeyCode::Esc {
            self.exit_tree();
        }
        vec![]
    }

    pub(crate) fn on_tree_paste(&mut self, _text: String) -> Vec<Effect> {
        vec![]
    }
}
