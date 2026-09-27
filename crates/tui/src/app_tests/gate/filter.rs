//! Whole-branch review I1: while the text filter is being typed, every key is the
//! filter's. None reaches the run view, so filter text holding `a` then `y` never
//! approves the run, and `f`, `h` and `Esc` never act as the run view's keys.

use super::*;

/// The gate with the filter being typed, `selected` chosen before `/`.
fn filtering(selected: NodeKey) -> App {
    let mut app = gate();
    select(&mut app, selected);
    assert!(tap(&mut app, KeyCode::Char('/')).is_empty());
    assert_eq!(app.tree_input, Some(TreeInput::Filter));
    app
}

fn view(app: &App) -> RunView {
    app.run_view.clone().expect("the run view is open")
}

#[test]
fn typing_a_filter_at_the_gate_sends_nothing_and_opens_nothing() {
    let mut app = filtering(task_key("t2"));
    let before = view(&app);
    for c in "daxey".chars() {
        let effects = tap(&mut app, KeyCode::Char(c));
        assert!(effects.is_empty(), "{c:?} produced {effects:?}");
        assert_eq!(app.modal, None, "{c:?} opened a modal");
    }
    assert_eq!(app.tree.filter, "daxey");
    assert_eq!(app.tree_input, Some(TreeInput::Filter));
    assert_eq!(view(&app), before);
}

#[test]
fn f_while_filtering_is_filter_text_not_the_round_filter() {
    let mut app = filtering(task_key("t2"));
    let before = view(&app);
    assert!(tap(&mut app, KeyCode::Char('f')).is_empty());
    assert_eq!(app.tree.filter, "f");
    assert_eq!(view(&app), before, "f cycled the run view's filter");
}

#[test]
fn h_on_the_root_while_filtering_is_filter_text_and_keeps_the_view() {
    let mut app = filtering(NodeKey::Run(RUN_ID.into()));
    let before = view(&app);
    assert!(tap(&mut app, KeyCode::Char('h')).is_empty());
    assert_eq!(app.tree.filter, "h");
    assert_eq!(app.tree_input, Some(TreeInput::Filter));
    assert_eq!(view(&app), before, "h closed the run view");
}

#[test]
fn esc_while_filtering_ends_the_filter_and_keeps_the_view() {
    let mut app = filtering(task_key("t2"));
    let before = view(&app);
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.tree.filter, "");
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert_eq!(app.modal, None);
    assert_eq!(view(&app), before, "Esc closed the run view");
}
