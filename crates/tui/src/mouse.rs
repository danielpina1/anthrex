//! Every mouse gesture the client understands: the wheel, clicks, double
//! clicks and drags, over the sidebar, the focused terminal and the graph
//! overview.
//!
//! Pure like the rest of `app`: a gesture reads the geometry the renderer
//! would produce for the same frame and returns effects. Nothing here draws.

use crate::app::{App, Effect};
use crate::graph::Pan;
use crate::tree::{self, NodeKey};
use crate::ui::{self, overview};
use proto::ClientMsg;
use std::time::{Duration, Instant};

/// Two left presses on the same cell within this are one double click.
/// Crossterm reports presses, never gestures, so the client recognises this
/// one itself.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// How far one wheel notch scrolls the graph canvas (decision 16).
const WHEEL_ROWS: u16 = 3;

/// What the last left press left behind: enough to recognise a double click,
/// and the cell a drag should measure its next step from.
#[derive(Debug, Default)]
pub struct MouseState {
    last_press: Option<(u16, u16, Instant)>,
    drag_from: Option<(u16, u16)>,
}

impl MouseState {
    /// Records a press and reports whether it completed a double click.
    ///
    /// A double click consumes the gesture, so a third press starts a new one
    /// instead of firing again on every press after the second.
    fn press(&mut self, column: u16, row: u16) -> bool {
        let double = self
            .last_press
            .is_some_and(|(x, y, at)| (x, y) == (column, row) && at.elapsed() < DOUBLE_CLICK);
        self.last_press = (!double).then(|| (column, row, Instant::now()));
        self.drag_from = Some((column, row));
        double
    }
}

impl App {
    /// The sidebar wheel scrolls tree rows and the overview's wheel pans the
    /// graph. Outside tree mode, the main wheel forwards SGR mouse reports
    /// when enabled, otherwise it scrolls the local terminal history.
    pub fn on_scroll(
        &mut self,
        up: bool,
        column: u16,
        row: u16,
        layout: &ui::Layout,
    ) -> Vec<Effect> {
        let main_inner = layout.main_inner;
        if self.modal.is_some() {
            return vec![];
        }
        if self.sidebar_visible && layout.sidebar_list.contains((column, row).into()) {
            self.tree
                .sidebar
                .scroll(if up { -3 } else { 3 }, self.rows().len());
            return vec![];
        }
        if self.overview {
            self.scroll_graph(up, column, row, layout.main);
            return vec![];
        }
        if self.tree_input.is_some() || !main_inner.contains((column, row).into()) {
            return vec![];
        }
        if self.parser.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None {
            let Some(id) = self.focused else {
                return vec![];
            };
            let x = column - main_inner.x + 1;
            let y = row - main_inner.y + 1;
            let button = if up { 64 } else { 65 };
            return vec![Effect::Send(ClientMsg::Input {
                window_id: id,
                bytes: format!("\x1b[<{button};{x};{y}M").into_bytes(),
            })];
        }
        let target = if up {
            self.scroll_offset + 3
        } else {
            self.scroll_offset.saturating_sub(3)
        };
        self.parser.screen_mut().set_scrollback(target);
        self.scroll_offset = self.parser.screen().scrollback();
        vec![]
    }

    /// The wheel over the graph canvas scrolls it vertically by three rows
    /// (decision 16). There are no panning keys, so this and a drag are the
    /// only ways to move the viewport by hand.
    fn scroll_graph(&mut self, up: bool, column: u16, row: u16, main: ratatui::layout::Rect) {
        let view = overview::view(self, main);
        if !view.canvas.contains((column, row).into()) {
            return;
        }
        let y = if up {
            view.pan.y.saturating_sub(WHEEL_ROWS)
        } else {
            view.pan.y.saturating_add(WHEEL_ROWS)
        };
        self.graph_pan = Pan { y, ..view.pan }.clamped(view.layout.size, view.canvas);
    }

    pub fn on_click(&mut self, column: u16, row: u16, layout: &ui::Layout) -> Vec<Effect> {
        if self.modal.is_some() {
            return vec![];
        }
        // Every press ends the previous gesture. Without this, a press on the
        // sidebar would leave the last canvas press as a drag anchor, and
        // dragging over the sidebar would pan the graph behind it.
        self.graph_mouse.drag_from = None;
        if self.overview
            && let Some(effects) = self.click_graph(column, row, layout.main)
        {
            return effects;
        }
        if !self.sidebar_visible {
            return vec![];
        }
        let rows = tree::build(&self.windows, &self.tree);
        let geometry =
            ui::tree_view::geometry(layout.sidebar_list, rows.len(), self.tree.sidebar.top);
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

    /// Clicking a node selects it and double-clicking focuses it; a click on
    /// an edge or a gap does nothing (decision 19).
    ///
    /// `None` means the press missed the canvas altogether, which leaves the
    /// sidebar — still drawn beside the overview — free to claim it. A press
    /// that lands on the canvas but misses every node is a different case:
    /// the canvas has claimed it, so it always returns `Some`, with an empty
    /// effect list when `node_at` finds nothing there.
    fn click_graph(
        &mut self,
        column: u16,
        row: u16,
        main: ratatui::layout::Rect,
    ) -> Option<Vec<Effect>> {
        let view = overview::view(self, main);
        if !view.canvas.contains((column, row).into()) {
            return None;
        }
        let double = self.graph_mouse.press(column, row);
        let Some(key) = view.geometry().node_at(&view.layout, column, row) else {
            return Some(vec![]);
        };
        let rows = tree::build(&self.windows, &self.tree);
        self.tree.select(&rows, key.clone());
        self.reveal_tree_anchor();
        Some(if double {
            self.activate_tree_node(key)
        } else {
            vec![]
        })
    }

    /// Dragging with the left button held pans the graph on both axes
    /// (decision 16). The canvas follows the cursor, so the cell the drag
    /// started on stays under it.
    pub fn on_drag(&mut self, column: u16, row: u16, layout: &ui::Layout) -> Vec<Effect> {
        if self.modal.is_some() || !self.overview {
            return vec![];
        }
        let Some((from_x, from_y)) = self.graph_mouse.drag_from else {
            return vec![];
        };
        self.graph_mouse.drag_from = Some((column, row));
        let view = overview::view(self, layout.main);
        self.graph_pan = Pan {
            x: dragged(view.pan.x, from_x, column),
            y: dragged(view.pan.y, from_y, row),
        }
        .clamped(view.layout.size, view.canvas);
        vec![]
    }
}

/// One axis of a drag: the viewport moves against the cursor by as far as the
/// cursor travelled, so the content appears to move with it.
fn dragged(pan: u16, from: u16, to: u16) -> u16 {
    if to >= from {
        pan.saturating_sub(to - from)
    } else {
        pan.saturating_add(from - to)
    }
}
