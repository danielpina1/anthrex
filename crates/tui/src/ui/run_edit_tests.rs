//! M8c.9: the task edit form's rendering (Interfaces "The task edit form").

use super::*;
use crate::run_edit::EditField;
use crate::run_edit::tests::edit_fixture_form;
use proto::TestMode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

fn draw(form: &TaskEditForm, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render(frame, form, frame.area(), theme::DEFAULT_ACCENT))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn line(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// The box's rows, each without its borders and trailing blanks, and its top border.
fn rows(buffer: &Buffer) -> (String, Vec<String>) {
    let top = (0..buffer.area.height)
        .find(|y| line(buffer, *y).contains('╭'))
        .expect("the box");
    let bottom = (top + 1..buffer.area.height)
        .find(|y| line(buffer, *y).contains('╰'))
        .expect("the box's bottom");
    let inner = (top + 1..bottom)
        .map(|y| {
            let text = line(buffer, y);
            let inner: String = text
                .trim()
                .trim_start_matches('│')
                .trim_end_matches('│')
                .to_string();
            inner.trim_end().to_string()
        })
        .collect();
    (line(buffer, top), inner)
}

fn position(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for y in 0..buffer.area.height {
        let text = line(buffer, y);
        if let Some(byte) = text.find(needle) {
            return (text[..byte].chars().count() as u16, y);
        }
    }
    panic!("{needle:?} not drawn");
}

#[test]
fn the_form_renders_its_fields() {
    let form = edit_fixture_form();
    let buffer = draw(&form, 80, 24);
    let (top, rows) = rows(&buffer);
    assert!(top.contains(" edit t1 "), "{top}");
    assert_eq!(
        rows,
        vec![
            "› runtime    ‹ claude ›",
            "  model      policy  claude-sonnet-5",
            "  strength   ‹ policy ›  standard",
            "  effort     ‹ medium ›",
            "  size       ‹ M ›",
            "  test mode  ‹ tdd ›",
            "  brief      Line one↵Line two",
            "",
            "⏎ save  tab next  ←/→ change  esc cancel",
        ]
    );
    // 72 columns wide, centred.
    let top_line = line(&buffer, position(&buffer, "╭").1);
    assert_eq!(top_line.trim().chars().count(), 72);
    // The resolved values are muted; the chosen ones are not.
    let muted = theme::muted().fg;
    let (x, y) = position(&buffer, "standard");
    assert_eq!(buffer[(x, y)].fg, muted.expect("muted has a colour"));
    let (x, y) = position(&buffer, "claude-sonnet-5");
    assert_eq!(buffer[(x, y)].fg, muted.unwrap());
    let (x, y) = position(&buffer, "‹ medium ›");
    assert_ne!(buffer[(x + 2, y)].fg, muted.unwrap());
}

#[test]
fn the_reason_row_and_an_error_row_appear() {
    let mut form = edit_fixture_form();
    form.test_mode = TestMode::Check;
    form.focus = EditField::Reason;
    form.error = Some("task t1: size: one (+2 more)".into());
    let (_, rows) = rows(&draw(&form, 80, 24));
    assert_eq!(rows[5], "  test mode  ‹ check ›");
    assert_eq!(rows[6], "› reason");
    assert_eq!(rows[7], "  brief      Line one↵Line two");
    assert_eq!(rows[8], "");
    assert_eq!(rows[9], "task t1: size: one (+2 more)");
    assert_eq!(rows[10], "⏎ save  tab next  ←/→ change  esc cancel");
    assert_eq!(rows.len(), 11);
}

#[test]
fn a_long_error_is_cut_to_the_box() {
    let mut form = edit_fixture_form();
    form.error = Some(format!("{}…", "x".repeat(300)));
    let (_, rows) = rows(&draw(&form, 80, 24));
    let error = &rows[8];
    assert_eq!(error.chars().count(), 70);
    assert!(error.ends_with('…'));
}

#[test]
fn a_submitting_form_says_so() {
    let mut form = edit_fixture_form();
    form.submitting = true;
    let (_, rows) = rows(&draw(&form, 80, 24));
    assert_eq!(rows.last().map(String::as_str), Some("saving…  esc close"));
}

#[test]
fn the_cursor_sits_in_the_focused_text_field() {
    let mut form = edit_fixture_form();
    form.focus = EditField::Brief;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| render(frame, &form, frame.area(), theme::DEFAULT_ACCENT))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let (x, y) = position(&buffer, "Line one↵");
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!(
        (cursor.x, cursor.y),
        (x + 17, y),
        "after the last character"
    );
}

#[test]
fn no_panic_at_degenerate_sizes() {
    let mut form = edit_fixture_form();
    form.test_mode = TestMode::None;
    form.error = Some("e".into());
    for width in 0..=20 {
        for height in 0..=14 {
            draw(&form, width, height);
        }
    }
}
