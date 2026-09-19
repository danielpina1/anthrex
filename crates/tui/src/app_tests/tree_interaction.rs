use super::*;
use crate::tree::NodeKey;

fn example() -> App {
    let mut app = app_with(tree::example_windows());
    app.enter_tree();
    app
}

fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

fn select(app: &mut App, key: NodeKey) {
    let rows = tree::build(&app.windows, &app.tree);
    app.tree.select(&rows, key);
}

fn subagent(window_id: u32, id: &str) -> NodeKey {
    NodeKey::Subagent {
        window_id,
        id: id.into(),
    }
}

fn subscription(id: u32) -> Vec<Effect> {
    vec![Effect::Send(ClientMsg::Subscribe {
        window_id: id,
        cols: 80,
        rows: 24,
    })]
}

#[test]
fn j_and_k_move_the_selection_without_wrapping() {
    for (down, up) in [
        (KeyCode::Char('j'), KeyCode::Char('k')),
        (KeyCode::Down, KeyCode::Up),
    ] {
        let mut app = example();
        assert!(tap(&mut app, down).is_empty());
        assert_eq!(app.tree.selected, Some(subagent(1, "a1")));
        for expected in [
            NodeKey::Window(1),
            NodeKey::Project("/r/shop".into()),
            NodeKey::Project("/r/shop".into()),
        ] {
            assert!(tap(&mut app, up).is_empty());
            assert_eq!(app.tree.selected, Some(expected));
        }
        select(&mut app, NodeKey::Window(8));
        assert!(tap(&mut app, down).is_empty());
        assert_eq!(app.tree.selected, Some(NodeKey::Window(8)));
        assert_eq!(app.focused, Some(1));
    }
}

#[test]
fn enter_on_a_window_focuses_it_and_leaves_tree_mode() {
    let mut app = example();
    select(&mut app, NodeKey::Window(2));
    assert_eq!(tap(&mut app, KeyCode::Enter), subscription(2));
    assert_eq!(app.focused, Some(2));
    assert_eq!(app.tree_input, None);
    assert!(!app.keymap.tree_mode());
    assert_eq!(app.tree.selected, None);
    app.enter_tree();
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.tree_input, None, "already-focused Enter still exits");
}

#[test]
fn enter_on_a_subagent_focuses_its_window() {
    let mut app = example();
    app.focus(2);
    select(&mut app, subagent(4, "b2"));
    app.overview = true;
    assert_eq!(tap(&mut app, KeyCode::Enter), subscription(4));
    assert_eq!(app.tree_input, None);
    assert!(!app.overview);
}

#[test]
fn enter_and_space_toggle_projects_space_toggles_windows() {
    let mut app = example();
    let blog = NodeKey::Project("/r/blog".into());
    select(&mut app, blog.clone());
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(app.tree.is_collapsed(&blog));
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert!(tap(&mut app, KeyCode::Char(' ')).is_empty());
    assert!(!app.tree.is_collapsed(&blog));
    select(&mut app, NodeKey::Window(1));
    assert!(tap(&mut app, KeyCode::Char(' ')).is_empty());
    assert!(app.tree.is_collapsed(&NodeKey::Window(1)));
    assert!(tap(&mut app, KeyCode::Char(' ')).is_empty());
    select(&mut app, subagent(1, "a1"));
    let collapsed = app.tree.collapsed.clone();
    assert!(tap(&mut app, KeyCode::Char(' ')).is_empty());
    assert_eq!(app.tree.collapsed, collapsed);
    assert_eq!(app.tree.selected, Some(subagent(1, "a1")));
}

#[test]
fn filter_input_edits_and_selects_the_first_match() {
    let mut app = example();
    for c in "/style".chars() {
        assert!(tap(&mut app, KeyCode::Char(c)).is_empty());
    }
    assert_eq!(app.tree_input, Some(TreeInput::Filter));
    assert_eq!(app.tree.filter, "style");
    assert_eq!(app.tree.selected, Some(NodeKey::Window(4)));
    assert_eq!(app.focused, Some(1));
    assert!(tap(&mut app, KeyCode::Backspace).is_empty());
    assert_eq!(app.tree.filter, "styl");
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert_eq!(app.tree.filter, "styl");
    assert!(tap(&mut app, KeyCode::Char('/')).is_empty());
    assert_eq!(app.tree.filter, "styl", "reopening filter retains text");
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.tree.filter, "");
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert_eq!(app.tree.selected, Some(NodeKey::Window(4)));
}

#[test]
fn filter_accepts_printable_unicode_and_shift_but_not_control_or_alt() {
    let mut app = example();
    tap(&mut app, KeyCode::Char('/'));
    for (c, mods) in [
        ('é', KeyModifiers::NONE),
        ('S', KeyModifiers::SHIFT),
        ('x', KeyModifiers::CONTROL),
        ('y', KeyModifiers::ALT),
        ('\n', KeyModifiers::NONE),
    ] {
        assert!(press(&mut app, KeyCode::Char(c), mods).is_empty());
    }
    assert_eq!(app.tree.filter, "éS");
    tap(&mut app, KeyCode::Backspace);
    assert_eq!(app.tree.filter, "é");
    tap(&mut app, KeyCode::Backspace);
    tap(&mut app, KeyCode::Backspace);
    assert!(app.tree.filter.is_empty());
}

