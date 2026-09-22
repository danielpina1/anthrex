//! Turns crossterm key events into PTY bytes, and implements the tmux-style prefix key.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    NextWindow,
    PrevWindow,
    FocusIndex(usize),
    NewWindow,
    KillWindow,
    RemoveWindow,
    ToggleSidebar,
    Detach,
    StopDaemon,
    Help,
    ToggleTree,
    ToggleOverview,
    NarrowSidebar,
    WidenSidebar,
    /// `C-b ,` (task M6.10, decision 23): renames the focused window, as in tmux.
    RenameWindow,
    /// `C-b R` (task M6.10, decision 23): restarts the focused window.
    RestartWindow,
    /// `C-b r` (task M6.10, decision 23): reconnects while disconnected, or toasts
    /// `connected` while already connected.
    Reconnect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Write these bytes to the focused PTY.
    Send(Vec<u8>),
    Run(Command),
    /// The prefix was pressed; the next key picks a command.
    AwaitPrefix,
    /// Prefix mode was cancelled with Esc.
    Cancel,
    /// A key pressed in tree mode; `App` interprets it.
    Tree(KeyEvent),
    Nothing,
}

#[derive(Debug, Clone)]
pub struct Keymap {
    prefix: (KeyCode, KeyModifiers),
    pending: bool,
    tree_mode: bool,
}

impl Keymap {
    pub fn default_prefix() -> (KeyCode, KeyModifiers) {
        (KeyCode::Char('b'), KeyModifiers::CONTROL)
    }

    pub fn new(prefix: (KeyCode, KeyModifiers)) -> Self {
        Self {
            prefix,
            pending: false,
            tree_mode: false,
        }
    }

    pub fn pending(&self) -> bool {
        self.pending
    }

    pub fn set_tree_mode(&mut self, on: bool) {
        self.tree_mode = on;
    }

    pub fn tree_mode(&self) -> bool {
        self.tree_mode
    }

    fn is_prefix(&self, key: &KeyEvent) -> bool {
        key.code == self.prefix.0 && key.modifiers == self.prefix.1
    }

    pub fn handle(&mut self, key: KeyEvent, app_cursor: bool) -> KeyAction {
        if key.kind == KeyEventKind::Release {
            return KeyAction::Nothing;
        }
        if self.pending {
            if key.kind == KeyEventKind::Repeat && self.is_prefix(&key) {
                return KeyAction::AwaitPrefix;
            }
            self.pending = false;
            if self.is_prefix(&key) {
                return encode_key(KeyEvent::new(self.prefix.0, self.prefix.1), app_cursor)
                    .map(KeyAction::Send)
                    .unwrap_or(KeyAction::Nothing);
            }
            return match key.code {
                KeyCode::Char('j') | KeyCode::Char('n') => KeyAction::Run(Command::NextWindow),
                KeyCode::Char('k') | KeyCode::Char('p') => KeyAction::Run(Command::PrevWindow),
                KeyCode::Char(d @ '1'..='9') => {
                    KeyAction::Run(Command::FocusIndex(d as usize - '1' as usize))
                }
                KeyCode::Char('c') => KeyAction::Run(Command::NewWindow),
                KeyCode::Char('x') => KeyAction::Run(Command::KillWindow),
                KeyCode::Char('X') => KeyAction::Run(Command::RemoveWindow),
                KeyCode::Char('s') => KeyAction::Run(Command::ToggleSidebar),
                KeyCode::Char('t') => KeyAction::Run(Command::ToggleTree),
                KeyCode::Char('T') => KeyAction::Run(Command::ToggleOverview),
                KeyCode::Char('<') => KeyAction::Run(Command::NarrowSidebar),
                KeyCode::Char('>') => KeyAction::Run(Command::WidenSidebar),
                KeyCode::Char('d') => KeyAction::Run(Command::Detach),
                KeyCode::Char('Q') => KeyAction::Run(Command::StopDaemon),
                KeyCode::Char(',') => KeyAction::Run(Command::RenameWindow),
                KeyCode::Char('R') => KeyAction::Run(Command::RestartWindow),
                KeyCode::Char('r') => KeyAction::Run(Command::Reconnect),
                KeyCode::Char('?') => KeyAction::Run(Command::Help),
                KeyCode::Esc => KeyAction::Cancel,
                _ => KeyAction::Nothing,
            };
        }
        if self.is_prefix(&key) {
            self.pending = true;
            return KeyAction::AwaitPrefix;
        }
        if self.tree_mode {
            return KeyAction::Tree(key);
        }
        encode_key(key, app_cursor)
            .map(KeyAction::Send)
            .unwrap_or(KeyAction::Nothing)
    }
}

