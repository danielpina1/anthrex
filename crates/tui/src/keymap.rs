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
    /// `C-b m` (task M6.5.12, spec §6): opens or closes the conversation view.
    ToggleConversation,
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
    /// A bare key pressed while the conversation view is open; the view interprets it.
    Conversation(KeyEvent),
    Nothing,
}

#[derive(Debug, Clone)]
pub struct Keymap {
    prefix: (KeyCode, KeyModifiers),
    pending: bool,
    tree_mode: bool,
    conversation_mode: bool,
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
            conversation_mode: false,
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

    /// Mirrors tree mode: while set, bare keys go to the conversation view instead of
    /// the PTY. The prefix still works.
    pub fn set_conversation_mode(&mut self, on: bool) {
        self.conversation_mode = on;
    }

    pub fn conversation_mode(&self) -> bool {
        self.conversation_mode
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
                KeyCode::Char('m') => KeyAction::Run(Command::ToggleConversation),
                KeyCode::Esc => KeyAction::Cancel,
                _ => KeyAction::Nothing,
            };
        }
        if self.is_prefix(&key) {
            self.pending = true;
            return KeyAction::AwaitPrefix;
        }
        if self.conversation_mode {
            return KeyAction::Conversation(key);
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
#[path = "keymap_tests.rs"]
mod tests;
