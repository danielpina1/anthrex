//! Pure model for the new-agent form and the remove-confirm dialog: fields, focus,
//! text editing and validation. No I/O, no clock, no filesystem (`AGENTS.md` rule 5,
//! the same discipline `app.rs` and `tree_input.rs` follow). Rendering is
//! `crates/tui/src/ui/dialog.rs` (task M5.11); opening, submitting and closing the
//! form is wired in `app.rs` / `app/modal_keys.rs` (task M5.9).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{Runtime, WindowSpec};
use std::path::{Path, PathBuf};
use unicode_segmentation::UnicodeSegmentation;

/// The longest a submitted window name may be, after trimming: design decision 32's
/// limit. `pub` though nothing outside this file reads it today — the milestone brief's
/// interface block names it `pub const NAME_MAX_CHARS: usize` explicitly, the same way
/// `daemon::worktree::RESERVED_BRANCH_PREFIX` is public for a decision that owns it
/// rather than for a caller that exists yet.
pub const NAME_MAX_CHARS: usize = 64;

/// A single-line text field. The cursor is a grapheme-cluster index into `text`, not a
/// byte or `char` index: deleting or moving past a multi-`char` grapheme (an accent
/// applied by combination rather than precomposition, a flag, a ZWJ emoji sequence)
/// must act on the whole cluster, exactly as `tree_input.rs`'s filter already learned
/// backspace has to. Every named char in this milestone's tests is one grapheme, so
/// the distinction from a plain `char` count is invisible there, but the type stays
/// grapheme-based rather than narrowing back to `char` under a comment that says
/// otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    text: String,
    cursor: usize,
}

