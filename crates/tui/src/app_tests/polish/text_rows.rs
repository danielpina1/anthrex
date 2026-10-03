//! M9.0.7.13a fix round 1: every text area's Up and Down move a row as the form draws
//! it, so the app hands each form the width its text area is drawn at (decision 35).
//! With the last frame 80×23 every area wraps a long line; with no width (0) Up would
//! leave the line or the field instead.

use super::super::action_forms::{answer_form, forms_app, open, task_t1, type_text};
use super::super::gate::{focus as edit_focus, form as edit_form, gate, open_form as open_edit};
use super::super::goal_form::{app_with_cache, form as goal_form, open_form as open_goal, roster};
use super::super::*;
use crate::run_edit::EditField;
use crate::run_goal::GoalField;
use proto::ActionKind;
use ratatui::layout::Rect;

fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

/// The last frame: 80×24, the status bar's row under the body.
fn drawn_80x24(app: &mut App) {
    app.set_body_area(Rect::new(0, 0, 80, 23));
}

/// Milestone 9.3 decisions 5 and 7 (changed expectation): the goal is the large
/// editor at 80x24, 72 columns wide, so it wraps at 71; Up and Down move a drawn row
/// and stay put on the first and the last (the focus no longer leaves by them).
#[test]
fn the_goal_moves_a_wrapped_row_and_stays_on_its_ends() {
    let mut app = app_with_cache(roster());
    drawn_80x24(&mut app);
    open_goal(&mut app);
    for _ in 0..150 {
        tap(&mut app, KeyCode::Char('a'));
    }
    assert_eq!(goal_form(&app).goal.cursor(), 150);
    assert!(tap(&mut app, KeyCode::Up).is_empty());
    assert_eq!(
        goal_form(&app).goal.cursor(),
        79,
        "a drawn row up, column 8"
    );
    tap(&mut app, KeyCode::Up);
    assert_eq!(goal_form(&app).goal.cursor(), 8);
    tap(&mut app, KeyCode::Up);
    assert_eq!(
        (goal_form(&app).focus, goal_form(&app).goal.cursor()),
        (GoalField::Goal, 8),
        "Up on the first row stays put"
    );
    tap(&mut app, KeyCode::Down);
    tap(&mut app, KeyCode::Down);
    assert_eq!(goal_form(&app).goal.cursor(), 150);
    tap(&mut app, KeyCode::Down);
    assert_eq!(
        (goal_form(&app).focus, goal_form(&app).goal.cursor()),
        (GoalField::Goal, 150),
        "Down on the last row stays put"
    );
    tap(&mut app, KeyCode::Tab);
    assert_eq!(goal_form(&app).focus, GoalField::Runtime, "Tab leaves");
}

#[test]
fn the_edit_brief_moves_a_wrapped_row() {
    let mut app = gate();
    drawn_80x24(&mut app);
    open_edit(&mut app, "t1");
    edit_focus(&mut app, EditField::Brief);
    // `Line one`, `Line two`, then 60 `b`s: the brief's area is 47 columns at 80, so
    // the last line wraps at 46 into two rows.
    press(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL);
    for _ in 0..60 {
        tap(&mut app, KeyCode::Char('b'));
    }
    let start = "Line one\nLine two\n".chars().count();
    assert_eq!(edit_form(&app).brief.cursor(), start + 60);
    assert!(tap(&mut app, KeyCode::Up).is_empty());
    assert_eq!(edit_form(&app).focus, EditField::Brief);
    assert_eq!(
        edit_form(&app).brief.cursor(),
        start + 14,
        "the wrapped row above, column 14, still in the long line"
    );
}

#[test]
fn the_answer_moves_a_wrapped_row() {
    let mut app = forms_app(false);
    drawn_80x24(&mut app);
    open(&mut app, task_t1(), ActionKind::Answer);
    tap(&mut app, KeyCode::Enter);
    // The answer's area is 52 columns at 80 (60 less the 8-column label): it wraps at
    // 51, so 70 characters are two rows.
    type_text(&mut app, &"c".repeat(70));
    assert_eq!(answer_form(&app).text.cursor(), 70);
    tap(&mut app, KeyCode::Up);
    assert_eq!(
        answer_form(&app).text.cursor(),
        19,
        "a drawn row up, column 19"
    );
}
