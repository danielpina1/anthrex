//! Pure tree-mode state transitions and input handling.

use crate::app::{App, Effect, TreeInput};
use crate::keymap::Command;
use crate::tree::{self, NodeKey};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

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
        if self.modal.is_some() {
            return vec![];
        }
        let rows = tree::build(&self.windows, &self.tree);
        if self.overview {
            let geometry = crate::ui::tree_view::geometry(
                layout.main_inner,
                rows.len(),
                self.tree.overview.top,
            );
            if let Some(index) = geometry.index_at(column, row) {
                let key = rows[index].key.clone();
                self.tree.select(&rows, key.clone());
                return self.activate_tree_node(key);
            }
        }
        if !self.sidebar_visible {
            return vec![];
        }
        let geometry =
            crate::ui::tree_view::geometry(layout.sidebar_list, rows.len(), self.tree.sidebar.top);
        let Some(index) = geometry.index_at(column, row) else {
            return vec![];
        };
        let key = rows[index].key.clone();
        if self.tree_input.is_some() {
            self.tree.select(&rows, key.clone());
        }
        let effects = match key {
            key @ NodeKey::Project(_) => {
                self.toggle_tree_node(&key);
                vec![]
            }
            NodeKey::Window(id) | NodeKey::Subagent { window_id: id, .. } => self.focus(id),
        };
        self.reveal_tree_anchor();
        effects
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
        self.enter_tree_navigation();
    }

    fn enter_overview(&mut self) {
        self.overview = true;
        self.enter_tree_navigation();
    }

    fn enter_tree_navigation(&mut self) {
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
        self.reveal_tree_anchor();
    }

    pub fn exit_tree(&mut self) {
        self.tree_input = None;
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
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_tree_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_tree_selection(-1),
            KeyCode::Enter => {
                if let Some(selected) = self.tree.selected.clone() {
                    return self.activate_tree_node(selected);
                }
            }
            KeyCode::Char(' ') => {
                if let Some(selected) = self.tree.selected.clone() {
                    self.toggle_tree_node(&selected);
                }
            }
            KeyCode::Char('/') => self.tree_input = Some(TreeInput::Filter),
            KeyCode::Esc => self.exit_tree(),
            _ => {}
        }
        vec![]
    }

    fn activate_tree_node(&mut self, key: NodeKey) -> Vec<Effect> {
        match key {
            key @ NodeKey::Project(_) => {
                self.toggle_tree_node(&key);
                vec![]
            }
            NodeKey::Window(id) | NodeKey::Subagent { window_id: id, .. } => {
                let effects = self.focus(id);
                self.exit_tree();
                effects
            }
        }
    }

    fn move_tree_selection(&mut self, delta: isize) {
        let rows = tree::build(&self.windows, &self.tree);
        self.tree.move_selection(&rows, delta);
        self.reveal_tree_anchor();
    }

    fn toggle_tree_node(&mut self, key: &NodeKey) {
        if self.tree.toggle(key) {
            let rows = tree::build(&self.windows, &self.tree);
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
        let rows = tree::build(&self.windows, &self.tree);
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
