use crate::app::{App, Modal};
use crate::theme;
use crate::ui::dialog;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

/// The help overlay's rows. Every hint that names the prefix key takes it from
/// `prefix_label` instead of a hard-coded `C-b` (decision 38).
fn help_lines(prefix_label: &str) -> Vec<(String, String)> {
    vec![
        (
            format!("{prefix_label} j / k"),
            "next / previous agent".to_string(),
        ),
        (
            format!("{prefix_label} 1-9"),
            "focus agent by number".to_string(),
        ),
        (format!("{prefix_label} c"), "new agent".to_string()),
        (
            format!("{prefix_label} t"),
            "tree mode (j/k, h/l, Enter, Space, /)".to_string(),
        ),
        (
            format!("{prefix_label} T"),
            "tree overview (j/k, h/l, wheel, drag)".to_string(),
        ),
        (
            "i".to_string(),
            "in the overview: show / hide the inspector".to_string(),
        ),
        (format!("{prefix_label} < / >"), "sidebar width".to_string()),
        (format!("{prefix_label} x"), "kill agent".to_string()),
        (
            format!("{prefix_label} X"),
            "remove agent (and worktree)".to_string(),
        ),
        (format!("{prefix_label} s"), "toggle sidebar".to_string()),
        (
            format!("{prefix_label} d"),
            "detach (agents keep running)".to_string(),
        ),
        (
            format!("{prefix_label} Q"),
            "stop daemon and all agents".to_string(),
        ),
        (
            format!("{prefix_label} {prefix_label}"),
            format!("send a literal {prefix_label}"),
        ),
        ("any key".to_string(), "close this help".to_string()),
    ]
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(modal) = &app.modal else {
        return;
    };
    let accent = app.settings.accent;
    match modal {
        Modal::NewAgent(form) => return dialog::render_new_agent(frame, form, area, accent),
        Modal::Remove(confirm) => {
            return dialog::render_remove_confirm(frame, confirm, area, accent);
        }
        Modal::ForceRemove { name, message, .. } => {
            return dialog::render_force_remove(frame, name, message, area, accent);
        }
        Modal::Confirm { .. } | Modal::Help | Modal::Notice { .. } => {}
    }
    let (title, body): (String, Vec<Line>) = match modal {
        Modal::Confirm { message, .. } => (
            " confirm ".to_string(),
            vec![
                Line::raw(message.clone()),
                Line::raw(""),
                Line::styled("y / Enter = yes    n / Esc = no", theme::muted()),
            ],
        ),
        Modal::Help => (
            " keys ".to_string(),
            help_lines(&app.settings.prefix_label)
                .into_iter()
                .map(|(key, what)| {
                    Line::from(vec![
                        Span::styled(format!("{key:<11}"), Style::default().fg(accent)),
                        Span::raw(what),
                    ])
                })
                .collect(),
        ),
        Modal::Notice { title, lines } => (
            title.clone(),
            lines.iter().map(|line| Line::raw(line.clone())).collect(),
        ),
        Modal::NewAgent(_) | Modal::Remove(_) | Modal::ForceRemove { .. } => {
            unreachable!("handled and returned from above")
        }
    };
    let width = body.iter().map(Line::width).max().unwrap_or(0).max(30) as u16 + 4;
    let height = body.len() as u16 + 2;
    let rect = centered(area, width, height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused(accent))
        .title(Line::from(Span::styled(title, theme::title(accent))));
    frame.render_widget(Paragraph::new(body).block(block), rect);
}