impl TextInput {
    /// Cursor starts at the end.
    pub fn new(text: &str) -> Self {
        let cursor = text.graphemes(true).count();
        Self {
            text: text.to_string(),
            cursor,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn len(&self) -> usize {
        self.text.graphemes(true).count()
    }

    /// Byte offset of the start of the `index`-th grapheme, or the text's length when
    /// `index` is at or past the end.
    fn byte_offset(&self, index: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .nth(index)
            .map(|(offset, _)| offset)
            .unwrap_or(self.text.len())
    }

    /// Inserts `s` at the cursor and leaves the cursor after it. `s` may itself hold
    /// several graphemes (a paste), so this is also what `on_paste` calls.
    pub fn insert(&mut self, s: &str) {
        let offset = self.byte_offset(self.cursor);
        self.text.insert_str(offset, s);
        let inserted_end = offset + s.len();
        // The new cursor is the grapheme index of the first real cluster boundary
        // at or after the byte offset where the inserted text ends. This is
        // computed directly from the resulting string's own cluster boundaries, not
        // from any kind of before/after count: a count (in isolation or as a delta)
        // cannot represent what a splice does, because merging with a neighbouring
        // grapheme can leave the count unchanged (this insertion's tail absorbed by
        // an orphan combining mark that follows it) or even decrease it (a ZWJ
        // fusing two previously separate emoji into one) — see the round-1 and
        // round-2 regression tests below for both.
        //
        // The common case is that `inserted_end` lands exactly on a boundary (the
        // splice didn't merge forward into what follows), and this is simply the
        // count of clusters up to and including the inserted text. When `s` merges
        // with a *following* grapheme it did not previously share a cluster with —
        // typing a base letter right in front of an already-present combining mark,
        // or a ZWJ fusing two previously-adjacent-but-separate emoji into one —
        // `inserted_end` lands inside that merged cluster instead, since there is no
        // boundary there to land on. We deliberately put the cursor just past the
        // whole merged cluster rather than before it: "the cursor moves forward
        // across what was just typed" stays true even when the typed text reaches
        // into text that was already there, and it's what keeps later keystrokes
        // from landing in front of earlier ones.
        self.cursor = self
            .text
            .grapheme_indices(true)
            .position(|(byte_idx, _)| byte_idx >= inserted_end)
            .unwrap_or_else(|| self.len());
    }

    /// Removes the grapheme before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self.byte_offset(self.cursor);
        let start = self.byte_offset(self.cursor - 1);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// Removes the grapheme at the cursor.
    pub fn delete(&mut self) {
        if self.cursor >= self.len() {
            return;
        }
        let start = self.byte_offset(self.cursor);
        let end = self.byte_offset(self.cursor + 1);
        self.text.replace_range(start..end, "");
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// The part of the text that fits in `width` columns with the cursor visible, and
    /// the cursor's column inside that slice. Counts graphemes, not display width: a
    /// wide (e.g. CJK) grapheme still counts as one column here, the same
    /// simplification the brief's doc comment calls "chars".
    ///
    /// The window always ends at or before the text's end and starts at or after 0,
    /// sliding by exactly as much as the cursor moves once it would otherwise leave
    /// the window — the same one-column-at-a-time scroll a single-line terminal input
    /// gives you.
    pub fn visible(&self, width: u16) -> (String, u16) {
        let width = width as usize;
        if width == 0 {
            return (String::new(), 0);
        }
        let graphemes: Vec<&str> = self.text.graphemes(true).collect();
        let len = graphemes.len();
        // Defence in depth: clamp the cursor into range here too, so a future bug
        // that leaves `self.cursor` past `len` still can't slice out of bounds.
        let cursor = self.cursor.min(len);
        let start = cursor.saturating_sub(width.saturating_sub(1));
        let end = (start + width).min(len);
        let visible: String = graphemes[start..end].concat();
        let column = (cursor - start) as u16;
        (visible, column)
    }
}

/// One field in the new-agent form, in the order decision 28 lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Runtime,
    Name,
    Directory,
    Worktree,
    Branch,
    Model,
    Prompt,
}

/// What a form opens with: the runtime, directory text and model of the last form the
/// daemon accepted this session, or the client's own first-run defaults (decision 31).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormDefaults {
    pub runtime: Runtime,
    pub dir: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAgentForm {
    pub runtime: Runtime,
    pub name: TextInput,
    pub dir: TextInput,
    pub worktree: bool,
    pub branch: TextInput,
    pub model: TextInput,
    pub prompt: TextInput,
    pub focus: FormField,
    pub error: Option<String>,
    pub submitting: bool,
}

/// Everything `validate` needs from the app, borrowed rather than cloned so this stays
/// a pure read.
pub struct FormContext<'a> {
    pub default_dir: &'a Path,
    pub home: Option<&'a Path>,
    pub existing_names: &'a [&'a str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormOutcome {
    /// Nothing worth telling the caller happened; the form keeps going.
    Stay,
    /// `Esc` or `Ctrl-C`: close the form, nothing is sent.
    Cancel,
    /// Enter validated cleanly. `submitting` is already set on `self`.
    Submit(WindowSpec),
}

fn next_runtime(runtime: Runtime) -> Runtime {
    match runtime {
        Runtime::Claude => Runtime::Codex,
        Runtime::Codex => Runtime::Shell,
        Runtime::Shell => Runtime::Claude,
    }
}

fn prev_runtime(runtime: Runtime) -> Runtime {
    match runtime {
        Runtime::Claude => Runtime::Shell,
        Runtime::Codex => Runtime::Claude,
        Runtime::Shell => Runtime::Codex,
    }
}

fn is_ctrl_c(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// A text field's own key handling: printable characters insert, the usual editing
/// keys move or erase, `Ctrl-U` clears. Shared by every `TextInput` field on the form
/// so the rules in decision 29's table are written exactly once, and by `app/prompt.rs`'s
/// rename box (task M6.10), whose one field wants the same editing keys.
pub(crate) fn apply_text_key(input: &mut TextInput, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Char('u') if ctrl => input.clear(),
        KeyCode::Char(c) if !ctrl && !alt => {
            let mut buf = [0u8; 4];
            input.insert(c.encode_utf8(&mut buf));
        }
        KeyCode::Backspace => input.backspace(),
        KeyCode::Delete => input.delete(),
        KeyCode::Left => input.left(),
        KeyCode::Right => input.right(),
        KeyCode::Home => input.home(),
        KeyCode::End => input.end(),
        _ => {}
    }
}

impl NewAgentForm {
    pub fn new(defaults: &FormDefaults) -> Self {
        Self {
            runtime: defaults.runtime,
            name: TextInput::default(),
            dir: TextInput::new(&defaults.dir),
            worktree: false,
            branch: TextInput::default(),
            model: TextInput::new(&defaults.model),
            prompt: TextInput::default(),
            focus: FormField::Runtime,
            error: None,
            submitting: false,
        }
    }

    /// The fields shown right now, in order: decision 28's base order, with `Branch`
    /// inserted after `Worktree` only while it is ticked, and `Model`/`Prompt` dropped
    /// entirely for `Runtime::Shell`.
    pub fn visible_fields(&self) -> Vec<FormField> {
        let mut fields = vec![
            FormField::Runtime,
            FormField::Name,
            FormField::Directory,
            FormField::Worktree,
        ];
        if self.worktree {
            fields.push(FormField::Branch);
        }
        if self.runtime != Runtime::Shell {
            fields.push(FormField::Model);
            fields.push(FormField::Prompt);
        }
        fields
    }

    fn move_focus(&mut self, delta: isize) {
        let fields = self.visible_fields();
        let current = fields.iter().position(|f| *f == self.focus).unwrap_or(0);
        let len = fields.len() as isize;
        let next = (current as isize + delta).rem_euclid(len);
        self.focus = fields[next as usize];
    }

    fn focused_text_input_mut(&mut self) -> Option<&mut TextInput> {
        match self.focus {
            FormField::Name => Some(&mut self.name),
            FormField::Directory => Some(&mut self.dir),
            FormField::Branch => Some(&mut self.branch),
            FormField::Model => Some(&mut self.model),
            FormField::Prompt => Some(&mut self.prompt),
            FormField::Runtime | FormField::Worktree => None,
        }
    }

    fn on_runtime_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Right | KeyCode::Char(' ') => self.runtime = next_runtime(self.runtime),
            KeyCode::Left => self.runtime = prev_runtime(self.runtime),
            KeyCode::Char('1') => self.runtime = Runtime::Claude,
            KeyCode::Char('2') => self.runtime = Runtime::Codex,
            KeyCode::Char('3') => self.runtime = Runtime::Shell,
            _ => {}
        }
    }

