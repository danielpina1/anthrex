//! Milestone 9 decision 44: rendering for the goal form (`crate::run_goal`, the pure
//! model). A 72-column box titled ` start a goal in <project> `, one row per field,
//! then a blank row, the error row when there is one, and the hint.

use crate::run_goal::{GoalField, GoalForm, field_label};
use crate::theme;
use crate::ui::dialog::{LABEL_WIDTH, MARKER_WIDTH, centered};
use crate::ui::tree_view::truncate;
use proto::Status;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

const FORM_WIDTH: u16 = 72;
const FIELDS: [GoalField; 4] = [
    GoalField::Goal,
    GoalField::Runtime,
    GoalField::Model,
    GoalField::Trust,
];
const HINT: &str = "⏎ start  tab next  ←/→ change  ctrl-j newline  esc cancel";
const SUBMITTING_HINT: &str = "starting… triage can take minutes  esc close";

fn one_row(frame: &mut Frame, inner: Rect, y: u16, line: Line) {
    if y < inner.y + inner.height {
        let row = Rect {
            y,
            height: 1,
            ..inner
        };
        frame.render_widget(Paragraph::new(line), row);
    }
}

pub fn render(frame: &mut Frame, form: &GoalForm, area: Rect, accent: Color) {
    let error_rows = usize::from(form.error.is_some());
    let height = (FIELDS.len() + 1 + error_rows + 1) as u16 + 2;
    let rect = centered(area, FORM_WIDTH.min(area.width), height);
    frame.render_widget(Clear, rect);
    let project = crate::safe_text::one_line(&form.project.display().to_string());
    let title = truncate(&format!(" start a goal in {project} "), rect.width as usize);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused(accent))
        .title(Line::from(Span::styled(title, theme::title(accent))));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let width = inner.width as usize;
    let value_width = width.saturating_sub(MARKER_WIDTH + LABEL_WIDTH);

    for (row, field) in FIELDS.iter().enumerate() {
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
        let input = match field {
            GoalField::Goal => Some(&form.goal),
            GoalField::Model => Some(&form.model),
            GoalField::Runtime | GoalField::Trust => None,
        };
        let mut cursor = None;
        match input.filter(|input| !input.text().is_empty()) {
            Some(input) => {
                let (visible, column) = input.visible(value_width as u16);
                spans.push(Span::raw(visible));
                cursor = Some(column);
            }
            None => {
                let value = form.value_text(*field);
                let style = if input.is_some() {
                    theme::muted()
                } else {
                    Style::default()
                };
                spans.push(Span::styled(truncate(&value, value_width), style));
                if input.is_some() {
                    cursor = Some(0);
                }
            }
        }
        if let Some(column) = cursor.filter(|_| focused && !form.submitting)
            && y < inner.y + inner.height
            && value_width > 0
        {
            let x = inner.x + (MARKER_WIDTH + LABEL_WIDTH) as u16 + column;
            frame.set_cursor_position((x, y));
        }
        one_row(frame, inner, y, Line::from(spans));
    }

    let mut y = inner.y + FIELDS.len() as u16 + 1;
    if let Some(error) = &form.error {
        let style = Style::default().fg(theme::status_color(Status::Attention));
        let error = crate::safe_text::one_line(error);
        one_row(
            frame,
            inner,
            y,
            Line::styled(truncate(&error, width), style),
        );
        y += 1;
    }
    let hint = if form.submitting {
        SUBMITTING_HINT
    } else {
        HINT
    };
    one_row(frame, inner, y, Line::styled(hint, theme::muted()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_goal::GoalForm;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn drawn(form: &GoalForm) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        terminal
            .draw(|frame| render(frame, form, frame.area(), Color::Blue))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..16)
            .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_form_shows_its_project_fields_error_and_hint() {
        let mut form = GoalForm::new("/r/demo".into());
        form.goal = crate::dialog::TextInput::new("add a↵b");
        form.error = Some("run start --goal refused\u{1b}[2J".into());
        let out = drawn(&form);
        assert!(out.contains(" start a goal in /r/demo "), "{out}");
        assert!(out.contains("goal       add a↵b"), "{out}");
        assert!(out.contains("runtime    ‹ configured ›"), "{out}");
        assert!(out.contains("model      default"), "{out}");
        assert!(
            out.contains("[ ] trust the project's own agent settings"),
            "{out}"
        );
        assert!(out.contains("run start --goal refused [2J"), "{out}");
        assert!(out.contains(HINT), "{out}");
    }
}
