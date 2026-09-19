use super::*;

fn tree_command(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Vec<Effect> {
    prefix(app);
    press(app, code, modifiers)
}

#[test]
fn tree_toggle_enters_and_leaves_tree_mode() {
    let mut app = app_with(project_windows());
    assert_eq!(app.focused, Some(2));

    assert!(tree_command(&mut app, KeyCode::Char('t'), KeyModifiers::NONE).is_empty());
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert!(app.keymap.tree_mode());
    assert_eq!(app.tree.selected, Some(tree::NodeKey::Window(2)));

    assert!(tree_command(&mut app, KeyCode::Char('t'), KeyModifiers::NONE).is_empty());
    assert_eq!(app.tree_input, None);
    assert!(!app.keymap.tree_mode());
    assert_eq!(app.tree.selected, None);

    assert!(tree_command(&mut app, KeyCode::Char('s'), KeyModifiers::NONE).is_empty());
    assert!(!app.sidebar_visible);
    assert!(tree_command(&mut app, KeyCode::Char('t'), KeyModifiers::NONE).is_empty());
    assert!(app.sidebar_visible);
}

#[test]
fn overview_toggle_enters_and_leaves_tree_mode() {
    let mut app = app_with(project_windows());

    assert!(tree_command(&mut app, KeyCode::Char('T'), KeyModifiers::SHIFT).is_empty());
    assert!(app.overview);
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert!(app.keymap.tree_mode());

    assert!(tree_command(&mut app, KeyCode::Char('T'), KeyModifiers::NONE).is_empty());
    assert!(!app.overview);
    assert_eq!(app.tree_input, None);
    assert!(!app.keymap.tree_mode());
}

#[test]
fn sidebar_width_steps_by_four_within_limits() {
    let mut app = app_with(project_windows());
    assert_eq!(app.sidebar_width, 34);

    for expected in [30, 26, 24, 24] {
        assert!(tree_command(&mut app, KeyCode::Char('<'), KeyModifiers::NONE).is_empty());
        assert_eq!(app.sidebar_width, expected);
    }

    app.sidebar_width = 56;
    for expected in [60, 60] {
        assert!(tree_command(&mut app, KeyCode::Char('>'), KeyModifiers::NONE).is_empty());
        assert_eq!(app.sidebar_width, expected);
    }

    assert!(tree_command(&mut app, KeyCode::Char('s'), KeyModifiers::NONE).is_empty());
    assert!(!app.sidebar_visible);
    assert!(tree_command(&mut app, KeyCode::Char('<'), KeyModifiers::NONE).is_empty());
    assert!(app.sidebar_visible);
}

#[test]
fn keys_in_tree_mode_never_reach_the_pty() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    assert!(tree_command(&mut app, KeyCode::Char('t'), KeyModifiers::NONE).is_empty());

    for code in [KeyCode::Char('j'), KeyCode::Char('q'), KeyCode::Enter] {
        let effects = press(&mut app, code, KeyModifiers::NONE);
        assert!(
            effects
                .iter()
                .all(|effect| !matches!(effect, Effect::Send(ClientMsg::Input { .. }))),
            "{code:?} leaked input: {effects:?}"
        );
    }
    let effects = app.on_paste("pasted text".into());
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::Send(ClientMsg::Input { .. }))),
        "paste leaked input: {effects:?}"
    );
}

#[test]
fn escape_leaves_tree_mode_and_closes_overview() {
    let mut app = app_with(project_windows());
    assert!(tree_command(&mut app, KeyCode::Char('T'), KeyModifiers::NONE).is_empty());

    assert!(press(&mut app, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert!(!app.overview);
    assert_eq!(app.tree_input, None);
    assert!(!app.keymap.tree_mode());
}