    fn on_worktree_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char(' ') {
            self.worktree = !self.worktree;
        }
    }

    /// Decision 29. Returns `Stay` for everything that only changes `self`, `Cancel`
    /// on `Esc`/`Ctrl-C`, and `Submit` on a validated `Enter` (which also sets
    /// `self.submitting`). While `self.submitting` is already true, only `Esc` and
    /// `Ctrl-C` do anything, exactly as the brief specifies.
    pub fn on_key(&mut self, key: KeyEvent, ctx: &FormContext<'_>) -> FormOutcome {
        if self.submitting {
            return if key.code == KeyCode::Esc || is_ctrl_c(&key) {
                FormOutcome::Cancel
            } else {
                FormOutcome::Stay
            };
        }
        if key.code == KeyCode::Esc || is_ctrl_c(&key) {
            return FormOutcome::Cancel;
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                self.move_focus(1);
                return FormOutcome::Stay;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.move_focus(-1);
                return FormOutcome::Stay;
            }
            KeyCode::Enter => {
                return match self.validate(ctx) {
                    Ok(spec) => {
                        self.submitting = true;
                        self.error = None;
                        FormOutcome::Submit(spec)
                    }
                    Err((field, message)) => {
                        self.focus = field;
                        self.error = Some(message);
                        FormOutcome::Stay
                    }
                };
            }
            _ => {}
        }
        match self.focus {
            FormField::Runtime => self.on_runtime_key(key),
            FormField::Worktree => self.on_worktree_key(key),
            _ => {
                if let Some(input) = self.focused_text_input_mut() {
                    apply_text_key(input, key);
                }
            }
        }
        FormOutcome::Stay
    }

    /// Decision 30: `\r\n`, `\r` and `\n` each become one space, so a paste can never
    /// corrupt a single-line field. Goes to the focused text field only; a paste while
    /// `Runtime` or `Worktree` is focused, or while the form is submitting, changes
    /// nothing.
    pub fn on_paste(&mut self, text: &str) {
        if self.submitting {
            return;
        }
        let sanitized = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', " ");
        if let Some(input) = self.focused_text_input_mut() {
            input.insert(&sanitized);
        }
    }

    /// Decision 32, checked in order: `Directory`, `Name`, `Branch` when ticked. The
    /// first failure is returned with the field that should take focus. `Model` and
    /// `Prompt` have no invalid state; hidden fields (a `Shell` model/prompt, an
    /// unticked branch) are dropped from the result but never block submission.
    pub fn validate(&self, ctx: &FormContext<'_>) -> Result<WindowSpec, (FormField, String)> {
        let cwd = expand_dir(self.dir.text(), ctx.default_dir, ctx.home)
            .map_err(|message| (FormField::Directory, message))?;

        let trimmed_name = self.name.text().trim();
        let name = if trimmed_name.is_empty() {
            None
        } else {
            if trimmed_name.chars().count() > NAME_MAX_CHARS {
                return Err((
                    FormField::Name,
                    format!("name must be at most {NAME_MAX_CHARS} characters"),
                ));
            }
            if trimmed_name.chars().any(|c| c.is_control()) {
                return Err((
                    FormField::Name,
                    "name must not contain control characters".to_string(),
                ));
            }
            if ctx.existing_names.contains(&trimmed_name) {
                return Err((
                    FormField::Name,
                    format!("a window named '{trimmed_name}' already exists"),
                ));
            }
            Some(trimmed_name.to_string())
        };

        let worktree_branch = if self.worktree {
            let branch = self.branch.text().trim();
            check_branch_syntax(branch).map_err(|message| (FormField::Branch, message))?;
            Some(branch.to_string())
        } else {
            None
        };

        let (model, initial_prompt) = if self.runtime == Runtime::Shell {
            (None, None)
        } else {
            let model = self.model.text().trim();
            let prompt = self.prompt.text().trim();
            (
                (!model.is_empty()).then(|| model.to_string()),
                (!prompt.is_empty()).then(|| prompt.to_string()),
            )
        };

        Ok(WindowSpec {
            name,
            runtime: self.runtime,
            cwd,
            worktree_branch,
            model,
            initial_prompt,
        })
    }

    /// What the next form should open with: this form's runtime, directory text and
    /// model, unparsed and unexpanded (decision 34 stores it back on `Created`).
    pub fn defaults(&self) -> FormDefaults {
        FormDefaults {
            runtime: self.runtime,
            dir: self.dir.text().to_string(),
            model: self.model.text().to_string(),
        }
    }
}

