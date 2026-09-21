use super::*;
use crate::dialog::{FormDefaults, NewAgentForm, TextInput};
use proto::Runtime;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::widgets::Block;

fn defaults() -> FormDefaults {
    FormDefaults {
        runtime: Runtime::Claude,
        dir: "/work".into(),
        model: String::new(),
    }
}

fn draw(width: u16, height: u16, paint: impl FnOnce(&mut ratatui::Frame)) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| paint(frame)).unwrap();
    terminal.backend().to_string()
}

#[test]
fn new_agent_form_renders_fields_and_error() {
    let mut form = NewAgentForm::new(&defaults());
    form.worktree = true;
    form.branch = TextInput::new("feat/x");
    form.error = Some("not a git repository: /x".into());
    let out = draw(100, 30, |frame| {
        render_new_agent(frame, &form, frame.area(), theme::DEFAULT_ACCENT);
    });
    for expected in [
        "new agent",
        "Runtime",
        "[claude]",
        "Branch",
        "feat/x",
        "not a git repository: /x",
        "Enter create",
    ] {
        assert!(out.contains(expected), "missing {expected:?} in:\n{out}");
    }

    form.runtime = Runtime::Shell;
    let out = draw(100, 30, |frame| {
        render_new_agent(frame, &form, frame.area(), theme::DEFAULT_ACCENT);
    });
    assert!(!out.contains("Model"), "{out}");
    assert!(!out.contains("Prompt"), "{out}");

    form.submitting = true;
    let out = draw(100, 30, |frame| {
        render_new_agent(frame, &form, frame.area(), theme::DEFAULT_ACCENT);
    });
    assert!(out.contains("creating the worktree"), "{out}");
}

#[test]
fn new_agent_form_places_the_cursor_in_the_focused_field() {
    let mut form = NewAgentForm::new(&defaults());
    form.focus = FormField::Name;
    form.name = TextInput::new("ab");
    let area = Rect::new(0, 0, 100, 30);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| render_new_agent(frame, &form, frame.area(), theme::DEFAULT_ACCENT))
        .unwrap();
    let cursor = terminal.get_cursor_position().unwrap();

    let width = 66u16.min(area.width.saturating_sub(2)).max(4);
    let fields = form.visible_fields();
    let height = (fields.len() + 1 + 1) as u16 + 2;
    let rect = centered(area, width, height);
    let inner = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .inner(rect);
    let name_row = fields
        .iter()
        .position(|f| *f == FormField::Name)
        .expect("Name is visible") as u16;
    assert_eq!(cursor.y, inner.y + name_row);
    assert_eq!(cursor.x, inner.x + (MARKER_WIDTH + LABEL_WIDTH) as u16 + 2);
}

#[test]
fn remove_dialog_shows_the_checkbox_only_for_worktree_windows() {
    let mut confirm = RemoveConfirm {
        window_id: 1,
        name: "api-worker".into(),
        branch: Some("feat/api".into()),
        remove_worktree: false,
    };
    let out = draw(100, 30, |frame| {
        render_remove_confirm(frame, &confirm, frame.area(), theme::DEFAULT_ACCENT);
    });
    assert!(out.contains("[ ] also remove worktree feat/api"), "{out}");
    assert!(out.contains("the branch is kept"), "{out}");

    confirm.remove_worktree = true;
    let out = draw(100, 30, |frame| {
        render_remove_confirm(frame, &confirm, frame.area(), theme::DEFAULT_ACCENT);
    });
    assert!(out.contains("[x] also remove worktree feat/api"), "{out}");

    let plain = RemoveConfirm {
        window_id: 2,
        name: "shell-1".into(),
        branch: None,
        remove_worktree: false,
    };
    let out = draw(100, 30, |frame| {
        render_remove_confirm(frame, &plain, frame.area(), theme::DEFAULT_ACCENT);
    });
    assert!(out.contains("Remove 'shell-1'?"), "{out}");
    assert!(!out.contains("also remove worktree"), "{out}");
    assert!(!out.contains("Space toggle"), "{out}");
}

#[test]
fn force_prompt_lists_the_three_choices() {
    let out = draw(100, 30, |frame| {
        render_force_remove(
            frame,
            "api",
            "worktree /tmp/shop-abc/feat-api has uncommitted or untracked changes",
            frame.area(),
            theme::DEFAULT_ACCENT,
        );
    });
    assert!(out.contains("force"), "{out}");
    assert!(out.contains("keep the worktree"), "{out}");
    assert!(out.contains("cancel"), "{out}");
}

/// The prompt whose `f` key deletes a checkout with `--force` must say whose checkout.
/// The whole-branch review's finding 1 was exactly a force prompt that named one window
/// in its message and forced another, and it was invisible on screen because the window
/// the dialog targeted was never drawn.
#[test]
fn force_prompt_names_the_window_it_targets() {
    let out = draw(100, 30, |frame| {
        render_force_remove(
            frame,
            "alpha",
            "worktree /tmp/shop-abc/feat-beta has uncommitted or untracked changes",
            frame.area(),
            theme::DEFAULT_ACCENT,
        );
    });
    assert!(
        out.contains("'alpha'"),
        "the targeted window's name must be on screen: {out}"
    );
    assert!(out.contains("feat-beta"), "{out}");
}

