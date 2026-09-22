//! Client state and its pure update functions. Rendering lives in `ui`; I/O in `lib.rs`.

use crate::dialog::{FormDefaults, NewAgentForm, RemoveConfirm};
use crate::keymap::{Command, KeyAction, Keymap};
use crate::settings::UiSettings;
use crate::tree::{self, TreeState};
use crossterm::event::KeyEvent;
pub use link::Link;
use prompt::RenamePrompt;
use proto::{ClientMsg, DaemonMsg, GitState, WindowInfo};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
    /// `bell.attention` / `bell.done` (decision 4): `lib.rs` writes the BEL byte.
    Bell,
    /// Decision 32: `C-b r` while not connected. `lib.rs` starts an attempt at once
    /// (unless one is already in flight) and opens a fresh 30 s window.
    Reconnect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    Kill(u32),
    /// Decision 23: a live window's restart confirmation. Carries the window id
    /// directly, the same way `Kill` does, rather than an index or a reliance on
    /// `App.focused` staying put — a dialog that instead trusted focus to still name
    /// the right window was milestone 5's worst defect (see `app/modal_keys.rs`'s
    /// `open_force_remove` doc comment for the sibling case this mirrors).
    Restart(u32),
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
    /// Every config `Problem` the CLI found, already formatted, shown once at start
    /// (decision 7). Dismissed by any key, like `Help`.
    Notice {
        title: String,
        lines: Vec<String>,
    },
    /// The rename box (task M6.10, decision 23). `app/prompt.rs` is the pure state and
    /// the client-side name check; key handling is `app/modal_keys.rs`'s
    /// `on_rename_key`, and rendering is `ui/modal.rs`.
    Rename(RenamePrompt),
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
    /// The client's resolved view of `config.toml` (task M6.9); loaded once by the
    /// CLI's `attach` and never touched again — reloading it while running is out of
    /// scope (milestone 6's "Out of scope" list).
    pub settings: UiSettings,
    pub modal: Option<Modal>,
    pub link: Link,
    pub spinner_frame: usize,
    pub scroll_offset: usize,
    pub default_dir: PathBuf,
    /// Set by `tui::run` from `dirs::home_dir()`.
    pub home_dir: Option<PathBuf>,
    /// The last accepted new-agent form's values this session (decision 31); `dir` empty is `new_agent_defaults`'s sentinel for "nothing accepted yet".
    pub form_defaults: FormDefaults,
    /// The window a `Remove` with `remove_worktree: true` is in flight for (decisions 35
    /// and 36). `None` whenever no such removal is outstanding.
    ///
    /// Exactly one, never a set, and that is the point rather than a simplification.
    /// `Ack { request: "remove" }` and `Error { request: "remove" }` carry no window id,
    /// so with two worktree removals outstanding the client could not tell which one a
    /// reply ended; `on_remove_confirm_key` therefore refuses to start a second while
    /// one is in flight. A `DaemonMsg::RemoveDirty` then cross-checks its own
    /// `window_id` against this slot, so the force prompt can only ever be opened for
    /// the window whose refusal it is displaying.
    pending_worktree_remove: Option<u32>,
    /// Git state by worktree root; pruned to current windows' roots (see `prune_git`).
    pub git: HashMap<PathBuf, GitState>,
    /// Decision 36: set the moment `C-b Q`'s confirm sends `Shutdown`, cleared the
    /// moment the wait ends — by the link closing (`on_link_lost`, the daemon's `Bye`
    /// alone does not end it: see that method's doc comment), by the send itself
    /// failing (`on_send_failed`), or by `on_tick`'s `STOPPING_TIMEOUT`. Exactly one of
    /// those three clears it on any given run, so a second `C-b Q` is always possible
    /// once one of them has.
    stopping: Option<Instant>,
    toast: Option<(String, Instant)>,
    windows_received_at: Instant,
    /// (cols, rows) of the main inner area; (0, 0) until the first draw.
    term_size: (u16, u16),
    pending_resize: Option<Instant>,
    pending_focus: Option<u32>,
    /// Decision 35: the window whose `Subscribe` was last handed to the connection —
    /// set optimistically by whatever emits the effect, cleared by `on_send_failed`
    /// when that particular send is reported refused. `App::focus`'s early return for
    /// the already-focused window also requires this to equal it, so a refused
    /// `Subscribe` is retried (by `on_tick`, or by a later `focus` call) instead of
    /// being masked forever by "we're already focused there."
    subscribed: Option<u32>,
}

