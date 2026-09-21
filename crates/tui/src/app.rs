//! Client state and its pure update functions. Rendering lives in `ui`; I/O in `lib.rs`.

use crate::dialog::{FormDefaults, NewAgentForm, RemoveConfirm};
use crate::keymap::{Command, KeyAction, Keymap};
use crate::tree::{self, TreeState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{ClientMsg, DaemonMsg, GitState, Runtime, Status, WindowInfo};
use std::collections::{HashMap, HashSet};
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
    StopDaemon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    Confirm {
        message: String,
        action: PendingAction,
    },
    Help,
    /// The new-agent form; keys are handled in `app/modal_keys.rs`.
    NewAgent(NewAgentForm),
    /// The remove-confirm dialog (task M5.10).
    Remove(RemoveConfirm),
    /// The force-removal follow-up after a dirty-tree refusal (task M5.10).
    ForceRemove {
        window_id: u32,
        name: String,
        message: String,
    },
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
    /// Whether the graph overview draws the inspector panel below its canvas.
    /// `i` toggles it, and the choice outlives the overview it was made in
    /// (decision 7).
    pub inspector_visible: bool,
    /// The canvas coordinate at the graph overview's top-left corner.
    pub graph_pan: crate::graph::Pan,
    /// The overview's canvas viewport on screen; `set_graph_viewport` keeps it
    /// current, and it stays empty until the first frame that draws one.
    pub(crate) graph_area: ratatui::layout::Rect,
    pub(crate) graph_mouse: crate::mouse::MouseState,
    pub keymap: Keymap,
    pub modal: Option<Modal>,
    pub connected: bool,
    pub spinner_frame: usize,
    pub scroll_offset: usize,
    pub default_dir: PathBuf,
    /// Set by `tui::run` from `dirs::home_dir()`.
    pub home_dir: Option<PathBuf>,
    /// The last accepted new-agent form's values this session (decision 31); `dir` empty is `new_agent_defaults`'s sentinel for "nothing accepted yet".
    pub form_defaults: FormDefaults,
    /// The window a `Remove` with `remove_worktree: true` is in flight for, so that a
    /// `remove-dirty` refusal knows which window's dialog to open and an `Ack` or a
    /// plain `Error` for the removal knows to stop watching for one (decisions 35 and
    /// 36). `None` whenever no such removal is outstanding.
    pending_worktree_remove: Option<u32>,
    /// Git state by worktree root; pruned to current windows' roots (see `prune_git`).
    pub git: HashMap<PathBuf, GitState>,
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
            inspector_visible: true,
            graph_pan: crate::graph::Pan::default(),
            graph_area: ratatui::layout::Rect::default(),
            graph_mouse: crate::mouse::MouseState::default(),
            keymap: Keymap::new(prefix),
            modal: None,
            connected: true,
            spinner_frame: 0,
            scroll_offset: 0,
            default_dir,
            home_dir: None,
            form_defaults: FormDefaults {
                runtime: Runtime::Claude,
                dir: String::new(),
                model: String::new(),
            },
            pending_worktree_remove: None,
            git: HashMap::new(),
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

    /// The focused window's git state, by its worktree root.
    pub fn focused_git(&self) -> Option<&GitState> {
        let worktree = self.focused_window()?.worktree.as_ref()?;
        self.git.get(worktree)
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
                // Decision 34: a still-open form closes and hands its values on.
                match self.modal.take() {
                    Some(Modal::NewAgent(form)) => self.form_defaults = form.defaults(),
                    other => self.modal = other,
                }
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
            DaemonMsg::Error { request, message } => {
                // Decision 33: a submitting `create` failure goes inline, not a toast.
                if request == proto::messages::request::CREATE
                    && let Some(Modal::NewAgent(form)) = &mut self.modal
                    && form.submitting
                {
                    form.error = Some(message);
                    form.submitting = false;
                } else if request == proto::messages::request::REMOVE_DIRTY
                    && self.open_force_remove(message.clone())
                {
                    // Decision 36: handled by opening the force-or-keep follow-up.
                } else {
                    self.clear_pending_worktree_remove_on(&request);
                    self.toast(message);
                }
                vec![]
            }
            DaemonMsg::Bye { reason } => {
                self.connected = false;
                self.toast(format!("daemon: {reason}"));
                vec![]
            }
            DaemonMsg::Ack { request } => {
                self.clear_pending_worktree_remove_on(&request);
                vec![]
            }
            DaemonMsg::Git { root, state } => {
                match state {
                    Some(state) => {
                        self.git.insert(root, state);
                    }
                    None => {
                        self.git.remove(&root);
                    }
                }
                vec![]
            }
        }
    }

    /// The daemon never publishes `None` for an unregistered root, so the client prunes its
    /// own `git` map to the current windows' roots on every window-list change.
    fn prune_git(&mut self) {
        let live: HashSet<PathBuf> = self
            .windows
            .iter()
            .filter_map(|w| w.worktree.clone())
            .collect();
        self.git.retain(|root, _| live.contains(root));
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
        // One derivation of the outgoing rows serves both readers below: the
        // agent order the focus falls back through, and the keys the reveal
        // compares against.
        let previous_rows = self.rows();
        let previous_order = tree::agent_order(&previous_rows);
        let previous_keys: Vec<_> = previous_rows.iter().map(|row| row.key.clone()).collect();
        let previous_index = self
            .focused
            .and_then(|id| previous_order.iter().position(|candidate| *candidate == id));
        let previous_selection = self.tree.selected.clone();
        self.windows = windows;
        self.prune_git();
        self.windows_received_at = Instant::now();
        self.tree.prune(&self.windows);
        let rows = tree::build(&self.windows, &self.tree);
        self.tree.repair_selection(&rows);
        // Only on the edges decision 15 names, never on every list. The daemon
        // republishes on every status flip and every output event — several
        // times a second while agents work — and revealing unconditionally
        // would snap the canvas back to the selection about as fast as a
        // person can scroll away from it, undoing what decision 16 grants.
        // A change of focus is the third edge and reveals from `focus` itself,
        // which is the only thing that moves it from here.
        if self.tree.selected != previous_selection
            || rows.iter().map(|row| &row.key).ne(previous_keys.iter())
        {
            self.reveal_tree_anchor();
        }

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
        if self.modal.is_some() {
            return self.on_modal_key(key);
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

    fn perform(&mut self, action: PendingAction) -> Vec<Effect> {
        match action {
            PendingAction::Kill(id) => vec![Effect::Send(ClientMsg::Kill { window_id: id })],
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
                // Decision 26: opens the form; `app/modal_keys.rs` submits it.
                let defaults = self.new_agent_defaults();
                self.modal = Some(Modal::NewAgent(NewAgentForm::new(&defaults)));
                vec![]
            }
            Command::KillWindow => self.confirm_focused("Kill", PendingAction::Kill),
            // Decision 35: straight to the remove-confirm dialog, not the generic
            // yes/no `Confirm` modal, because this one carries its own checkbox.
            Command::RemoveWindow => self.open_remove_confirm(),
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

    pub fn on_paste(&mut self, text: String) -> Vec<Effect> {
        // Decision 30: a paste goes to the open form's focused field, or is dropped for
        // any other modal — both checked before tree mode and the PTY (risk 10).
        if let Some(modal) = &mut self.modal {
            if let Modal::NewAgent(form) = modal {
                form.on_paste(&text);
            }
            return vec![];
        }
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

    pub(crate) fn scroll_to_live(&mut self) {
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

mod modal_keys;

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
