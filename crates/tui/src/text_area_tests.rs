use super::*;
use crate::ui::kit;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Color;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn type_str(area: &mut TextArea, s: &str) {
    for c in s.chars() {
        area.on_key(key(KeyCode::Char(c)));
    }
}

#[test]
fn typing_and_ctrl_j_make_two_lines() {
    let mut a = TextArea::new();
    type_str(&mut a, "ab");
    assert!(a.on_key(ctrl('j')));
    type_str(&mut a, "cd");
    assert_eq!(a.text(), "ab\ncd");
    assert_eq!(a.lines().count(), 2);
    // Enter belongs to the form.
    assert!(!a.on_key(key(KeyCode::Enter)));
    assert_eq!(a.text(), "ab\ncd");
}

#[test]
fn backspace_joins_lines() {
    let mut a = TextArea::new();
    type_str(&mut a, "ab");
    a.on_key(ctrl('j'));
    type_str(&mut a, "c");
    a.on_key(key(KeyCode::Left));
    a.on_key(key(KeyCode::Backspace));
    assert_eq!(a.text(), "abc");
    a.on_key(key(KeyCode::Home));
    a.on_key(key(KeyCode::Delete));
    assert_eq!(a.text(), "bc");
    a.on_key(key(KeyCode::End));
    type_str(&mut a, "!");
    assert_eq!(a.text(), "bc!");
}

#[test]
fn paste_keeps_newlines_and_drops_controls() {
    let mut a = TextArea::new();
    a.on_paste("one\r\ntwo\x1b[31m\u{202e}\u{7}\nthree\tx");
    assert_eq!(a.text(), "one\ntwo[31m\nthree x");
    assert_eq!(a.cursor(), a.text().chars().count());
}

#[test]
fn it_stops_at_text_max_chars() {
    let mut a = TextArea::new();
    a.on_paste(&"y".repeat(TEXT_MAX_CHARS + 10));
    assert_eq!(a.text().chars().count(), TEXT_MAX_CHARS);
    type_str(&mut a, "z");
    a.on_key(ctrl('j'));
    assert_eq!(a.text().chars().count(), TEXT_MAX_CHARS);
}

#[test]
fn rendering_keeps_the_cursor_row_visible() {
    let p = crate::theme::Palette {
        accent: Color::Blue,
        truecolor: false,
    };
    let mut a = TextArea::new();
    for i in 0..12 {
        if i > 0 {
            a.on_key(ctrl('j'));
        }
        type_str(&mut a, &format!("line{i}"));
    }
    let lines = kit::text_area(&a, 4, 30, p);
    let texts: Vec<String> = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    // Four text rows, plus the mark above; the marks are not counted in `rows`.
    assert_eq!(texts.len(), 5);
    assert_eq!(texts[0], "↑ 8 more");
    assert!(texts[1].starts_with("line8"));
    assert!(texts[4].starts_with("line11"));
}

#[test]
fn a_short_text_pads_to_the_rows_and_a_long_line_wraps() {
    let p = crate::theme::Palette {
        accent: Color::Blue,
        truecolor: false,
    };
    let mut a = TextArea::new();
    a.on_paste("abcdefghij");
    a.on_key(key(KeyCode::Home));
    let lines = kit::text_area(&a, 4, 6, p);
    assert_eq!(lines.len(), 4);
    let first: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(first, "abcde");
}