impl App {
    pub fn new(windows: Vec<WindowInfo>, default_dir: PathBuf, settings: UiSettings) -> Self {
        let mut tree = TreeState::default();
        tree.keep_finished_secs = settings.tree_keep_finished_secs;
        Self {
            windows,
            focused: None,
            parser: vt100::Parser::new(24, 80, settings.scrollback_lines),
            sidebar_visible: true,
            sidebar_width: settings.sidebar_width,
            tree,
            tree_input: None,
            overview: false,
            inspector_visible: true,
            graph_pan: crate::graph::Pan::default(),
            graph_area: ratatui::layout::Rect::default(),
            graph_mouse: crate::mouse::MouseState::default(),
            keymap: Keymap::new(settings.prefix),
            modal: None,
            link: Link::Connected,
            spinner_frame: 0,
            scroll_offset: 0,
            default_dir,
            home_dir: None,
            form_defaults: FormDefaults {
                runtime: settings.default_runtime,
                dir: String::new(),
                model: String::new(),
            },
            pending_worktree_remove: None,
            git: HashMap::new(),
            stopping: None,
            toast: None,
            windows_received_at: Instant::now(),
            term_size: (0, 0),
            pending_resize: None,
            pending_focus: None,
            subscribed: None,
            settings,
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

    /// Decision 7: every config `Problem` `attach` found, already formatted, shown
    /// once at start in a dismissable notice. Called right after `App::new`; does
    /// nothing when there is nothing to report.
    pub fn report_config_problems(&mut self, problems: Vec<String>) {
        if !problems.is_empty() {
            self.modal = Some(Modal::Notice {
                title: " config ".to_string(),
                lines: problems,
            });
        }
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
        // gained, so do nothing at all — unless (decision 35) the `Subscribe` we
        // believe is already active never actually made it out, in which case this is
        // the retry, not a redundant resend.
        if self.focused == Some(id) && self.subscribed == Some(id) {
            return vec![];
        }
        self.focused = Some(id);
        self.reveal_tree_anchor();
        self.scroll_offset = 0;
        let (cols, rows) = self.term_size;
        self.parser = vt100::Parser::new(rows.max(1), cols.max(1), self.settings.scrollback_lines);
        self.subscribed = Some(id);
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: id,
            cols,
            rows,
        })]
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
                    self.parser = vt100::Parser::new(
                        rows.max(1),
                        cols.max(1),
                        self.settings.scrollback_lines,
                    );
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
                } else {
                    self.clear_pending_worktree_remove_on(&request);
                    self.toast(message);
                }
                vec![]
            }
            // Decision 36: the force-or-keep follow-up, but only when this client asked
            // for *this* window's worktree to be removed. A refusal that cannot be
            // matched to an outstanding removal is shown as a toast and nothing is
            // offered to force — see `open_force_remove`.
            DaemonMsg::RemoveDirty { window_id, message } => {
                if !self.open_force_remove(window_id, message.clone()) {
                    self.toast(message);
                }
                vec![]
            }
            DaemonMsg::Bye { reason } => {
                // Nothing is outstanding on a connection that is gone. Leaving the slot
                // set would block every later worktree removal behind a reply that can
                // never arrive.
                self.pending_worktree_remove = None;
                self.toast(format!("daemon: {reason}"));
                // Task M6.10's decision 36, sharpened by M6.11's decision 30: `Bye` is
                // the daemon's own confirmation that it is stopping, but it is not the
                // same event as the link actually closing, and `self.link` must not
                // change here. The event loop keeps reading after `Bye` and only calls
                // `on_link_lost` once the channel actually closes — the read arm used
                // to be gated on a flag this handler set `false` right here, which
                // meant the real close (and the `Quit` a pending `C-b Q` produces from
                // it) was never observed at all. Quitting *here*, before the socket has
                // actually gone, would race the daemon's own exit the same way.
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
            // Decision 23: `C-b ,` opens the rename box prefilled with the focused
            // window's current name; the id travels with the modal itself
            // (`RenamePrompt::window_id`), not through `self.focused`.
            Command::RenameWindow => {
                if let Some(w) = self.focused_window() {
                    self.modal = Some(Modal::Rename(RenamePrompt::new(w.id, &w.name)));
                }
                vec![]
            }
            Command::RestartWindow => self.restart_focused(),
            // Decision 23's connected half: a no-op affirmation, unchanged from
            // M6.10. Decision 32's disconnected half: starts an attempt at once,
            // opening a fresh 30 s window; `lib.rs` is the one that actually checks
            // "unless one is already in flight" (`App` has no visibility into the
            // event loop's in-flight task) before spawning it.
            Command::Reconnect => {
                if self.connected() {
                    self.toast("connected");
                    vec![]
                } else {
                    let reason = match &self.link {
                        Link::Reconnecting { reason, .. } | Link::Lost { reason } => reason.clone(),
                        Link::Connected => String::new(),
                    };
                    self.link = Link::Reconnecting {
                        attempts: 0,
                        reason,
                    };
                    vec![Effect::Reconnect]
                }
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

    /// Called every 100 ms: advances the spinner, expires toasts, retries a dropped
    /// `Subscribe`, flushes a debounced resize.
    pub fn on_tick(&mut self) -> Vec<Effect> {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
        if self
            .toast
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= TOAST_TTL)
        {
            self.toast = None;
        }
        // Decision 36: the third of the three ways `C-b Q`'s wait can end — nothing
        // arrived at all within `link::STOPPING_TIMEOUT`.
        self.check_stopping_timeout();
        // Decision 35: a full outgoing queue drops a `Subscribe` silently (`Input`
        // does the same on every keystroke, so this is the one command worth
        // retrying instead of leaving the user stuck on a stale screen); retried here
        // every 100 ms until it drains, i.e. until `subscribed` catches up with
        // `focused` again.
        if self.connected()
            && let Some(id) = self.focused
            && self.subscribed != Some(id)
        {
            let (cols, rows) = self.term_size;
            self.subscribed = Some(id);
            return vec![Effect::Send(ClientMsg::Subscribe {
                window_id: id,
                cols,
                rows,
            })];
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

mod lifecycle;
mod link;
mod modal_keys;
pub(crate) mod prompt;
mod windows;

#[cfg(test)]
mod tests;
