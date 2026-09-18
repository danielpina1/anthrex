use crate::app::App;
use crate::theme;
use proto::Status;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

/// Rows per agent card: name line plus detail line.
pub const CARD_HEIGHT: u16 = 2;

pub fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

fn summary(app: &App) -> String {
    let n = app.windows.len();
    let working = app
        .windows
        .iter()
        .filter(|w| w.status == Status::Working)
        .count();
    let attention = app
        .windows
        .iter()
        .filter(|w| w.status == Status::Attention)
        .count();
    let mut parts = vec![format!("{n} agent{}", if n == 1 { "" } else { "s" })];
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    if attention > 0 {
        parts.push(format!("{attention} attention"));
    }
    parts.join(" · ")
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border())
        .title(Line::from(Span::styled(" anthrex ", theme::title())));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 2 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for (i, w) in app.windows.iter().enumerate() {
        let focused = app.focused == Some(w.id);
        let bar = if focused {
            Span::styled("▎", Style::default().fg(theme::ACCENT))
        } else {
            Span::raw(" ")
        };
        let glyph = Span::styled(
            theme::status_glyph(w.status, app.spinner_frame),
            Style::default().fg(theme::status_color(w.status)),
        );
        let name_style = if focused {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            bar.clone(),
            glyph,
            Span::raw(" "),
            Span::styled(format!("{} {}", i + 1, w.name), name_style),
        ]));
        let detail = format!(
            "  {} · {} · {}",
            w.runtime.label(),
            w.status.label(),
            format_elapsed(app.elapsed_secs(w))
        );
        lines.push(Line::from(vec![bar, Span::styled(detail, theme::muted())]));
    }
    if app.windows.is_empty() {
        lines.push(Line::from(Span::styled(" no agents yet", theme::muted())));
        lines.push(Line::from(Span::styled(
            " C-b c opens a shell",
            theme::muted(),
        )));
    }

    let list = list_area(inner);
    frame.render_widget(Paragraph::new(lines), list);
    let footer = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(summary(app), theme::muted()))),
        footer,
    );
}

/// The area cards are actually drawn into: `inner` minus the trailing spacer and footer rows.
fn list_area(inner: Rect) -> Rect {
    Rect {
        height: inner.height.saturating_sub(2),
        ..inner
    }
}

/// Which card (index into `app.windows`) is under a screen position inside the sidebar.
/// Only positions inside the drawn card list count: the spacer and footer rows below it,
/// and rows past the last fully-drawn card, both return `None`.
pub fn hit_test(inner: Rect, app: &App, column: u16, row: u16) -> Option<usize> {
    let list = list_area(inner);
    if column < list.x
        || column >= list.x + list.width
        || row < list.y
        || row >= list.y + list.height
    {
        return None;
    }
    let index = ((row - list.y) / CARD_HEIGHT) as usize;
    let drawn_cards = (list.height / CARD_HEIGHT) as usize;
    (index < app.windows.len() && index < drawn_cards).then_some(index)
}
