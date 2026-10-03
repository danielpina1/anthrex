use super::{Layout, kit, tree_view};
use crate::app::{App, region::KeyRegion};
use crate::{theme, tree};
use proto::Status;
use ratatui::{
    Frame,
    text::{Line, Span},
    widgets::Paragraph,
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
    let sep = theme::fold(" · ", app.palette().ascii);
    while !parts.is_empty()
        && UnicodeWidthStr::width(parts.join(&sep).as_str()) > usize::from(width)
    {
        parts.pop();
    }
    parts.join(&sep)
}

/// The agents block's title beside a top mark of `mark` columns (0 for none) in an
/// interior `interior` wide, each with its own space either side. The mark is kept
/// whole (principle 6); ` · tree` goes first (the ` TREE ` badge names the mode), then
/// `agents` is cut with `…`.
fn title_beside(tree: bool, mark: usize, interior: u16, ascii: bool) -> String {
    let marked = if mark > 0 { mark + 2 } else { 0 };
    let room = usize::from(interior).saturating_sub(marked + 2);
    let full = if tree { "agents · tree" } else { "agents" };
    if full.width() <= room {
        full.to_owned()
    } else if "agents".width() <= room {
        "agents".to_owned()
    } else {
        tree_view::truncate_in("agents", room, ascii)
    }
}

pub fn render(frame: &mut Frame, app: &App, layout: &Layout) {
    let keys_here = app.key_region() == KeyRegion::Sidebar;
    let p = app.palette();
    let muted = theme::role(theme::Role::Muted, p);
    let rows = app.rows();
    let geometry = tree_view::geometry(layout.sidebar_list, rows.len(), app.tree.sidebar.top);
    // Decision 27: rows above and below the list are counted in the border, muted, so
    // the list keeps every interior row and `TreeGeometry` keeps its meaning.
    let below = rows.len() - geometry.first - geometry.count;
    let (up, down) = kit::scroll_marks(geometry.first, below, p.ascii);
    let mark = |text: String| Line::from(Span::styled(format!(" {text} "), muted)).right_aligned();
    let title = title_beside(
        app.tree_input.is_some(),
        up.as_deref().map_or(0, UnicodeWidthStr::width),
        layout.sidebar.width.saturating_sub(2),
        p.ascii,
    );
    let mut block = kit::pane_frame(Line::from(title), keys_here, p);
    if let Some(up) = up {
        block = block.title_top(mark(up));
    }
    if let Some(down) = down {
        block = block.title_bottom(mark(down));
    }
    frame.render_widget(block, layout.sidebar);
    let pos_width = tree::numbered_order(&rows).len().max(1).to_string().len();
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
        lines.push(Line::from(Span::styled(" no agents yet", muted)));
        lines.push(Line::from(Span::styled(
            format!(" {} c opens a shell", app.settings.prefix_label),
            muted,
        )));
    }
    frame.render_widget(Paragraph::new(lines), geometry.list);
    // Final fix wave M3: the selected row's bar on the block's left border.
    let shown = &rows[geometry.first..geometry.first + geometry.count];
    if let Some(at) = shown
        .iter()
        .position(|row| app.tree.selected.as_ref() == Some(&row.key))
        && app.tree_input.is_some()
    {
        let y = geometry.list.y + u16::try_from(at).unwrap_or(u16::MAX);
        kit::selection_bar(frame.buffer_mut(), layout.sidebar.x, y, keys_here, p);
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            summary(app, layout.sidebar_footer.width),
            muted,
        ))),
        layout.sidebar_footer,
    );
}

#[cfg(test)]
#[path = "sidebar_tests.rs"]
mod tests;
