//! Milestone 8c decision 33: rendering for the plan gate's task edit form
//! (`crate::run_edit`, the pure model). A 72-column box titled ` edit <task> `, one row
//! per visible field, then a blank row, the error row when there is one, and the hint.

use crate::run_edit::{EditField, TaskEditForm, field_label};
use crate::theme;
use crate::ui::dialog::{LABEL_WIDTH, MARKER_WIDTH, centered};
use crate::ui::tree_view::truncate;
use proto::Status;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

#[cfg(test)]
#[path = "run_edit_tests.rs"]
mod tests;

const FORM_WIDTH: u16 = 72;
const HINT: &str = "⏎ save  tab next  ←/→ change  esc cancel";
const SUBMITTING_HINT: &str = "saving…  esc close";

fn is_text(field: EditField) -> bool {
    matches!(
        field,
        EditField::Model | EditField::Reason | EditField::Brief
    )
}

fn one_row(frame: &mut Frame, inner: Rect, y: u16, line: Line) {
    if y < inner.y + inner.height {
        frame.render_widget(
            Paragraph::new(line),
            Rect {
                y,
                height: 1,
                ..inner
            },
        );
    }
}

pub fn render(frame: &mut Frame, form: &TaskEditForm, area: Rect, accent: Color) {
    let fields = form.visible_fields();
    let error_rows = usize::from(form.error.is_some());
    let height = (fields.len() + 1 + error_rows + 1) as u16 + 2;
    let rect = centered(area, FORM_WIDTH.min(area.width), height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused(accent))
        .title(Line::from(Span::styled(
            format!(" edit {} ", form.task_id),
            theme::title(accent),
        )));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let width = inner.width as usize;
    let value_width = width.saturating_sub(MARKER_WIDTH + LABEL_WIDTH);

    let mut cursor = None;
    for (row, field) in fields.iter().enumerate() {
        let y = inner.y + row as u16;
        let focused = form.focus == *field;
        let label_style = if focused {
            Style::default().fg(accent)
        } else {
            Style::default()
        };
        let mut spans = vec![
            Span::styled(if focused { "› " } else { "  " }, label_style),
            Span::styled(
                format!("{:<LABEL_WIDTH$}", field_label(*field)),
                label_style,
            ),
        ];
        let (value, resolved) = form.value_parts(*field);
        let input = match field {
            EditField::Model => Some(&form.model),
            EditField::Reason => Some(&form.reason),
            EditField::Brief => Some(&form.brief),
            _ => None,
        };
        match input.filter(|input| !input.text().is_empty()) {
            Some(input) => {
                let (visible, column) = input.visible(value_width as u16);
                spans.push(Span::raw(visible));
                if focused {
                    cursor = Some(column);
                }
            }
            None => {
                spans.push(Span::raw(truncate(&value, value_width)));
                if let Some(resolved) = resolved {
                    let room = value_width.saturating_sub(value.chars().count() + 2);
                    if room > 0 {
                        spans.push(Span::raw("  "));
                        spans.push(Span::styled(truncate(&resolved, room), theme::muted()));
                    }
                }
                if focused && is_text(*field) {
                    cursor = Some(0);
                }
            }
        }
        if let Some(column) = cursor.filter(|_| focused)
            && y < inner.y + inner.height
            && value_width > 0
        {
            let x = inner.x + (MARKER_WIDTH + LABEL_WIDTH) as u16 + column;
            frame.set_cursor_position((x, y));
        }
        one_row(frame, inner, y, Line::from(spans));
    }

    let mut y = inner.y + fields.len() as u16 + 1;
    if let Some(error) = &form.error {
        let style = Style::default().fg(theme::status_color(Status::Attention));
        one_row(frame, inner, y, Line::styled(truncate(error, width), style));
        y += 1;
    }
    let hint = if form.submitting {
        SUBMITTING_HINT
    } else {
        HINT
    };
    one_row(frame, inner, y, Line::styled(hint, theme::muted()));
}
