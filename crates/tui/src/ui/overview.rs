use super::tree_view::{self, WideColumns};
use crate::{app::App, theme};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if app.modal.is_none() {
            theme::border_focused()
        } else {
            theme::border()
        })
        .title(Line::from(Span::styled(" tree overview ", theme::title())));
    let inner = super::inset(area);
    frame.render_widget(block, area);

    let rows = app.rows();
    let geometry = tree_view::geometry(inner, rows.len(), app.tree.overview.top);
    let columns = WideColumns::from_rows(&rows);
    let lines: Vec<_> = rows[geometry.first..geometry.first + geometry.count]
        .iter()
        .map(|row| {
            tree_view::wide_line(
                app,
                row,
                geometry.list.width,
                &columns,
                app.tree_input.is_some() && app.tree.selected.as_ref() == Some(&row.key),
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), geometry.list);
}