/// Decision 32's directory rule. `~` and `~/...` expand against `home`; any other
/// leading `~` is rejected rather than guessed at. A relative path is joined onto
/// `default_dir`; an absolute path is unchanged.
pub fn expand_dir(input: &str, default_dir: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("directory is required".to_string());
    }
    if trimmed == "~" || trimmed.starts_with("~/") {
        let home = home.ok_or_else(|| "cannot expand ~: home directory unknown".to_string())?;
        return Ok(if trimmed == "~" {
            home.to_path_buf()
        } else {
            // `PathBuf::join` replaces rather than appends when its argument looks
            // absolute, so a doubled slash right after `~/` (e.g. `~//etc/passwd`)
            // would otherwise silently drop `home` entirely. The remainder of a `~/`
            // path is always relative to home, so strip any extra leading slashes
            // first.
            home.join(trimmed[2..].trim_start_matches('/'))
        });
    }
    if trimmed.starts_with('~') {
        return Err("only ~ and ~/ are expanded".to_string());
    }
    let path = Path::new(trimmed);
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        default_dir.join(path)
    })
}

/// Decision 8's first four branch rules — the ones the client can check without
/// talking to git. `crates/daemon/src/worktree.rs` has the authoritative copy that
/// also runs `check-ref-format`; the messages here match it exactly so a client
/// rejection and a daemon rejection never read differently for the same input.
pub fn check_branch_syntax(branch: &str) -> Result<(), String> {
    if branch.trim().is_empty() {
        return Err("branch name is required".to_string());
    }
    if branch.chars().any(char::is_whitespace) {
        return Err("branch name cannot contain spaces".to_string());
    }
    if branch.starts_with('-') {
        return Err("branch name cannot start with '-'".to_string());
    }
    if branch.starts_with("anthrex/") {
        return Err("branches under anthrex/ are reserved for orchestration runs".to_string());
    }
    Ok(())
}

/// The remove-confirm dialog's data. Its key handling lives in
/// `crates/tui/src/app/modal_keys.rs` (task M5.9), alongside the rest of `Modal`'s
/// routing; this stays a plain data holder like the brief asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveConfirm {
    pub window_id: u32,
    pub name: String,
    pub branch: Option<String>,
    pub remove_worktree: bool,
}

#[cfg(test)]
#[path = "dialog_tests.rs"]
mod tests;
