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

/// Re-review, round 2: a combining mark inserted at cursor 0 in front of an
/// already-present *orphan* combining mark (one with no preceding base character,
/// so it is its own grapheme cluster) left the cursor stuck at 0, because the
/// round-1 fix measured the cursor advance as the change in the whole string's
/// grapheme count — and merging "e" into "\u{0301}" doesn't change that count (3
/// graphemes before, 3 after). Once the cursor is stuck, every following keystroke
/// splices at the same byte offset, silently reordering what the user typed.
#[test]
fn insert_before_an_orphan_combining_mark_does_not_reorder_later_keystrokes() {
    let mut t = TextInput::new("\u{0301}bc");
    t.home();
    t.insert("e");
    assert_eq!(
        t.text(),
        "e\u{0301}bc",
        "e merges forward with the orphan mark into one grapheme"
    );
    assert_eq!(
        t.cursor(),
        1,
        "cursor must advance past what was just typed"
    );

    t.insert("X");
    t.insert("Y");
    assert_eq!(
        t.text(),
        "e\u{0301}XYbc",
        "later keystrokes land after earlier ones, not reordered in front of them"
    );
}

/// Re-review, round 2: a ZWJ inserted between two emoji that were each inserted
/// separately (and so were, until now, two distinct grapheme clusters) fuses them
/// into a single cluster. The grapheme count therefore *decreases* on this insert,
/// which the round-1 fix's `after - before` (both `usize`) cannot represent: it
/// panicked with "attempt to subtract with overflow" at the old `dialog.rs:73`.
#[test]
fn zwj_fusing_two_separately_inserted_emoji_does_not_panic() {
    let mut t = TextInput::new("");
    t.insert("\u{1F468}");
    t.insert("\u{1F469}");
    assert_eq!(t.cursor(), 2, "two separate emoji, two graphemes");
    t.left();
    t.insert("\u{200D}");
    assert_eq!(t.text(), "\u{1F468}\u{200D}\u{1F469}");
    assert_eq!(
        t.cursor(),
        1,
        "the ZWJ fuses the pair into a single grapheme"
    );
    let (visible, column) = t.visible(3);
    assert_eq!(visible, "\u{1F468}\u{200D}\u{1F469}");
    assert_eq!(column, 1);
}

/// Both prior rounds fixed a specific reported input and were then broken by a
/// different one neither of us had thought to try. Rather than add a fourth named
/// case, this drives `insert` through every ordered sequence (repetition allowed)
/// of a small alphabet of grapheme-boundary troublemakers, up to length 3, from a
/// handful of starting contexts, and checks after *every single insert* the
/// invariants a correct `insert` can never violate:
///
/// - the cursor never exceeds `len()`
/// - `visible()` does not panic at any width, including 0 and 1
/// - each insertion happens at or after the byte offset where the previous one
///   ended — since `insert_str` only ever splices bytes in, never reorders or
///   deletes existing ones, this is the necessary and sufficient condition for
///   "nothing typed lands in front of something typed earlier" (the reordering
///   bug); checked unconditionally, in every context
/// - in contexts where the text after the cursor cannot itself reach backward and
///   absorb what gets typed (i.e. it doesn't start with a combining mark or ZWJ),
///   the *stronger* and more direct check also holds: the resulting string is
///   exactly the untouched prefix, then the typed pieces concatenated in order,
///   then the untouched suffix. ("Before an orphan mark" is deliberately excluded
///   from this stronger check: there, the first typed piece legitimately absorbs
///   the pre-existing mark into its own cluster — exactly the documented, correct
///   behaviour `insert_before_an_orphan_combining_mark_does_not_reorder_later_keystrokes`
///   pins down by hand — so the untouched-suffix assumption doesn't apply, even
///   though the weaker offset-monotonicity check above still does.)
#[test]
fn insert_exhaustive_combinations_never_reorder_or_go_out_of_range() {
    const ALPHABET: [&str; 6] = [
        "a",         // a plain ASCII letter
        "e",         // a base letter
        "\u{0301}",  // a lone combining acute
        "\u{200D}",  // a ZWJ
        "\u{1F1FA}", // a regional indicator
        "\u{1F600}", // an emoji
    ];

    struct Context {
        name: &'static str,
        base: &'static str,
        cursor: usize,
        /// Whether the untouched-prefix/typed/untouched-suffix equality is
        /// expected to hold here. False only where the suffix can legitimately
        /// reach backward and absorb the first typed piece.
        strict: bool,
    }

    let end_of = |base: &str| base.graphemes(true).count();
    let contexts = [
        Context {
            name: "empty",
            base: "",
            cursor: 0,
            strict: true,
        },
        Context {
            name: "start of plain text",
            base: "bc",
            cursor: 0,
            strict: true,
        },
        Context {
            name: "end of plain text",
            base: "bc",
            cursor: end_of("bc"),
            strict: true,
        },
        Context {
            name: "before an orphan mark",
            base: "\u{0301}xy",
            cursor: 0,
            strict: false,
        },
        Context {
            name: "after a trailing mark",
            base: "xy\u{0301}",
            cursor: end_of("xy\u{0301}"),
            strict: true,
        },
    ];

    // Every ordered sequence (with repetition) of `ALPHABET`, lengths 1..=3.
    fn sequences(alphabet: &[&'static str], max_len: usize) -> Vec<Vec<&'static str>> {
        let mut out = Vec::new();
        let mut stack: Vec<Vec<&'static str>> = vec![Vec::new()];
        while let Some(seq) = stack.pop() {
            if !seq.is_empty() {
                out.push(seq.clone());
            }
            if seq.len() < max_len {
                for piece in alphabet {
                    let mut next = seq.clone();
                    next.push(*piece);
                    stack.push(next);
                }
            }
        }
        out
    }

    let mut cases = 0;
    for ctx in &contexts {
        for seq in sequences(&ALPHABET, 3) {
            cases += 1;
            let mut t = TextInput::new(ctx.base);
            t.cursor = ctx.cursor;
            let split = t.byte_offset(ctx.cursor);
            let prefix = ctx.base[..split].to_string();
            let suffix = ctx.base[split..].to_string();
            let mut min_next_offset = split;

            let mut typed_so_far = String::new();
            for piece in &seq {
                let offset = t.byte_offset(t.cursor());
                assert!(
                    offset >= min_next_offset,
                    "{}: {:?} spliced {:?} at byte {offset}, before byte {min_next_offset} \
                     where the previous piece ended — later input landed in front of \
                     earlier input",
                    ctx.name,
                    seq,
                    piece
                );

                t.insert(piece);
                typed_so_far.push_str(piece);
                min_next_offset = offset + piece.len();

                let len = t.text().graphemes(true).count();
                assert!(
                    t.cursor() <= len,
                    "{}: {:?} left the cursor ({}) past len() ({}) on {:?}",
                    ctx.name,
                    seq,
                    t.cursor(),
                    len,
                    t.text()
                );
                for width in [0u16, 1, 2, 5, 20] {
                    t.visible(width); // must not panic
                }
                if ctx.strict {
                    let expected = format!("{prefix}{typed_so_far}{suffix}");
                    assert_eq!(
                        t.text(),
                        expected,
                        "{}: {:?} reordered the typed pieces",
                        ctx.name,
                        seq
                    );
                }
            }
        }
    }
    assert!(
        cases > 100,
        "sanity: expected a few hundred cases, got {cases}"
    );
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
