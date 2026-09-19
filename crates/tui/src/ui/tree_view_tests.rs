use crate::{app::App, keymap::Keymap, theme, tree, ui};
use proto::{Status, SubagentState};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Modifier};
use unicode_width::UnicodeWidthStr;

fn app() -> App {
    let mut app = App::new(
        tree::example_windows(),
        "/tmp".into(),
        Keymap::default_prefix(),
    );
    app.set_terminal_size(80, 24);
    app.enter_tree();
    app.overview = true;
    app
}

fn render(app: &App, width: u16) -> (Buffer, ui::Layout) {
    let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
    let mut layout = None;
    terminal
        .draw(|frame| layout = Some(ui::draw(frame, app)))
        .unwrap();
    (terminal.backend().buffer().clone(), layout.unwrap())
}

fn row(buffer: &Buffer, layout: &ui::Layout, offset: u16) -> String {
    let mut text = String::new();
    let mut x = layout.main_inner.x;
    while x < layout.main_inner.right() {
        let symbol = buffer[(x, layout.main_inner.y + offset)].symbol();
        text.push_str(symbol);
        x += UnicodeWidthStr::width(symbol).max(1) as u16;
    }
    text.trim_end().to_owned()
}

#[test]
fn wide_rows_show_full_fields_and_finished_duration() {
    let mut app = app();
    app.windows[0].tool = Some("Bash".into());
    let (buffer, layout) = render(&app, 160);
    assert!(row(&buffer, &layout, 0).starts_with("▾ shop  /r/shop"));
    assert!(row(&buffer, &layout, 0).ends_with("◆ attention  cl 4 · cx 3"));
    assert_eq!(
        row(&buffer, &layout, 1),
        "▎ ⠋ 1 api-worker  claude  claude-opus-5      working      2m  Bash"
    );
    assert_eq!(
        row(&buffer, &layout, 2),
        "  │ ├ ⠋ Explore: map routes  -  running  1m  Read"
    );
    assert_eq!(
        row(&buffer, &layout, 4),
        "  │ └ ✓ tests: run unit suite  -  done  45s"
    );
    for x in layout.main_inner.x..layout.main_inner.right() {
        assert!(
            buffer[(x, layout.main_inner.y + 1)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            !buffer[(x, layout.main_inner.y + 2)]
                .modifier
                .contains(Modifier::REVERSED)
        );
    }
}

#[test]
fn wide_subagent_permission_keeps_state_and_missing_end_is_zero() {
    let mut app = app();
    let sub = &mut app.windows[0].subagents[0];
    sub.label = None;
    sub.model = Some("claude-haiku-4-5".into());
    sub.needs_permission = true;
    let sub = &mut app.windows[0].subagents[2];
    sub.state = SubagentState::Failed;
    sub.ended_secs = None;
    let (buffer, layout) = render(&app, 160);
    assert_eq!(
        row(&buffer, &layout, 2),
        "  │ ├ ◆ Explore  claude-haiku-4-5  running  1m  Read"
    );
    assert_eq!(
        buffer[(layout.main_inner.x + 6, layout.main_inner.y + 2)].fg,
        theme::status_color(Status::Attention)
    );
    assert_eq!(
        row(&buffer, &layout, 4),
        "  │ └ ✕ tests: run unit suite  -  failed  0s"
    );
}

#[test]
fn wide_project_shortens_exact_home_to_tilde() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let mut app = app();
    for window in &mut app.windows {
        window.project = home.clone();
    }
    let (buffer, layout) = render(&app, 160);
    let text = row(&buffer, &layout, 0);
    assert!(text.contains("  ~ "), "{text}");
    assert!(!text.contains("~/"), "{text}");
}

#[test]
fn wide_columns_cap_unicode_names_and_models_without_splitting_graphemes() {
    let mut app = app();
    app.windows[0].name = "👩🏽‍💻".repeat(20);
    app.windows[0].model = Some("界".repeat(20));
    app.windows[1].name = "e\u{301}".repeat(30);
    let (buffer, layout) = render(&app, 180);
    let text = row(&buffer, &layout, 1);
    assert!(
        text.contains(&format!(
            "{}…   claude  {}…   working",
            "👩🏽‍💻".repeat(11),
            "界".repeat(13)
        )),
        "{text}"
    );
    let text = row(&buffer, &layout, 5);
    assert!(
        text.contains(&format!("{}…  codex ", "e\u{301}".repeat(23))),
        "{text}"
    );
    // A filter hides the long columns, so the remaining rows use their own widths.
    app.tree.filter = "frontend".into();
    let (buffer, layout) = render(&app, 160);
    let text = row(&buffer, &layout, 1);
    assert!(
        text.contains("frontend  claude  claude-sonnet-4-5  working"),
        "{text}"
    );
}

#[test]
fn wide_overview_stays_inside_tiny_main_areas() {
    let mut app = app();
    app.sidebar_visible = false;
    app.windows[0].name = "👩🏽‍💻".repeat(20);
    for width in 2..=20 {
        let (buffer, layout) = render(&app, width);
        let text = row(&buffer, &layout, 1);
        assert!(UnicodeWidthStr::width(text.as_str()) <= usize::from(layout.main_inner.width));
        assert!(!text.contains('\u{fffd}'));
        assert_eq!(
            buffer[(layout.main.right() - 1, layout.main_inner.y + 1)].symbol(),
            "│"
        );
    }
}