/// Fix wave C item 2: wave A tripled the refusal messages' length (~123 → 177–193
/// columns) without touching the 4 × 50 = 200-column budget this prompt wrapped them
/// into, so on a realistic path the sentence that says what forcing away destroys — the
/// entire payload of finding 2 — was cut off screen. The test that should have caught it
/// used the legacy short message and a 22-character path, both far under the budget, so
/// the truncation was invisible to it.
///
/// This one uses a realistic path (`<worktrees_root>/<project>-<hash8>/<branch-dir>`,
/// design decision 6, with a real `hash8` and a real branch name) and every
/// `DirtyReason`'s real sentence, and asserts two things for each: the branch directory
/// — the path's tail, and the part finding 1 cared about — reaches the screen, and the
/// message's own last line (from the same `wrap` call `render_force_remove` makes) does
/// too, with no word dropped between the message and what `wrap` produced for it.
#[test]
fn force_prompt_shows_every_dirty_reason_in_full_with_a_realistic_path() {
    use daemon::worktree::{DirtyReason, worktree_dir};
    use std::path::Path;

    let worktrees_root = Path::new("/home/dev/.local/share/anthrex/worktrees");
    let project_root = Path::new("/home/dev/projects/anthrex");
    let path = worktree_dir(worktrees_root, project_root, "feat/worktree-removal");
    let branch_dir = "feat-worktree-removal";
    assert!(
        path.to_string_lossy().ends_with(branch_dir),
        "fixture sanity, not the thing under test: {}",
        path.display()
    );

    for reason in [
        DirtyReason::Rebase,
        DirtyReason::Merge,
        DirtyReason::CherryPick,
        DirtyReason::Revert,
        DirtyReason::Sequence,
        DirtyReason::Bisect,
        DirtyReason::UnreachableHead,
    ] {
        let message = format!("worktree {} {reason}", path.display());
        assert!(
            message.len() > FORCE_WRAP_WIDTH * 3,
            "{reason:?}: fixture must actually exercise several wrapped lines: \
             {} columns",
            message.len()
        );

        // What `render_force_remove` itself hands to the screen for this message, so
        // the assertions below are the real budget, not a guess at it. Compared with
        // whitespace stripped from both sides, because a normal word-wrap break
        // consumes the one space at that point while a hard-cut break (inside the
        // path's one long "word") consumes none — either way, no character of the
        // message may go missing.
        let wrapped = wrap(&message, FORCE_WRAP_WIDTH, FORCE_MAX_LINES);
        let without_whitespace =
            |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(
            without_whitespace(&wrapped.concat()),
            without_whitespace(&message),
            "{reason:?}: the wrap must not drop any character of the message, which is \
             what `max_lines` truncating silently would do: {wrapped:?}"
        );
        let last_line = wrapped.last().expect("at least one wrapped line");

        let out = draw(100, 30, |frame| {
            render_force_remove(frame, "api", &message, frame.area(), theme::DEFAULT_ACCENT);
        });
        assert!(
            out.contains(branch_dir),
            "{reason:?}: the path's tail must reach the screen: {out}"
        );
        assert!(
            out.contains(last_line.as_str()),
            "{reason:?}: the sentence's final line must reach the screen, not be cut \
             before it: {out}"
        );
    }
}

/// Finding 9 of the whole-branch review: `wrap` hard-cut a single word longer than the
/// wrap width at its first `width` characters and silently dropped the rest. The word
/// that hits this in practice is the worktree path in the force-remove message, and
/// macOS's default data-directory path is long enough to hit it routinely — the dialog
/// exists to tell the user *which* worktree holds uncommitted work immediately before
/// offering to delete it, so losing the path's tail undercuts the one question it
/// answers.
///
/// One "word" (no whitespace) longer than two wrap widths, so every line `wrap` produces
/// is a hard-cut piece of it, and the pieces must reassemble the original exactly.
#[test]
fn wrap_continues_an_overlong_word_onto_the_lines_that_follow() {
    let path = "/Users/someone/Library/Application-Support/anthrex/worktrees/\
                shop-5cc2e648deadbeef1234/feat-a-rather-long-branch-name-for-good-measure";
    assert!(
        path.chars().count() > FORCE_WRAP_WIDTH * 2,
        "fixture must exceed two wrap widths to exercise more than one continuation"
    );

    let lines = wrap(path, FORCE_WRAP_WIDTH, 10);

    assert!(
        lines.len() >= 3,
        "a word this long, wrapped at {FORCE_WRAP_WIDTH}, must take at least three \
         lines: {lines:?}"
    );
    for line in &lines {
        assert!(
            line.chars().count() <= FORCE_WRAP_WIDTH,
            "no line may exceed the wrap width: {line:?}"
        );
    }
    assert_eq!(
        lines.concat(),
        path,
        "hard-cutting an overlong word must continue its remainder on the lines that \
         follow, not drop it"
    );
    assert!(
        lines
            .last()
            .expect("at least three lines")
            .ends_with("good-measure"),
        "the word's final segment must survive, on the last line, not just its first \
         {FORCE_WRAP_WIDTH} columns: {lines:?}"
    );
}
