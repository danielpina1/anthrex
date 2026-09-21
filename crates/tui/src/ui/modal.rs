use crate::app::Modal;
use crate::theme;
use crate::ui::dialog;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

pub const HELP: &[(&str, &str)] = &[
    ("C-b j / k", "next / previous agent"),
    ("C-b 1-9", "focus agent by number"),
    ("C-b c", "new agent"),
    ("C-b t", "tree mode (j/k, h/l, Enter, Space, /)"),
    ("C-b T", "tree overview (j/k, h/l, wheel, drag)"),
    ("i", "in the overview: show / hide the inspector"),
    ("C-b < / >", "sidebar width"),
    ("C-b x", "kill agent"),
    ("C-b X", "remove agent (and worktree)"),
    ("C-b s", "toggle sidebar"),
    ("C-b d", "detach (agents keep running)"),
    ("C-b Q", "stop daemon and all agents"),
    ("C-b C-b", "send a literal C-b"),
    ("any key", "close this help"),
];

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

pub fn render(frame: &mut Frame, modal: &Modal, area: Rect) {
    match modal {
        Modal::NewAgent(form) => return dialog::render_new_agent(frame, form, area),
        Modal::Remove(confirm) => return dialog::render_remove_confirm(frame, confirm, area),
        Modal::ForceRemove { message, .. } => {
            return dialog::render_force_remove(frame, message, area);
        }
        Modal::Confirm { .. } | Modal::Help => {}
    }
    let (title, body): (&str, Vec<Line>) = match modal {
        Modal::Confirm { message, .. } => (
            " confirm ",
            vec![
                Line::raw(message.clone()),
                Line::raw(""),
                Line::styled("y / Enter = yes    n / Esc = no", theme::muted()),
            ],
        ),
        Modal::Help => (
            " keys ",
            HELP.iter()
                .map(|(key, what)| {
                    Line::from(vec![
                        Span::styled(format!("{key:<11}"), Style::default().fg(theme::ACCENT)),
                        Span::raw(*what),
                    ])
                })
                .collect(),
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
        .border_style(theme::border_focused())
        .title(Line::from(Span::styled(title, theme::title())));
    frame.render_widget(Paragraph::new(body).block(block), rect);
}
