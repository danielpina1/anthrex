use super::{Layout, tree_view};
use crate::{app::App, theme, tree};
use proto::Status;
use ratatui::{
    Frame,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use unicode_width::UnicodeWidthStr;

fn summary(app: &App, width: u16) -> String {
    let n = app.windows.len();
    let mut parts = Vec::new();
    if n > 0 {
        parts.push(format!("{n} agent{}", if n == 1 { "" } else { "s" }));
    }
    for (status, label) in [
        (Status::Working, "working"),
        (Status::Attention, "attention"),
    ] {
        let count = app
            .windows
            .iter()
            .filter(|window| window.status == status)
            .count();
        if count > 0 {
            parts.push(format!("{count} {label}"));
        }
    }
    while !parts.is_empty()
        && UnicodeWidthStr::width(parts.join(" · ").as_str()) > usize::from(width)
    {
        parts.pop();
    }
    parts.join(" · ")
}

pub fn render(frame: &mut Frame, app: &App, layout: &Layout) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if app.tree_input.is_some() {
            theme::border_focused(app.settings.accent)
        } else {
            theme::border()
        })
        .title(Line::from(Span::styled(
            if app.tree_input.is_some() {
                " agents · tree "
            } else {
                " agents "
            },
            theme::title(app.settings.accent),
        )));
    frame.render_widget(block, layout.sidebar);
    let rows = app.rows();
    let geometry = tree_view::geometry(layout.sidebar_list, rows.len(), app.tree.sidebar.top);
    let pos_width = tree::agent_order(&rows).len().max(1).to_string().len();
    let mut lines: Vec<_> = rows[geometry.first..geometry.first + geometry.count]
        .iter()
        .map(|row| {
            tree_view::narrow_line(
                app,
                row,
                geometry.list.width,
                pos_width,
                app.tree_input.is_some() && app.tree.selected.as_ref() == Some(&row.key),
            )
        })
        .collect();
    if app.windows.is_empty() {
        lines.push(Line::from(Span::styled(" no agents yet", theme::muted())));
        lines.push(Line::from(Span::styled(
            " C-b c opens a shell",
            theme::muted(),
        )));
    }
    frame.render_widget(Paragraph::new(lines), geometry.list);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            summary(app, layout.sidebar_footer.width),
            theme::muted(),
        ))),
        layout.sidebar_footer,
    );
}
