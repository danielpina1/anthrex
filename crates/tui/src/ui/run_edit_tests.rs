//! M8c.9: the task edit form's rendering (Interfaces "The task edit form").

use super::*;
use crate::run_edit::EditField;
use crate::run_edit::tests::edit_fixture_form;
use crate::theme;
use proto::TestMode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

fn draw(form: &TaskEditForm, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render(frame, form, frame.area(), theme::Palette::PLAIN))
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
        .find(|y| line(buffer, *y).contains('┌'))
        .expect("the box");
    let bottom = (top + 1..buffer.area.height)
        .find(|y| line(buffer, *y).contains('└'))
        .expect("the box's bottom");
    let inner = (top + 1..bottom)
        .map(|y| {
            let text = line(buffer, y);
            // Inside the border and the one column of padding.
            let inner: String = text
                .trim()
                .trim_start_matches('│')
                .trim_end_matches('│')
                .to_string();
            let inner = inner.strip_prefix(' ').unwrap_or(&inner).to_string();
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
            "▌ runtime    ‹ claude ›",
            "  model      policy  claude-sonnet-5",
            "  strength   ‹ policy ›  standard",
            "  effort     ‹ medium ›",
            "  size       ‹ M ›",
            "  test mode  ‹ tdd ›",
            "  brief      Line one",
            "             Line two",
            "",
            "",
            "",
            "",
            "",
            "⏎ save · tab next · ←/→ change · ^J newline · esc cancel",
        ]
    );
    // The kit's 64 columns, centred.
    let top_line = line(&buffer, position(&buffer, "┌").1);
    assert_eq!(top_line.trim().chars().count(), 64);
    // The resolved values are muted; the chosen ones are not.
    let muted = theme::role(theme::Role::Muted, theme::Palette::PLAIN).fg;
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
    assert_eq!(rows[6], "▌ reason");
    assert_eq!(rows[7], "  brief      Line one");
    assert_eq!(rows[8], "             Line two");
    assert_eq!(rows[13], "task t1: size: one (+2 more)");
    assert_eq!(rows[14], "");
    assert_eq!(
        rows[15],
        "⏎ save · tab next · ←/→ change · ^J newline · esc cancel"
    );
    assert_eq!(rows.len(), 16);
}

#[test]
fn a_long_error_is_cut_to_the_box() {
    let mut form = edit_fixture_form();
    form.error = Some(format!("{}…", "x".repeat(300)));
    let (_, rows) = rows(&draw(&form, 80, 24));
    let error = &rows[12];
    assert_eq!(error.chars().count(), 60);
    assert!(error.ends_with('…'));
}

#[test]
fn a_submitting_form_says_so() {
    let mut form = edit_fixture_form();
    form.submitting = true;
    let (_, rows) = rows(&draw(&form, 80, 24));
    assert_eq!(rows.last().map(String::as_str), Some("saving… · esc close"));
}

#[test]
fn the_cursor_sits_in_the_focused_text_field() {
    // The brief is a text area: its cursor is a reversed cell after the last line.
    let mut form = edit_fixture_form();
    form.focus = EditField::Brief;
    let buffer = draw(&form, 80, 24);
    let (x, y) = position(&buffer, "Line two");
    assert!(
        buffer[(x + 8, y)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED),
        "after the last character"
    );
    // A one-line field keeps the hardware cursor.
    form.focus = EditField::Model;
    form.model = crate::dialog::TextInput::new("gpt");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| render(frame, &form, frame.area(), theme::Palette::PLAIN))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let (x, y) = position(&buffer, "gpt");
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!((cursor.x, cursor.y), (x + 3, y), "after `gpt`");
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

/// Agent text in the form passes `safe_text`: hostile characters planted at the
/// front of the plan's model, reason and brief, inside the drawn columns, never reach
/// a cell.
#[test]
fn hostile_fields_are_drawn_sanitised() {
    use crate::safe_text::tests::{first_hostile, hostile_text};
    let mut t = crate::run_edit::tests::edit_fixture_task();
    // The model's cursor is at its end, so the field shows its tail.
    t.route_spec.model = Some(format!("{}x\u{200D}y\u{202E}z", hostile_text()));
    t.test_mode = TestMode::Check;
    t.test_mode_reason = Some("r\u{200D}e\u{202E}n".into());
    t.brief = "b\u{200D}r\u{202E}f".into();
    let form = TaskEditForm::new(crate::tree::run_fixtures::RUN_ID, &t);
    let (_, rows) = rows(&draw(&form, 120, 40));
    assert_eq!(first_hostile(&rows.join(" ")), None, "{rows:#?}");
    assert!(
        rows[1].starts_with("  model      ") && rows[1].ends_with(" ab xyz"),
        "{rows:#?}"
    );
    assert_eq!(rows[6], "  reason     ren");
    assert_eq!(rows[7], "  brief      brf");
}

/// Fix round 1 (m3): on a short terminal the hints (with `esc`) and the error stay,
/// and the fields scroll with the focused one in view.
#[test]
fn a_short_terminal_keeps_the_hints_and_the_focused_field() {
    let mut form = crate::run_edit::TaskEditForm::in_run(
        &crate::tree::run_fixtures::gate_fixture().0.runs[0],
        &crate::run_edit::tests::edit_fixture_task(),
    );
    form.error = Some("task t1: size: one".into());
    for field in form.visible_fields() {
        form.focus = field;
        let buffer = draw(&form, 80, 14);
        let text: Vec<String> = (0..14).map(|y| line(&buffer, y)).collect();
        let shown = text.join("\n");
        assert!(shown.contains("esc cancel"), "{field:?}:\n{shown}");
        assert!(shown.contains("task t1: size: one"), "{field:?}:\n{shown}");
        let focused = format!("▌ {}", crate::run_edit::field_label(field));
        assert!(shown.contains(&focused), "{field:?}:\n{shown}");
    }
    assert!(
        form.visible_fields().contains(&EditField::Stage),
        "all eight rows"
    );
    // The model's hardware cursor follows the scroll.
    form.focus = EditField::Model;
    form.model = crate::dialog::TextInput::new("gpt");
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
    terminal
        .draw(|frame| render(frame, &form, frame.area(), theme::Palette::PLAIN))
        .unwrap();
    let (x, y) = position(terminal.backend().buffer(), "gpt");
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!((cursor.x, cursor.y), (x + 3, y));
}

/// Fix round 1 (m6): the hardware cursor's column is measured on the text as drawn,
/// where a hidden character takes no column.
#[test]
fn the_cursor_column_skips_hidden_characters() {
    let mut form = edit_fixture_form();
    form.focus = EditField::Model;
    form.model = crate::dialog::TextInput::new("x\u{200D}y\u{202E}z");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| render(frame, &form, frame.area(), theme::Palette::PLAIN))
        .unwrap();
    let (x, y) = position(terminal.backend().buffer(), "xyz");
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!((cursor.x, cursor.y), (x + 3, y), "after `xyz`");
}
