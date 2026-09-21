//! Pure model for the new-agent form and the remove-confirm dialog: fields, focus,
//! text editing and validation. No I/O, no clock, no filesystem (`AGENTS.md` rule 5,
//! the same discipline `app.rs` and `tree_input.rs` follow). Rendering is
//! `crates/tui/src/ui/dialog.rs` (task M5.11); opening, submitting and closing the
//! form is wired in `app.rs` / `app/modal_keys.rs` (task M5.9).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{Runtime, WindowSpec};
use std::path::{Path, PathBuf};
use unicode_segmentation::UnicodeSegmentation;

/// The longest a submitted window name may be, after trimming.
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
        let before = self.len();
        self.text.insert_str(offset, s);
        let after = self.len();
        // The cursor advances by the actual change in the whole string's grapheme
        // count, not by `s`'s own grapheme count in isolation: splicing `s` in can
        // merge with a grapheme on either side of the cursor (a combining mark
        // landing on the preceding base letter, a regional-indicator pair closing
        // into one flag), so the isolated count can overshoot the real advance.
        self.cursor += after - before;
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
/// so the rules in decision 29's table are written exactly once.
fn apply_text_key(input: &mut TextInput, key: KeyEvent) {
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
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctx() -> FormContext<'static> {
        FormContext {
            default_dir: Path::new("/work"),
            home: Some(Path::new("/home/me")),
            existing_names: &["api"],
        }
    }

    fn defaults(runtime: Runtime) -> FormDefaults {
        FormDefaults {
            runtime,
            dir: "/work".to_string(),
            model: String::new(),
        }
    }

    fn form(runtime: Runtime) -> NewAgentForm {
        NewAgentForm::new(&defaults(runtime))
    }

    #[test]
    fn text_input_edits_at_the_cursor() {
        let mut input = TextInput::new("");
        input.insert("helo");
        input.left();
        input.insert("l");
        assert_eq!(input.text(), "hello");
        assert_eq!(input.cursor(), 4);

        input.home();
        input.delete();
        assert_eq!(input.text(), "ello");

        input.end();
        input.backspace();
        assert_eq!(input.text(), "ell");

        let mut multi = TextInput::new("é日");
        assert_eq!(multi.cursor(), 2);
        multi.left();
        assert_eq!(multi.cursor(), 1);
        multi.left();
        assert_eq!(multi.cursor(), 0);
        multi.left(); // clamped: already at 0
        assert_eq!(multi.cursor(), 0);
        multi.right();
        multi.right();
        multi.right(); // clamped: already at the end
        assert_eq!(multi.cursor(), 2);
        multi.backspace();
        assert_eq!(multi.text(), "é");

        let mut cleared = TextInput::new("xyz");
        cleared.clear();
        assert_eq!(cleared.text(), "");
        assert_eq!(cleared.cursor(), 0);

        // "e" + a combining acute accent (U+0301) is two `char`s but one grapheme
        // cluster ("é" decomposed, as opposed to the precomposed "é" above). A
        // char-based or byte-based cursor would stop, or delete, in the middle of
        // this cluster; a grapheme-based one treats it as a single unit, the same
        // fix `tree_input.rs`'s filter needed for its own backspace.
        let mut combining = TextInput::new("e\u{0301}bc");
        assert_eq!(combining.cursor(), 3); // 3 graphemes: "é", "b", "c"
        combining.left();
        combining.left();
        assert_eq!(combining.cursor(), 1);
        combining.backspace();
        assert_eq!(combining.text(), "bc");
        assert_eq!(combining.cursor(), 0);
    }

    #[test]
    fn text_input_visible_keeps_the_cursor_in_view() {
        let text = "abcdefghijklmnopqrstuvwxyz1234";
        assert_eq!(text.chars().count(), 30);

        let at_end = TextInput::new(text);
        let (visible, column) = at_end.visible(10);
        assert_eq!(visible, "vwxyz1234");
        assert_eq!(column, 9);

        let mut at_start = TextInput::new(text);
        at_start.home();
        let (visible, column) = at_start.visible(10);
        assert_eq!(visible, "abcdefghij");
        assert_eq!(column, 0);
    }

    #[test]
    fn fields_follow_runtime_and_worktree() {
        let mut f = form(Runtime::Claude);
        assert_eq!(
            f.visible_fields(),
            vec![
                FormField::Runtime,
                FormField::Name,
                FormField::Directory,
                FormField::Worktree,
                FormField::Model,
                FormField::Prompt,
            ]
        );

        f.worktree = true;
        assert_eq!(
            f.visible_fields(),
            vec![
                FormField::Runtime,
                FormField::Name,
                FormField::Directory,
                FormField::Worktree,
                FormField::Branch,
                FormField::Model,
                FormField::Prompt,
            ]
        );

        f.runtime = Runtime::Shell;
        assert_eq!(
            f.visible_fields(),
            vec![
                FormField::Runtime,
                FormField::Name,
                FormField::Directory,
                FormField::Worktree,
                FormField::Branch,
            ]
        );
    }

    #[test]
    fn tab_and_shift_tab_wrap_over_visible_fields() {
        let ctx = ctx();

        let mut f = form(Runtime::Claude);
        assert_eq!(f.focus, FormField::Runtime);
        f.on_key(key(KeyCode::BackTab), &ctx);
        assert_eq!(f.focus, FormField::Prompt);

        let mut shell = form(Runtime::Shell);
        shell.focus = FormField::Worktree;
        shell.on_key(key(KeyCode::Tab), &ctx);
        assert_eq!(shell.focus, FormField::Runtime);

        let mut updown = form(Runtime::Claude);
        updown.on_key(key(KeyCode::Up), &ctx);
        assert_eq!(updown.focus, FormField::Prompt);
        updown.on_key(key(KeyCode::Down), &ctx);
        assert_eq!(updown.focus, FormField::Runtime);
    }

    #[test]
    fn runtime_field_keys() {
        let ctx = ctx();
        let mut f = form(Runtime::Claude);
        assert_eq!(f.focus, FormField::Runtime);

        f.on_key(key(KeyCode::Right), &ctx);
        assert_eq!(f.runtime, Runtime::Codex);
        f.on_key(key(KeyCode::Right), &ctx);
        assert_eq!(f.runtime, Runtime::Shell);
        f.on_key(key(KeyCode::Right), &ctx);
        assert_eq!(f.runtime, Runtime::Claude);

        f.on_key(key(KeyCode::Left), &ctx);
        assert_eq!(f.runtime, Runtime::Shell);
        f.on_key(key(KeyCode::Left), &ctx);
        assert_eq!(f.runtime, Runtime::Codex);

        f.on_key(key(KeyCode::Char(' ')), &ctx);
        assert_eq!(f.runtime, Runtime::Shell);

        f.on_key(key(KeyCode::Char('1')), &ctx);
        assert_eq!(f.runtime, Runtime::Claude);
        f.on_key(key(KeyCode::Char('2')), &ctx);
        assert_eq!(f.runtime, Runtime::Codex);
        f.on_key(key(KeyCode::Char('3')), &ctx);
        assert_eq!(f.runtime, Runtime::Shell);

        f.on_key(key(KeyCode::Char('z')), &ctx);
        assert_eq!(f.runtime, Runtime::Shell);
    }

    #[test]
    fn space_toggles_the_worktree_only_on_its_field() {
        let ctx = ctx();
        let mut f = form(Runtime::Claude);
        f.focus = FormField::Worktree;
        assert!(!f.worktree);
        f.on_key(key(KeyCode::Char(' ')), &ctx);
        assert!(f.worktree);
        f.on_key(key(KeyCode::Char(' ')), &ctx);
        assert!(!f.worktree);

        f.focus = FormField::Name;
        f.on_key(key(KeyCode::Char(' ')), &ctx);
        assert_eq!(f.name.text(), " ");
    }

    #[test]
    fn escape_and_ctrl_c_cancel() {
        let ctx = ctx();
        let mut f = form(Runtime::Claude);
        assert_eq!(f.on_key(key(KeyCode::Esc), &ctx), FormOutcome::Cancel);

        let mut f2 = form(Runtime::Claude);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(f2.on_key(ctrl_c, &ctx), FormOutcome::Cancel);
    }

    #[test]
    fn enter_submits_a_full_spec() {
        let ctx = ctx();
        let mut f = form(Runtime::Claude);
        f.runtime = Runtime::Codex;
        f.name = TextInput::new("  api-2 ");
        f.dir = TextInput::new("~/repos/shop");
        f.worktree = true;
        f.branch = TextInput::new("feat/x");
        f.model = TextInput::new("gpt-5-codex");
        f.prompt = TextInput::new("hi");

        let outcome = f.on_key(key(KeyCode::Enter), &ctx);
        assert_eq!(
            outcome,
            FormOutcome::Submit(WindowSpec {
                name: Some("api-2".to_string()),
                runtime: Runtime::Codex,
                cwd: PathBuf::from("/home/me/repos/shop"),
                worktree_branch: Some("feat/x".to_string()),
                model: Some("gpt-5-codex".to_string()),
                initial_prompt: Some("hi".to_string()),
            })
        );
        assert!(f.submitting);
    }

    #[test]
    fn shell_drops_hidden_fields() {
        let ctx = ctx();
        let mut shell = form(Runtime::Shell);
        shell.dir = TextInput::new("/work/proj");
        shell.model = TextInput::new("opus");
        shell.prompt = TextInput::new("hello");
        let spec = shell.validate(&ctx).unwrap();
        assert_eq!(spec.model, None);
        assert_eq!(spec.initial_prompt, None);

        let mut unticked = form(Runtime::Claude);
        unticked.dir = TextInput::new("/work/proj");
        unticked.branch = TextInput::new("feat/x");
        let spec = unticked.validate(&ctx).unwrap();
        assert_eq!(spec.worktree_branch, None);
    }

    #[test]
    fn validation_errors_focus_the_field() {
        let ctx = ctx();

        let mut f = form(Runtime::Claude);
        f.dir = TextInput::new("");
        assert_eq!(f.on_key(key(KeyCode::Enter), &ctx), FormOutcome::Stay);
        assert_eq!(f.focus, FormField::Directory);
        assert_eq!(f.error.as_deref(), Some("directory is required"));
        assert!(!f.submitting);

        let mut f = form(Runtime::Claude);
        f.dir = TextInput::new("~bob/x");
        assert_eq!(f.on_key(key(KeyCode::Enter), &ctx), FormOutcome::Stay);
        assert_eq!(f.focus, FormField::Directory);
        assert_eq!(f.error.as_deref(), Some("only ~ and ~/ are expanded"));

        let mut f = form(Runtime::Claude);
        f.dir = TextInput::new("/work/proj");
        f.name = TextInput::new("api");
        assert_eq!(f.on_key(key(KeyCode::Enter), &ctx), FormOutcome::Stay);
        assert_eq!(f.focus, FormField::Name);
        assert_eq!(
            f.error.as_deref(),
            Some("a window named 'api' already exists")
        );

        let mut f = form(Runtime::Claude);
        f.dir = TextInput::new("/work/proj");
        f.name = TextInput::new(&"a".repeat(65));
        assert_eq!(f.on_key(key(KeyCode::Enter), &ctx), FormOutcome::Stay);
        assert_eq!(f.focus, FormField::Name);
        assert_eq!(
            f.error.as_deref(),
            Some("name must be at most 64 characters")
        );

        let mut f = form(Runtime::Claude);
        f.dir = TextInput::new("/work/proj");
        f.worktree = true;
        f.branch = TextInput::new("");
        assert_eq!(f.on_key(key(KeyCode::Enter), &ctx), FormOutcome::Stay);
        assert_eq!(f.focus, FormField::Branch);
        assert_eq!(f.error.as_deref(), Some("branch name is required"));

        let mut f = form(Runtime::Claude);
        f.dir = TextInput::new("/work/proj");
        f.worktree = true;
        f.branch = TextInput::new("anthrex/x");
        assert_eq!(f.on_key(key(KeyCode::Enter), &ctx), FormOutcome::Stay);
        assert_eq!(f.focus, FormField::Branch);
        assert_eq!(
            f.error.as_deref(),
            Some("branches under anthrex/ are reserved for orchestration runs")
        );

        // None of the six cases above submitted anything.
        assert!(!f.submitting);
    }

    #[test]
    fn expand_dir_cases() {
        let default_dir = Path::new("/work");
        let home = Some(Path::new("/home/me"));
        assert_eq!(
            expand_dir("~", default_dir, home).unwrap(),
            PathBuf::from("/home/me")
        );
        assert_eq!(
            expand_dir("~/a", default_dir, home).unwrap(),
            PathBuf::from("/home/me/a")
        );
        assert_eq!(
            expand_dir("rel/x", default_dir, home).unwrap(),
            PathBuf::from("/work/rel/x")
        );
        assert_eq!(
            expand_dir("/abs", default_dir, home).unwrap(),
            PathBuf::from("/abs")
        );
        assert_eq!(
            expand_dir("~", default_dir, None).unwrap_err(),
            "cannot expand ~: home directory unknown"
        );
    }

    /// Finding 2 (Major): `home.join(remainder)` must never let the remainder replace
    /// `home` outright, which `PathBuf::join` does whenever its argument looks
    /// absolute — a doubled slash right after `~/` produces exactly that.
    #[test]
    fn expand_dir_keeps_the_home_prefix_even_with_a_doubled_slash() {
        let default_dir = Path::new("/work");
        let home = Some(Path::new("/home/me"));

        assert_eq!(
            expand_dir("~//etc/passwd", default_dir, home).unwrap(),
            PathBuf::from("/home/me/etc/passwd"),
            "a doubled slash after ~/ must not drop the home prefix"
        );
        assert_eq!(
            expand_dir("~///a", default_dir, home).unwrap(),
            PathBuf::from("/home/me/a"),
            "any number of extra leading slashes must still resolve under home"
        );
        assert_eq!(
            expand_dir("~/", default_dir, home).unwrap(),
            PathBuf::from("/home/me"),
            "a bare ~/ with nothing after it is just home"
        );
        assert_eq!(
            expand_dir("~", default_dir, home).unwrap(),
            PathBuf::from("/home/me"),
            "a bare ~ is unaffected by this fix"
        );
        assert_eq!(
            expand_dir("~/./a", default_dir, home).unwrap(),
            PathBuf::from("/home/me/./a"),
            "a remainder with a leading ./ is relative already and joins normally"
        );
    }

    #[test]
    fn submitting_ignores_everything_but_cancel() {
        let ctx = ctx();
        let mut f = form(Runtime::Claude);
        f.dir = TextInput::new("/work/proj");
        let outcome = f.on_key(key(KeyCode::Enter), &ctx);
        assert!(matches!(outcome, FormOutcome::Submit(_)));
        assert!(f.submitting);

        let before = f.clone();
        assert_eq!(f.on_key(key(KeyCode::Tab), &ctx), FormOutcome::Stay);
        assert_eq!(f, before);

        assert_eq!(f.on_key(key(KeyCode::Char('x')), &ctx), FormOutcome::Stay);
        assert_eq!(f, before);

        assert_eq!(f.on_key(key(KeyCode::Esc), &ctx), FormOutcome::Cancel);
    }

    /// Finding 1 (Critical): `insert` must derive the cursor advance from the actual
    /// change in the whole string's grapheme count, not from the inserted fragment's
    /// own count in isolation — a combining mark spliced onto an existing base letter
    /// merges into one grapheme, so the naive count overshoots and leaves the cursor
    /// past `len()`, which then made `visible()` panic. Every case here builds the
    /// string incrementally via `insert`, since that is the path a real keystroke or
    /// paste takes and the path the original 13 tests never exercised.
    #[test]
    fn insert_across_a_grapheme_boundary_keeps_the_cursor_in_range() {
        // A combining mark inserted right after the base letter it attaches to, at
        // the end of the string — the review's exact repro. `visible(1)` used to
        // panic here; now it must not, and its result must be internally consistent.
        let mut t = TextInput::new("cafe");
        t.insert("\u{0301}");
        assert_eq!(t.text(), "cafe\u{0301}");
        assert_eq!(
            t.cursor(),
            4,
            "4 graphemes: c, a, f, e-with-combining-acute"
        );
        let (visible, column) = t.visible(1);
        assert_eq!(visible, "");
        assert_eq!(column, 0);
        let (visible, column) = t.visible(2);
        assert_eq!(visible, "e\u{0301}");
        assert_eq!(column, 1);

        // The same merge, but in the middle of the string rather than at the end.
        let mut mid = TextInput::new("caferolls");
        for _ in 0..5 {
            mid.left(); // cursor after "cafe" (index 4), before "rolls"
        }
        assert_eq!(mid.cursor(), 4);
        mid.insert("\u{0301}");
        assert_eq!(mid.text(), "cafe\u{0301}rolls");
        assert_eq!(
            mid.cursor(),
            4,
            "cursor lands right after the merged é, not past it"
        );
        let (visible, column) = mid.visible(3);
        assert_eq!(visible, "fe\u{0301}r");
        assert_eq!(column, 2);

        // A wide cluster (a regional-indicator flag) built by two separate inserts:
        // each is its own grapheme alone, but together they form one.
        let mut flag = TextInput::new("");
        flag.insert("\u{1F1FA}");
        assert_eq!(flag.cursor(), 1);
        assert_eq!(flag.visible(5), ("\u{1F1FA}".to_string(), 1));
        flag.insert("\u{1F1F8}");
        assert_eq!(flag.text(), "\u{1F1FA}\u{1F1F8}");
        assert_eq!(
            flag.cursor(),
            1,
            "the pair merges into a single flag grapheme"
        );
        assert_eq!(flag.visible(5), ("\u{1F1FA}\u{1F1F8}".to_string(), 1));
    }

    #[test]
    fn visible_clamps_a_cursor_past_the_end_instead_of_panicking() {
        // Defence in depth per the review: even if some future bug leaves the cursor
        // past `len()`, `visible()` must not slice out of bounds.
        let mut t = TextInput::new("cafe");
        t.cursor = 99;
        let (visible, column) = t.visible(2);
        assert_eq!(visible, "e");
        assert_eq!(column, 1);
    }

    #[test]
    fn delete_removes_at_the_cursor_not_only_at_index_zero() {
        // Finding 3 (Minor): the only required-test call to `delete` is immediately
        // after `home()`, so a mutation hardcoding index 0 would still pass. Cover a
        // middle cursor and end-of-string (a no-op).
        let mut mid = TextInput::new("hello");
        mid.left();
        mid.left();
        mid.left(); // cursor 2, on the first "l"
        mid.delete();
        assert_eq!(mid.text(), "helo");
        assert_eq!(mid.cursor(), 2);

        let mut at_end = TextInput::new("hello");
        at_end.end();
        at_end.delete();
        assert_eq!(at_end.text(), "hello", "delete at end-of-string is a no-op");
        assert_eq!(at_end.cursor(), 5);
    }

    #[test]
    fn paste_replaces_newlines_with_spaces() {
        let mut f = form(Runtime::Claude);
        f.focus = FormField::Prompt;
        f.on_paste("a\r\nb\nc");
        assert_eq!(f.prompt.text(), "a b c");

        let untouched = form(Runtime::Claude);
        let mut on_runtime = form(Runtime::Claude);
        on_runtime.on_paste("x\ny");
        assert_eq!(on_runtime, untouched);
    }
}