#[test]
fn filter_repairs_empty_results_and_retains_visible_selection() {
    let mut app = example();
    select(&mut app, subagent(4, "b2"));
    tap(&mut app, KeyCode::Char('/'));
    app.on_paste("tokens".into());
    assert_eq!(app.tree.selected, Some(subagent(4, "b2")));
    app.on_paste("no-match".into());
    assert_eq!(app.tree.selected, None);
    tap(&mut app, KeyCode::Esc);
    assert_eq!(app.tree.selected, Some(NodeKey::Window(1)));
    assert_eq!(app.focused, Some(1));
}

#[test]
fn clearing_filter_with_collapsed_projects_selects_the_first_row() {
    let mut app = example();
    app.tree.toggle(&NodeKey::Project("/r/shop".into()));
    app.tree.toggle(&NodeKey::Project("/r/blog".into()));
    tap(&mut app, KeyCode::Char('/'));
    app.on_paste("style".into());
    assert_eq!(app.tree.selected, Some(NodeKey::Window(4)));
    tap(&mut app, KeyCode::Esc);
    assert_eq!(app.tree.selected, Some(NodeKey::Project("/r/shop".into())));
    assert_eq!(app.focused, Some(1));
}

#[test]
fn escape_while_navigating_leaves_tree_mode_and_clears_the_filter() {
    let mut app = example();
    app.overview = true;
    app.tree.filter = "style".into();
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.tree_input, None);
    assert!(!app.keymap.tree_mode());
    assert!(!app.overview);
    assert!(app.tree.filter.is_empty());
}

#[test]
fn paste_goes_to_the_filter_only_while_typing_it() {
    let mut app = example();
    assert!(app.on_paste("ignored".into()).is_empty());
    assert!(app.tree.filter.is_empty());
    tap(&mut app, KeyCode::Char('/'));
    assert!(app.on_paste("to\nkens".into()).is_empty());
    assert_eq!(app.tree.filter, "tokens");
    assert_eq!(app.tree.selected, Some(NodeKey::Window(4)));
    assert!(app.on_paste("\r\n \t".into()).is_empty());
    assert_eq!(app.tree.filter, "tokens \t", "only newlines are removed");
}

#[test]
fn selection_stays_visible_while_moving() {
    let mut app = app_with((1..=20).map(|id| project_win(id, "/r/shop")).collect());
    app.set_tree_viewports(5, 20);
    app.enter_tree();
    for _ in 0..12 {
        assert!(tap(&mut app, KeyCode::Char('j')).is_empty());
    }
    assert_eq!(app.tree.selected, Some(NodeKey::Window(13)));
    assert_eq!(app.tree.sidebar.top, 9);
    assert_eq!(app.tree.overview.top, 0);
    assert_eq!(app.focused, Some(1));
    app.set_tree_viewports(3, 4);
    assert_eq!(app.tree.sidebar.top, 11);
    assert_eq!(app.tree.overview.top, 10);
    app.on_daemon(DaemonMsg::WindowsChanged {
        windows: app.windows.clone(),
    });
    assert_eq!(
        app.tree.sidebar.top, 11,
        "list updates reveal selection, not focus"
    );
    tap(&mut app, KeyCode::Esc);
    assert_eq!(app.tree.sidebar.top, 1, "exit reveals focused window");
}

#[test]
fn clicks_select_in_tree_mode_and_keep_the_mode_active() {
    let mut app = example();
    let layout = crate::ui::layout(ratatui::layout::Rect::new(0, 0, 120, 30), 34);
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    assert_eq!(app.on_click(2, 6, &layout), subscription(2));
    assert_eq!(app.tree.selected, Some(NodeKey::Window(2)));
    assert_eq!(app.on_click(2, 3, &layout), subscription(1));
    assert_eq!(app.tree.selected, Some(subagent(1, "a1")));
    assert!(app.on_click(2, 1, &layout).is_empty());
    assert_eq!(app.tree.selected, Some(NodeKey::Project("/r/shop".into())));
    assert!(app.tree.is_collapsed(&NodeKey::Project("/r/shop".into())));
    assert_eq!(
        app.focused,
        Some(1),
        "collapse hides focus without changing it"
    );
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
}

#[test]
fn prefix_still_has_priority_over_filter_input() {
    let mut app = example();
    tap(&mut app, KeyCode::Char('/'));
    prefix(&mut app);
    assert_eq!(tap(&mut app, KeyCode::Char('2')), subscription(2));
    assert_eq!(app.tree_input, Some(TreeInput::Filter));
    assert!(app.tree.filter.is_empty());
    prefix(&mut app);
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.tree_input, Some(TreeInput::Filter));
}
