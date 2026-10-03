//! Milestone 9.3 decision 4 as amended (task 9a's fix round 1): the editor keeps a
//! viewport top, nano's way. Moving past an edge scrolls one row, a jump scrolls as
//! little as it needs, PgUp/PgDn shift the view by the rows they moved and keep the goal
//! column. Every view is read back through `kit::editor`, as the dialog draws it.

use super::super::TextArea;
use crate::ui::kit;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Modifier;
use unicode_segmentation::UnicodeSegmentation;

const W: u16 = 40;
const ROWS: u16 = 5;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn press(area: &mut TextArea, k: KeyEvent, times: usize) {
    for _ in 0..times {
        area.on_editor_key(k, W, ROWS);
    }
}

/// The view as drawn: the first row's text and the cursor's row in the view.
fn view(area: &TextArea) -> (String, usize) {
    let lines = kit::editor(area, ROWS, W, true);
    assert_eq!(lines.len(), usize::from(ROWS));
    let texts: Vec<String> = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    let row = lines
        .iter()
        .position(|l| {
            l.spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::REVERSED))
        })
        .expect("the cursor is always in view");
    (texts[0].trim_end().to_string(), row)
}

fn first(n: usize, row: usize) -> (String, usize) {
    (format!("line{n}"), row)
}

/// `line0` … `line29`, the cursor at the end (as a restored draft is).
fn thirty() -> TextArea {
    let text: Vec<String> = (0..30).map(|i| format!("line{i}")).collect();
    TextArea::editor(&text.join("\n"))
}

/// The cursor as `(logical line, grapheme column)`, both from 0.
fn at(area: &TextArea) -> (usize, usize) {
    let before: String = area.text().graphemes(true).take(area.cursor()).collect();
    let line = before.matches('\n').count();
    let column = before
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .graphemes(true)
        .count();
    (line, column)
}

#[test]
fn moving_up_and_down_through_a_long_text_scrolls_by_one_row_at_the_edges() {
    let mut a = thirty();
    // A restored draft: the last five lines, the cursor on the bottom row.
    assert_eq!(view(&a), first(25, 4));
    // Up inside the view moves the cursor, not the view.
    press(&mut a, key(KeyCode::Up), 4);
    assert_eq!(view(&a), first(25, 0));
    // One more crosses the top edge: the view scrolls one row.
    press(&mut a, key(KeyCode::Up), 1);
    assert_eq!(view(&a), first(24, 0));
    press(&mut a, key(KeyCode::Up), 3);
    assert_eq!(view(&a), first(21, 0));
    // Back down: the view stays until the cursor crosses the bottom edge.
    press(&mut a, key(KeyCode::Down), 4);
    assert_eq!(view(&a), first(21, 4));
    press(&mut a, key(KeyCode::Down), 1);
    assert_eq!(view(&a), first(22, 4));
    // From the top the same, downwards.
    press(&mut a, ctrl_code(KeyCode::Home), 1);
    assert_eq!(view(&a), first(0, 0));
    press(&mut a, key(KeyCode::Down), 4);
    assert_eq!(view(&a), first(0, 4));
    press(&mut a, key(KeyCode::Down), 1);
    assert_eq!(view(&a), first(1, 4));
    // Typing and deleting inside the view leave it alone.
    press(&mut a, key(KeyCode::Up), 2);
    a.on_editor_key(key(KeyCode::Enter), W, ROWS);
    assert_eq!(view(&a), first(1, 3));
    a.on_editor_key(key(KeyCode::Backspace), W, ROWS);
    assert_eq!(view(&a), first(1, 2));
}

