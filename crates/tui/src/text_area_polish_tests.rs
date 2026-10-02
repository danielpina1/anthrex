//! M9.0.7.13: `TextArea` Up/Down move the cursor a row, at the same column, clamped
//! (decision 35), so a four-row field is edited without another field's arrow keys.

use super::*;
use crate::theme::Palette;
use crate::ui::kit;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Modifier;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// The cursor as `(logical line, grapheme column)`.
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

/// The drawn row and the reversed cell's grapheme of `kit::text_area` at `width`.
fn drawn_cursor(area: &TextArea, rows: u16, width: u16) -> (usize, String) {
    let lines = kit::text_area(area, rows, width, Palette::PLAIN);
    for (row, line) in lines.iter().enumerate() {
        for span in &line.spans {
            if span.style.add_modifier.contains(Modifier::REVERSED) {
                return (row, span.content.to_string());
            }
        }
    }
    panic!("no cursor drawn: {lines:?}");
}

#[test]
fn up_and_down_move_by_a_wrapped_row() {
    // Three lines, the cursor at column 4 of the middle one.
    let mut area = TextArea::from_text("first line\nsecond line\nend");
    assert_eq!(at(&area), (2, 3));
    assert!(area.on_key(key(KeyCode::Up)));
    assert!(area.on_key(key(KeyCode::Right)));
    assert_eq!(at(&area), (1, 4));

    assert!(area.on_key(key(KeyCode::Up)));
    assert_eq!(at(&area), (0, 4), "Up keeps the column");
    assert!(
        !area.on_key(key(KeyCode::Up)),
        "nothing above the first row: the key is the form's"
    );
    assert_eq!(at(&area), (0, 4));

    assert!(area.on_key(key(KeyCode::Down)));
    assert_eq!(at(&area), (1, 4));
    assert!(area.on_key(key(KeyCode::Down)));
    assert_eq!(
        at(&area),
        (2, 3),
        "clamped to the end of a shorter last line"
    );
    assert!(
        !area.on_key(key(KeyCode::Down)),
        "nothing below the last row"
    );
    assert_eq!(at(&area), (2, 3));

    // One logical line wrapped at `kit::text_area`'s width: `abcde` / `fghij` at 6
    // columns (it wraps one column short, so the cursor always has a cell).
    let mut wrapped = TextArea::from_text("abcdefghij");
    for _ in 0..3 {
        wrapped.on_key(key(KeyCode::Left));
    }
    assert_eq!(wrapped.cursor(), 7);
    assert_eq!(drawn_cursor(&wrapped, 2, 6), (1, "h".to_string()));
    assert!(wrapped.on_key_in(key(KeyCode::Up), 6));
    assert_eq!(wrapped.cursor(), 2, "a wrapped row up, the same column");
    assert_eq!(drawn_cursor(&wrapped, 2, 6), (0, "c".to_string()));
    assert!(!wrapped.on_key_in(key(KeyCode::Up), 6));
    assert!(wrapped.on_key_in(key(KeyCode::Down), 6));
    assert_eq!(wrapped.cursor(), 7);

    // From the end of the second row, Up clamps inside the first row: its last
    // column is the second row's start, which would draw the cursor below.
    wrapped.on_key(key(KeyCode::End));
    assert_eq!(drawn_cursor(&wrapped, 2, 6), (1, " ".to_string()));
    assert!(wrapped.on_key_in(key(KeyCode::Up), 6));
    assert_eq!(drawn_cursor(&wrapped, 2, 6), (0, "e".to_string()));

    // Unwrapped, the same text is one row: Up and Down are not the area's.
    let mut one = TextArea::from_text("abcdefghij");
    assert!(!one.on_key(key(KeyCode::Up)));
    assert!(!one.on_key(key(KeyCode::Down)));
    assert_eq!(one.cursor(), 10);
}
