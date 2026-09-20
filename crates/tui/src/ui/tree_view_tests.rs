use crate::{app::App, keymap::Keymap, theme, tree, ui};
use proto::{Runtime, Status, SubagentInfo, SubagentState, WindowInfo};
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

/// A plain app for tests that call the renderers directly, without going
/// through a full `Terminal` draw.
fn app_narrow() -> App {
    App::new(
        tree::example_windows(),
        "/tmp".into(),
        Keymap::default_prefix(),
    )
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
        "├─▎ ⠋ 1 api-worker  claude  claude-opus-5      working      2m  Bash"
    );
    assert_eq!(
        row(&buffer, &layout, 2),
        "│ ├─⠋ Explore: map routes               -  running  1m  Read"
    );
    assert_eq!(
        row(&buffer, &layout, 4),
        "│ └─✓ tests: run unit suite             -  done  45s"
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
        "│ ├─◆ Explore                           claude-haiku-4-5  running  1m  Read"
    );
    assert_eq!(
        buffer[(layout.main_inner.x + 4, layout.main_inner.y + 2)].fg,
        theme::status_color(Status::Attention)
    );
    assert_eq!(
        row(&buffer, &layout, 4),
        "│ └─✕ tests: run unit suite             -  failed  0s"
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

/// The joined text of every span a renderer produced for one row.
fn spans_text(line: &ratatui::text::Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

#[test]
fn narrow_rows_start_with_their_guides() {
    let app = app_narrow();
    let rows = app.rows();
    let nested = rows
        .iter()
        .filter(|row| !matches!(row.kind, tree::RowKind::Project { .. }));
    let mut saw_a_guide = false;
    for row in nested {
        let line = ui::tree_view::narrow_line(&app, row, 40, 1, false);
        let text = spans_text(&line);
        assert!(
            text.starts_with(row.guides.as_str()),
            "{:?} should start with its guides {:?}: {text:?}",
            row.key,
            row.guides
        );
        saw_a_guide |= !row.guides.is_empty();
    }
    assert!(saw_a_guide, "the fixture should exercise nested rows");
}

#[test]
fn wide_rows_start_with_their_guides() {
    let app = app_narrow();
    let rows = app.rows();
    let columns = ui::tree_view::WideColumns::from_rows(&rows);
    for row in rows
        .iter()
        .filter(|row| !matches!(row.kind, tree::RowKind::Project { .. }))
    {
        let line = ui::tree_view::wide_line(&app, row, 200, &columns, false);
        let text = spans_text(&line);
        assert!(
            text.starts_with(row.guides.as_str()),
            "{:?} should start with its guides {:?}: {text:?}",
            row.key,
            row.guides
        );
    }

    // frontend's b1 (guides "│ └─") and b3 (guides "│     └─") sit two levels
    // apart. With the guides eating into the shared name-column budget, the
    // trailing fields — everything after the name — must start at the same
    // offset whatever the row's own guide width (decision 27).
    let shallow = rows
        .iter()
        .find(|row| {
            row.key
                == tree::NodeKey::Subagent {
                    window_id: 4,
                    id: "b1".into(),
                }
        })
        .expect("frontend's b1");
    let deep = rows
        .iter()
        .find(|row| {
            row.key
                == tree::NodeKey::Subagent {
                    window_id: 4,
                    id: "b3".into(),
                }
        })
        .expect("frontend's b3");
    assert!(
        UnicodeWidthStr::width(shallow.guides.as_str())
            < UnicodeWidthStr::width(deep.guides.as_str())
    );
    let field_start = |row: &tree::Row<'_>| -> usize {
        let line = ui::tree_view::wide_line(&app, row, 200, &columns, false);
        line.spans[0..3]
            .iter()
            .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
            .sum()
    };
    assert_eq!(field_start(shallow), field_start(deep));
}

#[test]
fn the_focus_marker_follows_the_guides() {
    let mut app = app_narrow();
    app.focused = Some(1); // api-worker: nested under shop, has sub-agents
    let rows = app.rows();
    let window_row = rows
        .iter()
        .find(|row| row.key == tree::NodeKey::Window(1))
        .expect("api-worker");
    assert!(!window_row.guides.is_empty(), "api-worker is nested");
    let line = ui::tree_view::narrow_line(&app, window_row, 60, 1, false);
    assert_eq!(line.spans[0].content.as_ref(), window_row.guides.as_str());
    assert_eq!(line.spans[1].content.as_ref(), "▎", "focus marker");
    assert_eq!(line.spans[2].content.as_ref(), " ", "not collapsed");
    assert_eq!(
        line.spans[3].content.as_ref(),
        theme::status_glyph(Status::Working, app.spinner_frame),
        "status glyph"
    );
}

#[test]
fn the_collapsed_marker_follows_the_guides() {
    let mut app = app_narrow();
    app.tree.collapsed.insert(tree::NodeKey::Window(1));
    let rows = app.rows();
    let window_row = rows
        .iter()
        .find(|row| row.key == tree::NodeKey::Window(1))
        .expect("api-worker");
    let line = ui::tree_view::narrow_line(&app, window_row, 60, 1, false);
    assert_eq!(line.spans[0].content.as_ref(), window_row.guides.as_str());
    assert_eq!(line.spans[1].content.as_ref(), " ", "not focused");
    assert_eq!(line.spans[2].content.as_ref(), "▸", "collapsed marker");
    assert_eq!(
        line.spans[3].content.as_ref(),
        theme::status_glyph(Status::Working, app.spinner_frame),
        "status glyph follows both markers"
    );
}

#[test]
fn a_deep_name_is_truncated_not_the_guides() {
    let mut subagents = Vec::new();
    let mut parent: Option<String> = None;
    for level in 1..=6u64 {
        let id = format!("s{level}");
        subagents.push(SubagentInfo {
            id: id.clone(),
            parent_id: parent.clone(),
            kind: "Explore".into(),
            label: Some(if level == 6 {
                "a sub-agent label far too long for a 32-column sidebar".into()
            } else {
                format!("level {level}")
            }),
            model: None,
            state: SubagentState::Running,
            tool: None,
            started_secs: 100 - level,
            ended_secs: None,
            needs_permission: false,
        });
        parent = Some(id);
    }
    let window = WindowInfo {
        id: 1,
        name: "deep".into(),
        runtime: Runtime::Shell,
        cwd: "/p".into(),
        project: "/p".into(),
        worktree: None,
        branch: None,
        status: Status::Idle,
        tool: None,
        since_secs: 0,
        last_output_secs: 0,
        session_id: None,
        model: None,
        subagents,
        exit: None,
    };
    let app = App::new(vec![window], "/tmp".into(), Keymap::default_prefix());
    let rows = app.rows();
    let deepest = rows
        .iter()
        .find(|row| {
            row.key
                == tree::NodeKey::Subagent {
                    window_id: 1,
                    id: "s6".into(),
                }
        })
        .expect("the chain reaches six levels");
    // Sole window, sole child at every level: nothing ever has a later
    // sibling, so every ancestor column is a gap and the row's own bit is
    // always the elbow.
    assert_eq!(deepest.guides, "  ".repeat(6) + "└─");

    let line = ui::tree_view::narrow_line(&app, deepest, 32, 1, false);
    let text = spans_text(&line);
    assert_eq!(UnicodeWidthStr::width(text.as_str()), 32);
    assert!(
        text.starts_with(deepest.guides.as_str()),
        "guides must survive: {text:?}"
    );
    assert!(
        text.contains('…'),
        "the long label should be elided, not the guides: {text:?}"
    );
}
