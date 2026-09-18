use crate::app::App;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

const HINTS: [(&str, &str); 5] = [("C-b ?", "help"), ("C-b c", "new shell"), ("C-b j/k", "switch"), ("C-b x", "kill"), ("C-b d", "detach")];

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    if app.keymap.pending() {
        spans.push(Span::styled(" PREFIX ", Style::default().fg(Color::Black).bg(theme::ACCENT).add_modifier(Modifier::BOLD)));
        spans.push(Span::raw(" "));
    } else if !app.connected {
        spans.push(Span::styled(" DISCONNECTED ", Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)));
        spans.push(Span::raw(" "));
    } else {
        spans.push(Span::raw(" "));
    }
    for (key, what) in HINTS {
        spans.push(Span::styled(key, Style::default().fg(theme::ACCENT)));
        spans.push(Span::styled(format!(" {what}  "), theme::muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    if let Some(text) = app.toast_text() {
        let width = (text.chars().count() as u16 + 1).min(area.width);
        let right = Rect { x: area.x + area.width - width, width, ..area };
        let toast = Span::styled(text.to_string(), Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD));
        frame.render_widget(Paragraph::new(Line::from(toast)), right);
    }
}
