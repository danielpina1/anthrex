use crate::{app::App, keymap::Keymap, theme, tree, ui};
use proto::{Runtime, Status, SubagentInfo, SubagentState, WindowInfo};
use unicode_width::UnicodeWidthStr;

/// A plain app for tests that call the renderers directly, without going
/// through a full `Terminal` draw.
fn app_narrow() -> App {
    App::new(
        tree::example_windows(),
        "/tmp".into(),
        Keymap::default_prefix(),
    )
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
