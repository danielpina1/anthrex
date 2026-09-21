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
        render_new_agent(frame, &form, frame.area());
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
        render_new_agent(frame, &form, frame.area());
    });
    assert!(!out.contains("Model"), "{out}");
    assert!(!out.contains("Prompt"), "{out}");

    form.submitting = true;
    let out = draw(100, 30, |frame| {
        render_new_agent(frame, &form, frame.area());
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
        .draw(|frame| render_new_agent(frame, &form, frame.area()))
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
        render_remove_confirm(frame, &confirm, frame.area());
    });
    assert!(out.contains("[ ] also remove worktree feat/api"), "{out}");
    assert!(out.contains("the branch is kept"), "{out}");

    confirm.remove_worktree = true;
    let out = draw(100, 30, |frame| {
        render_remove_confirm(frame, &confirm, frame.area());
    });
    assert!(out.contains("[x] also remove worktree feat/api"), "{out}");

    let plain = RemoveConfirm {
        window_id: 2,
        name: "shell-1".into(),
        branch: None,
        remove_worktree: false,
    };
    let out = draw(100, 30, |frame| {
        render_remove_confirm(frame, &plain, frame.area());
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
            "worktree /tmp/shop-abc/feat-api has uncommitted or untracked changes",
            frame.area(),
        );
    });
    assert!(out.contains("force"), "{out}");
    assert!(out.contains("keep the worktree"), "{out}");
    assert!(out.contains("cancel"), "{out}");
}
