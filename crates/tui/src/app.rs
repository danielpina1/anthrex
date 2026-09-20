//! Client state and its pure update functions. Rendering lives in `ui`; I/O in `lib.rs`.

use crate::keymap::{Command, KeyAction, Keymap};
use crate::tree::{self, TreeState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{ClientMsg, DaemonMsg, Runtime, Status, WindowInfo, WindowSpec};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const SCROLLBACK_LINES: usize = 5000;
pub const TOAST_TTL: Duration = Duration::from_secs(4);
pub const RESIZE_DEBOUNCE: Duration = Duration::from_millis(30);
const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

fn push_sanitized_paste_byte(bytes: &mut Vec<u8>, byte: u8) {
    bytes.push(byte);
    while bytes.ends_with(BRACKETED_PASTE_START) || bytes.ends_with(BRACKETED_PASTE_END) {
        bytes.truncate(bytes.len() - BRACKETED_PASTE_START.len());
    }
}

fn sanitize_paste(text: &str) -> Vec<u8> {
    let source = text.as_bytes();
    let mut bytes = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'\r' && source.get(index + 1) == Some(&b'\n') {
            push_sanitized_paste_byte(&mut bytes, b'\r');
            index += 2;
        } else {
            let byte = if source[index] == b'\n' {
                b'\r'
            } else {
                source[index]
            };
            push_sanitized_paste_byte(&mut bytes, byte);
            index += 1;
        }
    }
    bytes
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Send(ClientMsg),
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    Kill(u32),
    Remove(u32),
    StopDaemon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    Confirm {
        message: String,
        action: PendingAction,
    },
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeInput {
    Navigate,
    Filter,
}

pub struct App {
    pub windows: Vec<WindowInfo>,
    pub focused: Option<u32>,
    pub parser: vt100::Parser,
    pub sidebar_visible: bool,
    pub sidebar_width: u16,
    pub tree: TreeState,
    pub tree_input: Option<TreeInput>,
    pub overview: bool,
    pub keymap: Keymap,
    pub modal: Option<Modal>,
    pub connected: bool,
    pub spinner_frame: usize,
    pub scroll_offset: usize,
    pub default_dir: PathBuf,
    toast: Option<(String, Instant)>,
    windows_received_at: Instant,
    /// (cols, rows) of the main inner area; (0, 0) until the first draw.
    term_size: (u16, u16),
    pending_resize: Option<Instant>,
    pending_focus: Option<u32>,
}

impl App {
    pub fn new(
        windows: Vec<WindowInfo>,
        default_dir: PathBuf,
        prefix: (KeyCode, KeyModifiers),
    ) -> Self {
        Self {
            windows,
            focused: None,
            parser: vt100::Parser::new(24, 80, SCROLLBACK_LINES),
            sidebar_visible: true,
            sidebar_width: crate::ui::DEFAULT_SIDEBAR_WIDTH,
            tree: TreeState::default(),
            tree_input: None,
            overview: false,
            keymap: Keymap::new(prefix),
            modal: None,
            connected: true,
            spinner_frame: 0,
            scroll_offset: 0,
            default_dir,
            toast: None,
            windows_received_at: Instant::now(),
            term_size: (0, 0),
            pending_resize: None,
            pending_focus: None,
        }
    }

    pub fn focused_window(&self) -> Option<&WindowInfo> {
        self.focused
            .and_then(|id| self.windows.iter().find(|w| w.id == id))
    }

    pub fn focused_index(&self) -> Option<usize> {
        self.focused
            .and_then(|id| self.windows.iter().position(|w| w.id == id))
    }

    /// Seconds since the window's status changed, extrapolated from the last list we received.
    pub fn elapsed_secs(&self, w: &WindowInfo) -> u64 {
        self.age_secs(w.since_secs)
    }

    /// Seconds value from the last window list, plus the time since it arrived.
    pub fn age_secs(&self, secs: u64) -> u64 {
        secs.saturating_add(self.windows_received_at.elapsed().as_secs())
    }

    pub fn toast_text(&self) -> Option<&str> {
        self.toast.as_ref().map(|(t, _)| t.as_str())
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    /// Focus this window as soon as it appears in the list (used for `Created` and `attach <name>`).
    pub fn request_focus(&mut self, id: u32) {
        self.pending_focus = Some(id);
    }

    /// The renderer calls this with the main inner area after every draw.
    pub fn set_terminal_size(&mut self, cols: u16, rows: u16) -> Vec<Effect> {
        if cols == 0 || rows == 0 || (cols, rows) == self.term_size {
            return vec![];
        }
        let first = self.term_size == (0, 0);
        self.term_size = (cols, rows);
        self.parser.screen_mut().set_size(rows, cols);
        if first {
            return self.ensure_focus();
        }
        self.pending_resize = Some(Instant::now());
        vec![]
    }

    fn ensure_focus(&mut self) -> Vec<Effect> {
        if self.term_size == (0, 0) {
            return vec![];
        }
        if let Some(id) = self.pending_focus
            && self.windows.iter().any(|w| w.id == id)
        {
            self.pending_focus = None;
            return self.focus(id);
        }
        if self.focused_window().is_some() {
            return vec![];
        }
        match tree::agent_order(&self.rows()).first().copied() {
            Some(id) => self.focus(id),
            None => {
                self.focused = None;
                vec![]
            }
        }
    }

    pub fn focus(&mut self, id: u32) -> Vec<Effect> {
        if !self.windows.iter().any(|w| w.id == id) {
            return vec![];
        }
        if self.term_size == (0, 0) {
            self.pending_focus = Some(id);
            return vec![];
        }
        // Re-focusing the window we are already on would throw away a screen we have and
        // ask for a second subscription to the same window. The daemon then has to tear
        // the first forwarder down and race it against the new snapshot; nothing is
        // gained, so do nothing at all.
        if self.focused == Some(id) {
            return vec![];
        }
        self.focused = Some(id);
        self.reveal_tree_anchor();
        self.scroll_offset = 0;
        let (cols, rows) = self.term_size;
        self.parser = vt100::Parser::new(rows.max(1), cols.max(1), SCROLLBACK_LINES);
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: id,
            cols,
            rows,
        })]
    }

    fn focus_relative(&mut self, delta: isize) -> Vec<Effect> {
        let visible = tree::agent_order(&self.rows());
        if visible.is_empty() {
            return vec![];
        }
        let len = visible.len() as isize;
        let id = if let Some(current) = self
            .focused
            .and_then(|id| visible.iter().position(|candidate| *candidate == id))
        {
            visible[(current as isize + delta).rem_euclid(len) as usize]
        } else {
            let expanded = tree::agent_order(&tree::build(&self.windows, &TreeState::default()));
            let current = self
                .focused
                .and_then(|id| expanded.iter().position(|candidate| *candidate == id));
            current
                .and_then(|current| {
                    (1..=expanded.len()).find_map(|offset| {
                        let index = (current as isize + delta.signum() * offset as isize)
                            .rem_euclid(expanded.len() as isize)
                            as usize;
                        visible
                            .contains(&expanded[index])
                            .then_some(expanded[index])
                    })
                })
                .unwrap_or(visible[0])
        };
        self.focus(id)
    }

    pub fn on_daemon(&mut self, msg: DaemonMsg) -> Vec<Effect> {
        match msg {
            DaemonMsg::Welcome { windows, .. } | DaemonMsg::WindowsChanged { windows } => {
                self.replace_windows(windows)
            }
            DaemonMsg::Created { window_id } => {
                if self.windows.iter().any(|w| w.id == window_id) {
                    self.focus(window_id)
                } else {
                    self.pending_focus = Some(window_id);
                    vec![]
                }
            }
            DaemonMsg::Snapshot {
                window_id,
                cols,
                rows,
                bytes,
            } => {
                if Some(window_id) == self.focused {
                    self.parser = vt100::Parser::new(rows.max(1), cols.max(1), SCROLLBACK_LINES);
                    self.parser.process(&bytes);
                    self.scroll_offset = 0;
                }
                vec![]
            }
            DaemonMsg::Output { window_id, bytes } => {
                if Some(window_id) == self.focused {
                    self.parser.process(&bytes);
                }
                vec![]
            }
            DaemonMsg::Error { message, .. } => {
                self.toast(message);
                vec![]
            }
            DaemonMsg::Bye { reason } => {
                self.connected = false;
                self.toast(format!("daemon: {reason}"));
                vec![]
            }
            DaemonMsg::Ack { .. } => vec![],
            // App.git and the bottom-bar segment land in a later M4.5 task.
            DaemonMsg::Git { .. } => vec![],
        }
    }

    fn replace_windows(&mut self, windows: Vec<WindowInfo>) -> Vec<Effect> {
        for w in &windows {
            if Some(w.id) == self.focused {
                continue;
            }
            let previous = self
                .windows
                .iter()
                .find(|old| old.id == w.id)
                .map(|old| old.status);
            if previous != Some(w.status) {
                match w.status {
                    Status::Attention => self.toast(format!("{} needs attention", w.name)),
                    Status::Done => self.toast(format!("{} finished", w.name)),
                    _ => {}
                }
            }
        }
        let previous_order = tree::agent_order(&self.rows());
        let previous_index = self
            .focused
            .and_then(|id| previous_order.iter().position(|candidate| *candidate == id));
        self.windows = windows;
        self.windows_received_at = Instant::now();
        self.tree.prune(&self.windows);
        let rows = tree::build(&self.windows, &self.tree);
        self.tree.repair_selection(&rows);
        self.reveal_tree_anchor();

        if let Some(id) = self.pending_focus
            && self.windows.iter().any(|w| w.id == id)
        {
            self.pending_focus = None;
            return self.focus(id);
        }
        if self.focused_window().is_none() {
            if let Some(i) = previous_index
                && !self.windows.is_empty()
            {
                let order = tree::agent_order(&self.rows());
                if let Some(id) = order.get(i.min(order.len().saturating_sub(1))).copied() {
                    return self.focus(id);
                }
            }
            return self.ensure_focus();
        }
        vec![]
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if let Some(modal) = self.modal.clone() {
            return self.on_modal_key(modal, key);
        }
        let app_cursor = self.parser.screen().application_cursor();
        match self.keymap.handle(key, app_cursor) {
            KeyAction::Send(bytes) => {
                self.scroll_to_live();
                match self.focused {
                    Some(id) => vec![Effect::Send(ClientMsg::Input {
                        window_id: id,
                        bytes,
                    })],
                    None => vec![],
                }
            }
            KeyAction::Run(cmd) => self.run(cmd),
            KeyAction::Tree(key) => self.on_tree_key(key),
            KeyAction::AwaitPrefix | KeyAction::Cancel | KeyAction::Nothing => vec![],
        }
    }

    fn on_modal_key(&mut self, modal: Modal, key: KeyEvent) -> Vec<Effect> {
        match modal {
            Modal::Help => {
                self.modal = None;
                vec![]
            }
            Modal::Confirm { action, .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    self.modal = None;
                    self.perform(action)
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.modal = None;
                    vec![]
                }
                _ => vec![],
            },
        }
    }

    fn perform(&mut self, action: PendingAction) -> Vec<Effect> {
        match action {
            PendingAction::Kill(id) => vec![Effect::Send(ClientMsg::Kill { window_id: id })],
            PendingAction::Remove(id) => vec![Effect::Send(ClientMsg::Remove {
                window_id: id,
                remove_worktree: false,
                force: false,
            })],
            PendingAction::StopDaemon => vec![Effect::Send(ClientMsg::Shutdown), Effect::Quit],
        }
    }

    fn run(&mut self, cmd: Command) -> Vec<Effect> {
        match cmd {
            Command::NextWindow => self.focus_relative(1),
            Command::PrevWindow => self.focus_relative(-1),
            Command::FocusIndex(i) => match tree::agent_order(&self.rows()).get(i).copied() {
                Some(id) => self.focus(id),
                None => vec![],
            },
            Command::NewWindow => {
                let (cols, rows) = self.term_size;
                vec![Effect::Send(ClientMsg::CreateWindow {
                    spec: WindowSpec {
                        name: None,
                        runtime: Runtime::Shell,
                        cwd: self.default_dir.clone(),
                        worktree_branch: None,
                        model: None,
                        initial_prompt: None,
                    },
                    cols: cols.max(1),
                    rows: rows.max(1),
                })]
            }
            Command::KillWindow => self.confirm_focused("Kill", PendingAction::Kill),
            Command::RemoveWindow => self.confirm_focused("Remove", PendingAction::Remove),
            Command::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                vec![]
            }
            Command::Detach => vec![Effect::Quit],
            Command::StopDaemon => {
                self.modal = Some(Modal::Confirm {
                    message: "Stop the daemon and kill every agent?".into(),
                    action: PendingAction::StopDaemon,
                });
                vec![]
            }
            Command::Help => {
                self.modal = Some(Modal::Help);
                vec![]
            }
            cmd @ (Command::ToggleTree
            | Command::ToggleOverview
            | Command::NarrowSidebar
            | Command::WidenSidebar) => self.run_tree_command(cmd),
        }
    }

    fn confirm_focused(&mut self, verb: &str, make: fn(u32) -> PendingAction) -> Vec<Effect> {
        if let Some((id, name)) = self.focused_window().map(|w| (w.id, w.name.clone())) {
            self.modal = Some(Modal::Confirm {
                message: format!("{verb} '{name}'?"),
                action: make(id),
            });
        }
        vec![]
    }

    pub fn on_paste(&mut self, text: String) -> Vec<Effect> {
        if self.tree_input.is_some() {
            return self.on_tree_paste(text);
        }
        let Some(id) = self.focused else {
            return vec![];
        };
        self.scroll_to_live();
        let bracketed = self.parser.screen().bracketed_paste();
        let mut bytes = Vec::with_capacity(text.len() + 12);
        if bracketed {
            bytes.extend_from_slice(BRACKETED_PASTE_START);
        }
        bytes.extend_from_slice(&sanitize_paste(&text));
        if bracketed {
            bytes.extend_from_slice(BRACKETED_PASTE_END);
        }
        vec![Effect::Send(ClientMsg::Input {
            window_id: id,
            bytes,
        })]
    }

    /// The sidebar wheel scrolls tree rows. Outside tree mode, the main wheel forwards
    /// SGR mouse reports when enabled, otherwise it scrolls the local terminal history.
    pub fn on_scroll(
        &mut self,
        up: bool,
        column: u16,
        row: u16,
        layout: &crate::ui::Layout,
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

    fn scroll_to_live(&mut self) {
        if self.scroll_offset != 0 {
            self.parser.screen_mut().set_scrollback(0);
            self.scroll_offset = 0;
        }
    }

    /// Called every 100 ms: advances the spinner, expires toasts, flushes a debounced resize.
    pub fn on_tick(&mut self) -> Vec<Effect> {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
        if self
            .toast
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= TOAST_TTL)
        {
            self.toast = None;
        }
        if self
            .pending_resize
            .is_some_and(|at| at.elapsed() >= RESIZE_DEBOUNCE)
        {
            self.pending_resize = None;
            if let Some(id) = self.focused {
                let (cols, rows) = self.term_size;
                return vec![Effect::Send(ClientMsg::Resize {
                    window_id: id,
                    cols,
                    rows,
                })];
            }
        }
        vec![]
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
