//! Client state and its pure update functions. Rendering lives in `ui`; I/O in `lib.rs`.

use crate::keymap::{Command, KeyAction, Keymap};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{ClientMsg, DaemonMsg, Runtime, Status, WindowInfo, WindowSpec};
use ratatui::layout::Rect;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const SCROLLBACK_LINES: usize = 5000;
pub const TOAST_TTL: Duration = Duration::from_secs(4);
pub const RESIZE_DEBOUNCE: Duration = Duration::from_millis(30);

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

pub struct App {
    pub windows: Vec<WindowInfo>,
    pub focused: Option<u32>,
    pub parser: vt100::Parser,
    pub sidebar_visible: bool,
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
        w.since_secs + self.windows_received_at.elapsed().as_secs()
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
        match self.windows.first().map(|w| w.id) {
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
        if self.windows.is_empty() {
            return vec![];
        }
        let len = self.windows.len() as isize;
        let current = self.focused_index().map(|i| i as isize).unwrap_or(0);
        let next = (current + delta).rem_euclid(len) as usize;
        let id = self.windows[next].id;
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
        let previous_index = self.focused_index();
        self.windows = windows;
        self.windows_received_at = Instant::now();

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
                let id = self.windows[i.min(self.windows.len() - 1)].id;
                return self.focus(id);
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
            Command::FocusIndex(i) => match self.windows.get(i).map(|w| w.id) {
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
        let Some(id) = self.focused else {
            return vec![];
        };
        self.scroll_to_live();
        let bracketed = self.parser.screen().bracketed_paste();
        let mut bytes = Vec::with_capacity(text.len() + 12);
        if bracketed {
            bytes.extend_from_slice(b"\x1b[200~");
        }
        bytes.extend_from_slice(text.replace("\r\n", "\r").replace('\n', "\r").as_bytes());
        if bracketed {
            bytes.extend_from_slice(b"\x1b[201~");
        }
        vec![Effect::Send(ClientMsg::Input {
            window_id: id,
            bytes,
        })]
    }

    /// Mouse wheel over the main area. Forwarded as an SGR mouse report when the program
    /// enabled mouse tracking; otherwise scrolls the local scrollback view.
    pub fn on_scroll(&mut self, up: bool, column: u16, row: u16, main_inner: Rect) -> Vec<Effect> {
        if self.modal.is_some() || !main_inner.contains((column, row).into()) {
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
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use proto::{Runtime, Status};

    fn win(id: u32, name: &str, status: Status) -> WindowInfo {
        WindowInfo {
            id,
            name: name.into(),
            runtime: Runtime::Shell,
            cwd: "/tmp".into(),
            branch: None,
            status,
            tool: None,
            since_secs: 0,
            last_output_secs: 0,
            has_session: false,
            exit: None,
        }
    }

    fn app_with(windows: Vec<WindowInfo>) -> App {
        let mut app = App::new(windows, "/tmp".into(), Keymap::default_prefix());
        // The renderer reports the size on the first draw; simulate that.
        let _ = app.set_terminal_size(80, 24);
        app
    }

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Vec<Effect> {
        app.on_key(key(code, mods))
    }

    fn prefix(app: &mut App) {
        assert!(press(app, KeyCode::Char('b'), KeyModifiers::CONTROL).is_empty());
    }

    #[test]
    fn first_size_report_subscribes_to_the_first_window() {
        let mut app = App::new(
            vec![win(4, "a", Status::Idle), win(5, "b", Status::Idle)],
            "/tmp".into(),
            Keymap::default_prefix(),
        );
        assert_eq!(app.focused, None);
        let effects = app.set_terminal_size(100, 30);
        assert_eq!(
            effects,
            vec![Effect::Send(ClientMsg::Subscribe {
                window_id: 4,
                cols: 100,
                rows: 30
            })]
        );
        assert_eq!(app.focused, Some(4));
        assert_eq!(app.parser.screen().size(), (30, 100));
    }

    #[test]
    fn focus_requested_before_the_first_size_report_subscribes_once_sized() {
        let mut app = App::new(
            vec![win(4, "a", Status::Idle), win(5, "b", Status::Idle)],
            "/tmp".into(),
            Keymap::default_prefix(),
        );
        assert!(app.focus(5).is_empty());
        assert_eq!(app.focused, None);
        assert!(
            app.on_daemon(DaemonMsg::Created { window_id: 5 })
                .is_empty()
        );
        assert_eq!(app.focused, None);
        let effects = app.set_terminal_size(100, 30);
        assert_eq!(
            effects,
            vec![Effect::Send(ClientMsg::Subscribe {
                window_id: 5,
                cols: 100,
                rows: 30
            })]
        );
        assert_eq!(app.focused, Some(5));
    }

    #[test]
    fn later_size_changes_are_debounced_into_a_resize() {
        let mut app = app_with(vec![win(1, "a", Status::Idle)]);
        assert!(app.set_terminal_size(120, 40).is_empty());
        assert!(
            app.on_tick().is_empty(),
            "resize is not sent before the debounce window"
        );
        std::thread::sleep(RESIZE_DEBOUNCE + Duration::from_millis(5));
        assert_eq!(
            app.on_tick(),
            vec![Effect::Send(ClientMsg::Resize {
                window_id: 1,
                cols: 120,
                rows: 40
            })]
        );
        assert!(app.on_tick().is_empty());
    }

    #[test]
    fn snapshot_and_output_feed_the_focused_parser_only() {
        let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
        app.on_daemon(DaemonMsg::Snapshot {
            window_id: 1,
            cols: 80,
            rows: 24,
            bytes: b"hello".to_vec(),
        });
        assert!(app.parser.screen().contents().starts_with("hello"));
        app.on_daemon(DaemonMsg::Output {
            window_id: 2,
            bytes: b"IGNORED".to_vec(),
        });
        assert!(!app.parser.screen().contents().contains("IGNORED"));
        app.on_daemon(DaemonMsg::Output {
            window_id: 1,
            bytes: b" world".to_vec(),
        });
        assert!(app.parser.screen().contents().starts_with("hello world"));
    }

    #[test]
    fn keys_go_to_the_focused_window_and_prefix_switches() {
        let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
        assert_eq!(
            press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE),
            vec![Effect::Send(ClientMsg::Input {
                window_id: 1,
                bytes: b"l".to_vec()
            })]
        );
        prefix(&mut app);
        assert_eq!(
            press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE),
            vec![Effect::Send(ClientMsg::Subscribe {
                window_id: 2,
                cols: 80,
                rows: 24
            })]
        );
        assert_eq!(app.focused, Some(2));
        prefix(&mut app);
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(app.focused, Some(1), "next wraps around");
        prefix(&mut app);
        press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        assert_eq!(app.focused, Some(2));
        prefix(&mut app);
        assert_eq!(
            press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn new_window_creates_a_shell_in_the_default_dir_and_focuses_it_when_created() {
        let mut app = app_with(vec![]);
        prefix(&mut app);
        let effects = press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
        match &effects[..] {
            [
                Effect::Send(ClientMsg::CreateWindow {
                    spec,
                    cols: 80,
                    rows: 24,
                }),
            ] => {
                assert_eq!(spec.runtime, Runtime::Shell);
                assert_eq!(spec.cwd, PathBuf::from("/tmp"));
                assert_eq!(spec.name, None);
            }
            other => panic!("unexpected effects {other:?}"),
        }
        // The daemon may announce the window list before or after Created; both orders focus it.
        assert!(
            app.on_daemon(DaemonMsg::Created { window_id: 9 })
                .is_empty()
        );
        let effects = app.on_daemon(DaemonMsg::WindowsChanged {
            windows: vec![win(9, "shell-9", Status::Starting)],
        });
        assert_eq!(
            effects,
            vec![Effect::Send(ClientMsg::Subscribe {
                window_id: 9,
                cols: 80,
                rows: 24
            })]
        );
        assert_eq!(app.focused, Some(9));
    }

    /// I8: re-focusing the focused window must not resubscribe or reset the parser.
    #[test]
    fn focusing_the_already_focused_window_does_nothing() {
        let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
        assert_eq!(app.focused, Some(1));
        app.on_daemon(DaemonMsg::Snapshot {
            window_id: 1,
            cols: 80,
            rows: 24,
            bytes: b"kept".to_vec(),
        });
        assert!(
            app.focus(1).is_empty(),
            "no Subscribe for the window we are already on"
        );
        assert!(
            app.parser.screen().contents().starts_with("kept"),
            "the screen must survive"
        );
        // Only a real change still subscribes.
        assert_eq!(
            app.focus(2),
            vec![Effect::Send(ClientMsg::Subscribe {
                window_id: 2,
                cols: 80,
                rows: 24
            })]
        );
        // C-b 2 on the window already focused is likewise a no-op.
        prefix(&mut app);
        assert!(press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE).is_empty());
    }

    #[test]
    fn kill_asks_for_confirmation_first() {
        let mut app = app_with(vec![win(1, "a", Status::Working)]);
        prefix(&mut app);
        assert!(press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE).is_empty());
        assert!(matches!(
            app.modal,
            Some(Modal::Confirm {
                action: PendingAction::Kill(1),
                ..
            })
        ));
        assert!(press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE).is_empty());
        assert!(app.modal.is_none());
        prefix(&mut app);
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(
            press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE),
            vec![Effect::Send(ClientMsg::Kill { window_id: 1 })]
        );
        prefix(&mut app);
        press(&mut app, KeyCode::Char('Q'), KeyModifiers::SHIFT);
        assert_eq!(
            press(&mut app, KeyCode::Enter, KeyModifiers::NONE),
            vec![Effect::Send(ClientMsg::Shutdown), Effect::Quit]
        );
    }

    #[test]
    fn removed_focused_window_moves_focus_to_a_neighbour() {
        let mut app = app_with(vec![
            win(1, "a", Status::Idle),
            win(2, "b", Status::Idle),
            win(3, "c", Status::Idle),
        ]);
        app.focus(2);
        let effects = app.on_daemon(DaemonMsg::WindowsChanged {
            windows: vec![win(1, "a", Status::Idle), win(3, "c", Status::Idle)],
        });
        assert_eq!(
            effects,
            vec![Effect::Send(ClientMsg::Subscribe {
                window_id: 3,
                cols: 80,
                rows: 24
            })]
        );
        let effects = app.on_daemon(DaemonMsg::WindowsChanged { windows: vec![] });
        assert!(effects.is_empty());
        assert_eq!(app.focused, None);
    }

    #[test]
    fn background_status_changes_raise_toasts() {
        let mut app = app_with(vec![
            win(1, "a", Status::Working),
            win(2, "b", Status::Working),
        ]);
        app.on_daemon(DaemonMsg::WindowsChanged {
            windows: vec![
                win(1, "a", Status::Attention),
                win(2, "b", Status::Attention),
            ],
        });
        assert_eq!(
            app.toast_text(),
            Some("b needs attention"),
            "the focused window (1) never toasts"
        );
        app.on_daemon(DaemonMsg::WindowsChanged {
            windows: vec![win(1, "a", Status::Attention), win(2, "b", Status::Done)],
        });
        assert_eq!(app.toast_text(), Some("b finished"));
        app.on_daemon(DaemonMsg::Error {
            request: "kill".into(),
            message: "no window with id 7".into(),
        });
        assert_eq!(app.toast_text(), Some("no window with id 7"));
    }

    #[test]
    fn paste_uses_bracketed_mode_when_the_program_asked_for_it() {
        let mut app = app_with(vec![win(1, "a", Status::Idle)]);
        assert_eq!(
            app.on_paste("ab\ncd".into()),
            vec![Effect::Send(ClientMsg::Input {
                window_id: 1,
                bytes: b"ab\rcd".to_vec()
            })]
        );
        app.parser.process(b"\x1b[?2004h");
        assert_eq!(
            app.on_paste("x".into()),
            vec![Effect::Send(ClientMsg::Input {
                window_id: 1,
                bytes: b"\x1b[200~x\x1b[201~".to_vec()
            })]
        );
    }

    #[test]
    fn wheel_scrolls_the_local_scrollback_and_any_key_snaps_back() {
        let mut app = app_with(vec![win(1, "a", Status::Idle)]);
        for i in 0..40 {
            app.parser.process(format!("line {i}\r\n").as_bytes());
        }
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        assert!(app.on_scroll(true, 5, 5, area).is_empty());
        assert_eq!(app.scroll_offset, 3);
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(app.scroll_offset, 0);
        app.parser.process(b"\x1b[?1000h\x1b[?1006h");
        let effects = app.on_scroll(false, 5, 5, area);
        assert_eq!(
            effects,
            vec![Effect::Send(ClientMsg::Input {
                window_id: 1,
                bytes: b"\x1b[<65;6;6M".to_vec()
            })]
        );
    }
}
