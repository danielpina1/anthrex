//! Milestone 9.3 (task M9.3.9a): the editor's rows and its position row (decision 7,
//! KG §1.2, §1.3), and the hostile-text rule (decision 33).

use super::{editor, editor_position};
use crate::safe_text::tests::first_hostile;
use crate::text_area::TextArea;
use crate::theme::{Palette, Role, role};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

const P: Palette = Palette {
    accent: Color::Blue,
    truecolor: false,
    ascii: false,
};
const ASCII: Palette = Palette {
    accent: Color::Blue,
    truecolor: false,
    ascii: true,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn texts(lines: &[Line]) -> Vec<String> {
    lines.iter().map(text).collect()
}

/// The drawn row and the reversed cell of `lines`.
fn cursor(lines: &[Line]) -> Option<(usize, String)> {
    for (row, line) in lines.iter().enumerate() {
        for span in &line.spans {
            if span.style.add_modifier.contains(Modifier::REVERSED) {
                return Some((row, span.content.to_string()));
            }
        }
    }
    None
}

/// Twelve lines, 1,284 characters with the newlines, the cursor on line 12 column 4.
fn twelve_lines() -> TextArea {
    let mut text = format!("{}\n", "a".repeat(100)).repeat(11);
    text.push_str(&"b".repeat(1_284 - 11 * 101));
    let mut area = TextArea::editor(&text);
    area.on_editor_key(key(KeyCode::Home), 80, 4);
    for _ in 0..3 {
        area.on_editor_key(key(KeyCode::Right), 80, 4);
    }
    area
}

#[test]
fn the_position_row_reads_line_col_and_count() {
    let area = twelve_lines();
    assert_eq!(area.text().chars().count(), 1_284);
    assert_eq!(
        text(&editor_position(&area, 60, P)),
        "ln 12, col 4 · 1,284 / 16,384"
    );
    // The ASCII twin of `·`.
    assert_eq!(
        text(&editor_position(&area, 60, ASCII)),
        "ln 12, col 4 - 1,284 / 16,384"
    );
    // An empty editor, and a count under a thousand without a comma.
    assert_eq!(
        text(&editor_position(&TextArea::editor(""), 60, P)),
        "ln 1, col 1 · 0 / 16,384"
    );
    assert_eq!(
        text(&editor_position(&TextArea::editor("abc"), 60, P)),
        "ln 1, col 4 · 3 / 16,384"
    );
    // Too narrow for all of it: the count stays, the line and column go.
    assert_eq!(text(&editor_position(&area, 20, P)), "1,284 / 16,384");
}

#[test]
fn the_count_turns_to_attention_above_90_percent() {
    let count_style = |n: usize| {
        let area = TextArea::editor(&"x".repeat(n));
        let line = editor_position(&area, 60, P);
        let count = format!("{},{:03}", n / 1000, n % 1000);
        line.spans
            .iter()
            .find(|s| s.content == count.as_str())
            .unwrap_or_else(|| panic!("no {count} span in {line:?}"))
            .style
    };
    // 90% of 16,384 is 14,745.6: 14,745 is not above it, 14,746 is.
    assert_eq!(count_style(14_745), role(Role::Muted, P));
    assert_eq!(count_style(14_746), role(Role::Attention, P));
    assert_eq!(count_style(16_384), role(Role::Attention, P));
    assert_eq!(count_style(1_284), role(Role::Muted, P));
}

#[test]
fn the_cap_text_replaces_the_position() {
    let mut area = TextArea::editor(&"x".repeat(proto::GOAL_MAX_CHARS));
    area.on_editor_key(key(KeyCode::Char('y')), 80, 4);
    assert!(area.at_cap());
    let line = editor_position(&area, 60, P);
    assert_eq!(text(&line), "goal is at its 16,384-character limit");
    assert!(
        line.spans
            .iter()
            .all(|s| s.style == role(Role::Attention, P))
    );
    assert_eq!(
        text(&editor_position(&area, 60, ASCII)),
        "goal is at its 16,384-character limit"
    );
    // The next key that inserts nothing past the cap brings the position back.
    area.on_editor_key(key(KeyCode::Left), 80, 4);
    assert_eq!(
        text(&editor_position(&area, 60, P)),
        "ln 1, col 16384 · 16,384 / 16,384"
    );
}

#[test]
fn editor_rows_are_sanitised() {
    // Decision 33: visible carriers, planted past `TextArea::clean`, inside the drawn
    // columns. Every row passes `one_line` in the renderer itself.
    let area = TextArea::unclean("x\u{200D}y\u{202E}z\nnext");
    let lines = editor(&area, 3, 20, false);
    assert_eq!(texts(&lines), ["xyz", "next", ""]);
    // Focused, the cursor at the end of the second row.
    let lines = editor(&area, 3, 20, true);
    assert_eq!(texts(&lines), ["xyz", "next ", ""]);
    for line in &lines {
        assert_eq!(first_hostile(&text(line)), None, "{line:?}");
    }
    // The cursor on the carriers' row: every piece around it is clean too.
    let mut on_row = TextArea::unclean("x\u{200D}y\u{202E}z");
    on_row.on_editor_key(key(KeyCode::Left), 20, 2);
    let lines = editor(&on_row, 1, 20, true);
    assert_eq!(texts(&lines), ["xyz"]);
    assert_eq!(cursor(&lines), Some((0, "z".to_string())));
    // And the drawn columns of a buffer hold exactly that.
    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
    Paragraph::new(lines).render(buf.area, &mut buf);
    let drawn: String = (0..20).map(|x| buf[(x, 0)].symbol().to_string()).collect();
    assert_eq!(drawn.trim_end(), "xyz");
}

#[test]
fn the_editor_scrolls_to_the_cursor() {
    let text30: Vec<String> = (0..30).map(|i| format!("line{i}")).collect();
    let mut area = TextArea::editor(&text30.join("\n"));
    // The cursor at the end: the last five lines, the cursor on the last row.
    let lines = editor(&area, 5, 40, true);
    assert_eq!(lines.len(), 5, "exactly the rows asked for, no marks");
    assert_eq!(
        texts(&lines),
        ["line25", "line26", "line27", "line28", "line29 "]
    );
    assert_eq!(cursor(&lines), Some((4, " ".to_string())));
    // Ctrl-Home: the first five, the cursor on the first row.
    area.on_editor_key(ctrl_code(KeyCode::Home), 40, 4);
    let lines = editor(&area, 5, 40, true);
    assert_eq!(texts(&lines)[0], "line0");
    assert_eq!(texts(&lines)[4], "line4");
    assert_eq!(cursor(&lines), Some((0, "l".to_string())));
    // A PgDn moves four rows: the cursor's row stays in view.
    area.on_editor_key(key(KeyCode::PageDown), 40, 4);
    area.on_editor_key(key(KeyCode::PageDown), 40, 4);
    let lines = editor(&area, 5, 40, true);
    assert_eq!(texts(&lines)[4], "line8");
    assert_eq!(cursor(&lines), Some((4, "l".to_string())));
    // Unfocused: no cursor drawn.
    assert_eq!(cursor(&editor(&area, 5, 40, false)), None);
    // A short text pads to the rows.
    let short = editor(&TextArea::editor("hi"), 4, 40, false);
    assert_eq!(texts(&short), ["hi", "", "", ""]);
}

#[test]
fn the_editor_draws_the_cursor_where_its_keys_put_it() {
    // One logical line wrapped at 6 columns, one short of the width, as Up and Down
    // move through it: `abcde` / `fghij`.
    let mut area = TextArea::editor("abcdefghij");
    for _ in 0..3 {
        area.on_editor_key(key(KeyCode::Left), 6, 1);
    }
    let lines = editor(&area, 2, 6, true);
    assert_eq!(texts(&lines), ["abcde", "fghij"]);
    assert_eq!(cursor(&lines), Some((1, "h".to_string())));
    area.on_editor_key(key(KeyCode::Up), 6, 1);
    assert_eq!(
        cursor(&editor(&area, 2, 6, true)),
        Some((0, "c".to_string()))
    );
    // The end of a wrapped row is the next row's start.
    area.on_editor_key(key(KeyCode::Up), 6, 1);
    area.on_editor_key(ctrl_code(KeyCode::Home), 6, 1);
    for _ in 0..5 {
        area.on_editor_key(key(KeyCode::Right), 6, 1);
    }
    assert_eq!(
        cursor(&editor(&area, 2, 6, true)),
        Some((1, "f".to_string()))
    );
}