/// Encodes a key the way an xterm-compatible terminal would send it.
/// `app_cursor` is the DECCKM state of the receiving program (cursor keys send `ESC O x`).
pub fn encode_key(key: KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    use KeyCode::*;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);

    let cursor = |final_byte: char| -> Vec<u8> {
        if modifier == 1 {
            if app_cursor {
                format!("\x1bO{final_byte}")
            } else {
                format!("\x1b[{final_byte}")
            }
        } else {
            format!("\x1b[1;{modifier}{final_byte}")
        }
        .into_bytes()
    };
    let tilde = |code: u8| -> Vec<u8> {
        if modifier == 1 {
            format!("\x1b[{code}~")
        } else {
            format!("\x1b[{code};{modifier}~")
        }
        .into_bytes()
    };

    let bytes = match key.code {
        Char(c) => {
            let mut out = Vec::with_capacity(5);
            if alt {
                out.push(0x1b);
            }
            if ctrl {
                let byte = match c.to_ascii_lowercase() {
                    l @ 'a'..='z' => l as u8 - b'a' + 1,
                    ' ' | '@' | '2' => 0x00,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '7' | '/' => 0x1f,
                    '?' | '8' => 0x7f,
                    _ => return None,
                };
                out.push(byte);
            } else {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            out
        }
        // Alt prefixes these with ESC, the same way Alt+<char> does. Alt+Enter in
        // particular is how claude and codex take a newline without submitting, on
        // terminals that do not speak the kitty keyboard protocol.
        Enter => {
            if alt {
                vec![0x1b, b'\r']
            } else {
                vec![b'\r']
            }
        }
        Tab => {
            if alt {
                vec![0x1b, b'\t']
            } else {
                vec![b'\t']
            }
        }
        BackTab => b"\x1b[Z".to_vec(),
        Backspace => {
            if alt {
                vec![0x1b, 0x7f]
            } else {
                vec![0x7f]
            }
        }
        Esc => {
            if alt {
                vec![0x1b, 0x1b]
            } else {
                vec![0x1b]
            }
        }
        Up => cursor('A'),
        Down => cursor('B'),
        Right => cursor('C'),
        Left => cursor('D'),
        Home => cursor('H'),
        End => cursor('F'),
        Insert => tilde(2),
        Delete => tilde(3),
        PageUp => tilde(5),
        PageDown => tilde(6),
        F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => tilde(15),
            6 => tilde(17),
            7 => tilde(18),
            8 => tilde(19),
            9 => tilde(20),
            10 => tilde(21),
            11 => tilde(23),
            12 => tilde(24),
            _ => return None,
        },
        _ => return None,
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_and_control_characters() {
        assert_eq!(
            encode_key(key(KeyCode::Char('a'), KeyModifiers::NONE), false),
            Some(b"a".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('é'), KeyModifiers::NONE), false),
            Some("é".as_bytes().to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL), false),
            Some(vec![0x03])
        );
        assert_eq!(
            encode_key(
                key(
                    KeyCode::Char('C'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT
                ),
                false
            ),
            Some(vec![0x03])
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('['), KeyModifiers::CONTROL), false),
            Some(vec![0x1b])
        );
        assert_eq!(
            encode_key(key(KeyCode::Char(' '), KeyModifiers::CONTROL), false),
            Some(vec![0x00])
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('x'), KeyModifiers::ALT), false),
            Some(vec![0x1b, b'x'])
        );
        assert_eq!(
            encode_key(key(KeyCode::Enter, KeyModifiers::NONE), false),
            Some(vec![b'\r'])
        );
        assert_eq!(
            encode_key(key(KeyCode::Backspace, KeyModifiers::NONE), false),
            Some(vec![0x7f])
        );
        assert_eq!(
            encode_key(key(KeyCode::Esc, KeyModifiers::NONE), false),
            Some(vec![0x1b])
        );
        assert_eq!(
            encode_key(key(KeyCode::Tab, KeyModifiers::NONE), false),
            Some(vec![b'\t'])
        );
        assert_eq!(
            encode_key(key(KeyCode::BackTab, KeyModifiers::SHIFT), false),
            Some(b"\x1b[Z".to_vec())
        );
    }

    /// I3: Alt+Enter is how claude and codex insert a newline without submitting; it must
    /// reach them as ESC CR, not as a bare CR that submits the prompt.
    #[test]
    fn alt_prefixes_enter_tab_escape_and_backspace_with_escape() {
        assert_eq!(
            encode_key(key(KeyCode::Enter, KeyModifiers::ALT), false),
            Some(vec![0x1b, b'\r'])
        );
        assert_eq!(
            encode_key(key(KeyCode::Tab, KeyModifiers::ALT), false),
            Some(vec![0x1b, b'\t'])
        );
        assert_eq!(
            encode_key(key(KeyCode::Esc, KeyModifiers::ALT), false),
            Some(vec![0x1b, 0x1b])
        );
        assert_eq!(
            encode_key(key(KeyCode::Backspace, KeyModifiers::ALT), false),
            Some(vec![0x1b, 0x7f])
        );
        // Without Alt they are unchanged.
        assert_eq!(
            encode_key(key(KeyCode::Enter, KeyModifiers::NONE), false),
            Some(vec![b'\r'])
        );
        assert_eq!(
            encode_key(key(KeyCode::Tab, KeyModifiers::NONE), false),
            Some(vec![b'\t'])
        );
        assert_eq!(
            encode_key(key(KeyCode::Esc, KeyModifiers::NONE), false),
            Some(vec![0x1b])
        );
    }

    #[test]
    fn cursor_keys_honour_application_mode_and_modifiers() {
        assert_eq!(
            encode_key(key(KeyCode::Up, KeyModifiers::NONE), false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::Up, KeyModifiers::NONE), true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::Left, KeyModifiers::SHIFT), false),
            Some(b"\x1b[1;2D".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::Right, KeyModifiers::CONTROL), true),
            Some(b"\x1b[1;5C".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::Home, KeyModifiers::NONE), true),
            Some(b"\x1bOH".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::Delete, KeyModifiers::NONE), false),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::PageUp, KeyModifiers::SHIFT), false),
            Some(b"\x1b[5;2~".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::F(1), KeyModifiers::NONE), false),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::F(5), KeyModifiers::NONE), false),
            Some(b"\x1b[15~".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::F(12), KeyModifiers::NONE), false),
            Some(b"\x1b[24~".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::CapsLock, KeyModifiers::NONE), false),
            None
        );
    }

    #[test]
    fn prefix_then_command() {
        let mut km = Keymap::new(Keymap::default_prefix());
        let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert!(km.pending());
        assert_eq!(
            km.handle(key(KeyCode::Char('j'), KeyModifiers::NONE), false),
            KeyAction::Run(Command::NextWindow)
        );
        assert!(!km.pending());
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert_eq!(
            km.handle(key(KeyCode::Char('3'), KeyModifiers::NONE), false),
            KeyAction::Run(Command::FocusIndex(2))
        );
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert_eq!(
            km.handle(key(KeyCode::Char('X'), KeyModifiers::SHIFT), false),
            KeyAction::Run(Command::RemoveWindow)
        );
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert_eq!(
            km.handle(key(KeyCode::Char('?'), KeyModifiers::NONE), false),
            KeyAction::Run(Command::Help)
        );
    }

    #[test]
    fn prefix_twice_sends_literal_and_escape_cancels() {
        let mut km = Keymap::new(Keymap::default_prefix());
        let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        km.handle(prefix, false);
        assert_eq!(km.handle(prefix, false), KeyAction::Send(vec![0x02]));
        km.handle(prefix, false);
        assert_eq!(
            km.handle(key(KeyCode::Esc, KeyModifiers::NONE), false),
            KeyAction::Cancel
        );
        assert!(!km.pending());
        km.handle(prefix, false);
        assert_eq!(
            km.handle(key(KeyCode::Char('z'), KeyModifiers::NONE), false),
            KeyAction::Nothing
        );
    }

    #[test]
    fn ordinary_keys_pass_through_and_releases_are_ignored() {
        let mut km = Keymap::new(Keymap::default_prefix());
        assert_eq!(
            km.handle(key(KeyCode::Char('q'), KeyModifiers::NONE), false),
            KeyAction::Send(b"q".to_vec())
        );
        let mut release = key(KeyCode::Char('q'), KeyModifiers::NONE);
        release.kind = crossterm::event::KeyEventKind::Release;
        assert_eq!(km.handle(release, false), KeyAction::Nothing);
    }

    #[test]
    fn held_prefix_auto_repeat_keeps_waiting() {
        let mut km = Keymap::new(Keymap::default_prefix());
        let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert!(km.pending());
        let mut repeat = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        repeat.kind = crossterm::event::KeyEventKind::Repeat;
        assert_eq!(km.handle(repeat, false), KeyAction::AwaitPrefix);
        assert!(km.pending());
        assert_eq!(
            km.handle(key(KeyCode::Char('j'), KeyModifiers::NONE), false),
            KeyAction::Run(Command::NextWindow)
        );
        assert!(!km.pending());
    }

    #[test]
    fn prefix_opens_tree_overview_and_width_commands() {
        let mut km = Keymap::new(Keymap::default_prefix());
        let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);

        for (key, command) in [
            (
                key(KeyCode::Char('t'), KeyModifiers::NONE),
                Command::ToggleTree,
            ),
            (
                key(KeyCode::Char('T'), KeyModifiers::NONE),
                Command::ToggleOverview,
            ),
            (
                key(KeyCode::Char('T'), KeyModifiers::SHIFT),
                Command::ToggleOverview,
            ),
            (
                key(KeyCode::Char('<'), KeyModifiers::NONE),
                Command::NarrowSidebar,
            ),
            (
                key(KeyCode::Char('>'), KeyModifiers::NONE),
                Command::WidenSidebar,
            ),
        ] {
            assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
            assert_eq!(km.handle(key, false), KeyAction::Run(command));
        }
    }

    /// Task M6.10, decision 23: `,` renames, `R` restarts, `r` reconnects. `R` must
    /// match whether or not crossterm also reports the shift modifier (compare
    /// `prefix_opens_tree_overview_and_width_commands`'s `T` cases above).
    #[test]
    fn rename_restart_and_reconnect_keys() {
        let mut km = Keymap::new(Keymap::default_prefix());
        let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);

        for (k, command) in [
            (
                key(KeyCode::Char(','), KeyModifiers::NONE),
                Command::RenameWindow,
            ),
            (
                key(KeyCode::Char('R'), KeyModifiers::SHIFT),
                Command::RestartWindow,
            ),
            (
                key(KeyCode::Char('R'), KeyModifiers::NONE),
                Command::RestartWindow,
            ),
            (
                key(KeyCode::Char('r'), KeyModifiers::NONE),
                Command::Reconnect,
            ),
        ] {
            assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
            assert_eq!(km.handle(k, false), KeyAction::Run(command));
        }
    }

    #[test]
    fn tree_mode_returns_keys_instead_of_bytes() {
        let mut km = Keymap::new(Keymap::default_prefix());
        let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        km.set_tree_mode(true);

        for tree_key in [
            key(KeyCode::Char('j'), KeyModifiers::NONE),
            key(KeyCode::Enter, KeyModifiers::NONE),
            key(KeyCode::Esc, KeyModifiers::NONE),
            key(KeyCode::Char('/'), KeyModifiers::NONE),
        ] {
            assert_eq!(km.handle(tree_key, false), KeyAction::Tree(tree_key));
        }

        let mut release = key(KeyCode::Char('j'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(km.handle(release, false), KeyAction::Nothing);

        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert_eq!(
            km.handle(key(KeyCode::Char('j'), KeyModifiers::NONE), false),
            KeyAction::Run(Command::NextWindow)
        );
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert_eq!(km.handle(prefix, false), KeyAction::Send(vec![0x02]));

        km.set_tree_mode(false);
        assert_eq!(
            km.handle(key(KeyCode::Char('j'), KeyModifiers::NONE), false),
            KeyAction::Send(b"j".to_vec())
        );
    }
}
