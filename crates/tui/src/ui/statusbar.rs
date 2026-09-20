use crate::app::{App, TreeInput};
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

const HINTS: [(&str, &str); 5] = [
    ("C-b ?", "help"),
    ("C-b c", "new shell"),
    ("C-b t", "tree"),
    ("C-b j/k", "switch"),
    ("C-b d", "detach"),
];

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    if app.keymap.pending() {
        spans.push(Span::styled(
            " PREFIX ",
            Style::default()
                .fg(Color::Black)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    } else if let Some(input) = app.tree_input {
        spans.push(Span::styled(
            match input {
                TreeInput::Navigate => " TREE ",
                TreeInput::Filter => " FILTER ",
            },
            Style::default()
                .fg(Color::Black)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    } else if !app.connected {
        spans.push(Span::styled(
            " DISCONNECTED ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    } else {
        spans.push(Span::raw(" "));
    }
    match app.tree_input {
        Some(TreeInput::Navigate) => spans.push(Span::styled(
            "j/k move  ⏎ focus  space fold  / filter  esc back",
            theme::muted(),
        )),
        Some(TreeInput::Filter) => spans.push(Span::styled(
            format!("/{}", app.tree.filter),
            theme::muted(),
        )),
        None => {
            for (key, what) in HINTS {
                spans.push(Span::styled(key, Style::default().fg(theme::ACCENT)));
                spans.push(Span::styled(format!(" {what}  "), theme::muted()));
            }
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    if let Some(text) = app.toast_text() {
        let width = (text.chars().count() as u16 + 1).min(area.width);
        let right = Rect {
            x: area.x + area.width - width,
            width,
            ..area
        };
        let toast = Span::styled(
            text.to_string(),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(Paragraph::new(Line::from(toast)), right);
    }
}