#[test]
fn jumps_scroll_as_little_as_needed() {
    let mut a = thirty();
    press(&mut a, ctrl_code(KeyCode::Home), 1);
    press(&mut a, key(KeyCode::Down), 10);
    assert_eq!(view(&a), first(6, 4));
    press(&mut a, key(KeyCode::Up), 4);
    assert_eq!(view(&a), first(6, 0));
    // A paste that ends inside the view does not move it.
    a.on_editor_paste("a\nb\n", W, ROWS);
    assert_eq!(at(&a), (8, 0));
    assert_eq!(
        view(&a),
        ("a".to_string(), 2),
        "the view still starts at line 6"
    );
    // One that ends below it scrolls until the cursor is on the bottom row.
    a.on_editor_paste("1\n2\n3\n4\n5\n6\n", W, ROWS);
    assert_eq!(at(&a), (14, 0));
    assert_eq!(view(&a).1, 4);
    // Ctrl-End and Ctrl-Home land on the nearest edge row.
    press(&mut a, ctrl_code(KeyCode::End), 1);
    assert_eq!(view(&a), first(25, 4), "38 lines: `line25` is the 34th");
    press(&mut a, ctrl_code(KeyCode::Home), 1);
    assert_eq!(view(&a), ("line0".to_string(), 0));
    // Ctrl-U of a long cut, from the top row.
    press(
        &mut a,
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
        7,
    );
    assert_eq!(
        view(&a),
        ("b".to_string(), 0),
        "seven lines cut, `b` is first"
    );
    press(
        &mut a,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        1,
    );
    assert_eq!(at(&a), (7, 0));
    assert_eq!(view(&a), first(3, 4));
}

#[test]
fn pgup_and_pgdn_shift_the_view_by_the_rows_moved() {
    let mut a = thirty();
    press(&mut a, ctrl_code(KeyCode::Home), 1);
    press(&mut a, key(KeyCode::PageDown), 1);
    assert_eq!(at(&a).0, 4);
    assert_eq!(
        view(&a),
        first(4, 0),
        "the view moved four rows with the cursor"
    );
    press(&mut a, key(KeyCode::PageDown), 1);
    assert_eq!(view(&a), first(8, 0));
    press(&mut a, key(KeyCode::PageUp), 1);
    assert_eq!(view(&a), first(4, 0));
    press(&mut a, key(KeyCode::PageUp), 1);
    assert_eq!(view(&a), first(0, 0));
    // At the top PgUp moves nothing and the view stays.
    press(&mut a, key(KeyCode::PageUp), 1);
    assert_eq!(view(&a), first(0, 0));
    // From the bottom row at the end, PgUp keeps the cursor on the bottom row.
    press(&mut a, ctrl_code(KeyCode::End), 1);
    assert_eq!(view(&a), first(25, 4));
    press(&mut a, key(KeyCode::PageUp), 1);
    assert_eq!(view(&a), first(21, 4));
    press(&mut a, key(KeyCode::PageDown), 1);
    assert_eq!(view(&a), first(25, 4));
}

#[test]
fn pgup_and_pgdn_keep_the_goal_column_across_short_rows() {
    // Column 30, a blank line inside the page, column 30 again (nano).
    let text = format!(
        "{}\n\n{}\n{}",
        "a".repeat(40),
        "b".repeat(40),
        "c".repeat(40)
    );
    let mut a = TextArea::editor(&text);
    a.on_editor_key(ctrl_code(KeyCode::Home), 80, 3);
    for _ in 0..30 {
        a.on_editor_key(key(KeyCode::Right), 80, 3);
    }
    assert_eq!(at(&a), (0, 30));
    a.on_editor_key(key(KeyCode::PageDown), 80, 3);
    assert_eq!(at(&a), (2, 30), "two rows down, past the blank line");
    a.on_editor_key(key(KeyCode::PageUp), 80, 3);
    assert_eq!(at(&a), (0, 30));
    // A short row at the end of the page still clamps.
    a.on_editor_key(key(KeyCode::Down), 80, 3);
    a.on_editor_key(key(KeyCode::PageUp), 80, 2);
    assert_eq!(at(&a), (0, 0), "from the blank line, column 0");
    // Display columns, not graphemes: `日本` is four columns wide.
    let mut w = TextArea::editor("日本語x\n\nabcdefgh");
    for _ in 0..2 {
        w.on_editor_key(key(KeyCode::Left), 80, 3);
    }
    assert_eq!(at(&w), (2, 6));
    w.on_editor_key(key(KeyCode::PageUp), 80, 3);
    assert_eq!(at(&w), (0, 3), "column 6 is after `日本語`");
}
