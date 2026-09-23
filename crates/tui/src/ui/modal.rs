use crate::app::{App, Modal};
use crate::dialog::TextInput;
use crate::theme;
use crate::ui::dialog;
use proto::Status;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

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
        (format!("{prefix_label} ,"), "rename agent".to_string()),
        (format!("{prefix_label} R"), "restart agent".to_string()),
        (format!("{prefix_label} r"), "reconnect".to_string()),
        (format!("{prefix_label} m"), "conversation".to_string()),
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

/// `input`'s text with a solid block drawn at the cursor's grapheme position — the
/// rename box's own way of showing a cursor (task M6.10 brief's mock), rather than the
/// new-agent form's hardware cursor (`ui/dialog.rs`'s `render_new_agent`), because this
/// box is a single line inside the generic (title, body) modal path below, which has
/// nowhere to report a cursor position back to.
fn text_with_cursor_block(input: &TextInput) -> String {
    let graphemes: Vec<&str> = input.text().graphemes(true).collect();
    let cursor = input.cursor().min(graphemes.len());
    let mut out = String::with_capacity(input.text().len() + 3);
    out.push_str(&graphemes[..cursor].concat());
    out.push('█');
    out.push_str(&graphemes[cursor..].concat());
    out
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
        Modal::Confirm { .. } | Modal::Help | Modal::Notice { .. } | Modal::Rename(_) => {}
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
        // Task M6.10 brief's mock: the input line, then the error (or a blank line
        // when there is none, keeping the box a constant three body lines whether or
        // not `error` is set), then the hint.
        Modal::Rename(prompt) => (
            " rename ".to_string(),
            vec![
                Line::raw(text_with_cursor_block(&prompt.input)),
                match &prompt.error {
                    Some(message) => Line::styled(
                        message.clone(),
                        Style::default().fg(theme::status_color(Status::Attention)),
                    ),
                    None => Line::raw(""),
                },
                Line::styled("Enter = rename    Esc = cancel", theme::muted()),
            ],
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
