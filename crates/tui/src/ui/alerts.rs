//! Milestone 9.0.5 decisions 19 and 20: the Alerts box at the bottom of the sidebar
//! column. One line an alert, `● <label>  <text>`, most urgent first; `no alerts` when
//! there is none; `+<k> more` on the last row when they do not fit and the box is not
//! focused, and a window that keeps the selection in view when it is. The alerts are
//! `app::alerts`'s, recomputed here on every draw. Pure: rendering takes `&App`.

use super::{Layout, kit};
use crate::app::{Alert, App, alerts, region::KeyRegion};
use crate::safe_text::one_line;
use crate::theme;
use crate::ui::tree_view::truncate;
use ratatui::Frame;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

/// Decision 20: the box never lists more than this many rows.
pub const ALERTS_MAX_ROWS: u16 = 6;

/// `Alerts (<n>)`.
pub(crate) fn title(n: usize) -> String {
    format!("Alerts ({n})")
}

/// One alert's row, cut to `width` columns: the `●` and the label in the priority's
/// colour, then two spaces and the text in the default colour; reversed when selected.
fn alert_line(alert: &Alert, width: usize, selected: bool) -> Line<'static> {
    let colour = Style::default().fg(theme::alert_color(alert.priority));
    let reversed = |style: Style| {
        if selected {
            style.add_modifier(Modifier::REVERSED)
        } else {
            style
        }
    };
    let mut spans = Vec::new();
    let mut left = width;
    let mut push = |text: String, style: Style, left: &mut usize| {
        if *left == 0 || text.is_empty() {
            return;
        }
        let text = truncate(&text, *left);
        *left -= text.width();
        spans.push(Span::styled(text, reversed(style)));
    };
    push("● ".to_owned(), colour, &mut left);
    push(one_line(&alert.label), colour, &mut left);
    // The text only when some of it shows after its two-space gap.
    if left > 2 {
        push(
            format!("  {}", one_line(&alert.text)),
            Style::default(),
            &mut left,
        );
    }
    Line::from(spans)
}

/// Decision 20's rows for the box's interior, `width` × `rows`.
pub(crate) fn lines(app: &App, width: u16, rows: u16) -> Vec<Line<'static>> {
    let (width, rows) = (usize::from(width), usize::from(rows));
    if width == 0 || rows == 0 {
        return Vec::new();
    }
    let all = alerts(app);
    if all.is_empty() {
        return vec![Line::from(Span::styled(
            truncate("no alerts", width),
            theme::muted(),
        ))];
    }
    let focus = app.alerts_focus.as_ref();
    let selected = focus
        .and_then(|focus| focus.selected.as_ref())
        .and_then(|key| all.iter().position(|alert| alert.key == *key));
    if let Some(_focus) = focus {
        // Focused: a window of `rows` alerts that keeps the selection in view.
        let at = selected.unwrap_or(0);
        let first = at.saturating_sub(rows - 1);
        return all
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .map(|(n, alert)| alert_line(alert, width, Some(n) == selected))
            .collect();
    }
    if all.len() <= rows {
        return all
            .iter()
            .map(|alert| alert_line(alert, width, false))
            .collect();
    }
    // One row: the most urgent alert, not a `+<k> more` alone (the title has the
    // count).
    let shown = (rows - 1).max(1);
    let mut out: Vec<Line<'static>> = all
        .iter()
        .take(shown)
        .map(|alert| alert_line(alert, width, false))
        .collect();
    if shown < rows {
        let more = format!("+{} more", all.len() - shown);
        out.push(Line::from(Span::styled(
            truncate(&more, width),
            theme::muted(),
        )));
    }
    out
}

pub fn render(frame: &mut Frame, app: &App, layout: &Layout) {
    if layout.alerts.height == 0 || layout.alerts.width == 0 {
        return;
    }
    let n = alerts(app).len();
    // Until task 6's Alerts view, the box itself takes the keys under `C-b a`.
    let keys_here = app.key_region() == KeyRegion::Alerts;
    let block = kit::pane_frame(Line::from(title(n)), keys_here, app.palette());
    frame.render_widget(block, layout.alerts);
    let inner = layout.alerts_inner;
    frame.render_widget(Paragraph::new(lines(app, inner.width, inner.height)), inner);
}

#[cfg(test)]
#[path = "alerts_tests.rs"]
mod tests;
