//! The Alerts box at the bottom of the sidebar column (milestone 9.0.5 decisions 19
//! and 20; milestone 9.0.7 decisions 9 and 10). Two lines an alert, most urgent first:
//! who it is for (`⚑ Add mul() · 0723 › t2  41s`), then what happened, wrapped; `no
//! alerts` when there is none; `↓ <k> more` on the last row when they do not fit. The
//! box grows into the column's free rows (`ui::layout_for`, decision 9). The alerts
//! are `app::alerts`'s, built once a frame (`Shown`). The box is never focused:
//! `C-b a` opens the Alerts view (`ui/alerts_view.rs`). Pure: rendering takes `&App`.

use super::{Layout, kit};
use crate::app::{Alert, AlertWho, App, alerts};
use crate::safe_text::one_line;
use crate::theme::{self, Glyph, Palette, Role, fold, glyph};
use crate::tree::format_elapsed;
use crate::ui::tree_view::truncate_in;
use ratatui::Frame;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

/// An alert's text takes at most this many lines under its who line (decision 10).
const TEXT_LINES: usize = 2;

/// Decision 8: `now` under ten seconds, else `41s`, `2m`, `3h`.
pub(crate) fn age_text(secs: u64) -> String {
    if secs < 10 {
        "now".to_owned()
    } else {
        format_elapsed(secs)
    }
}

/// ` ⚑ Alerts <n> ` in `Attention`, or ` Alerts ` muted with none (decision 10).
fn title(n: usize, p: Palette) -> Line<'static> {
    if n == 0 {
        return Line::from(Span::styled("Alerts", theme::role(Role::Muted, p)));
    }
    let text = format!("{} Alerts {n}", glyph(Glyph::NeedsYou, p.ascii));
    Line::from(Span::styled(text, theme::role(Role::Attention, p)))
}

/// The who, in `room` columns: a run's name (`kit::run_name_in`, the goal cut before
/// the short id), or a project's directory name. Where the goal would get no column
/// beside the id, the name's head is kept instead.
pub(crate) fn who_text(who: &AlertWho, room: usize, p: Palette) -> String {
    match who {
        AlertWho::Run { goal, id } => {
            let width = u16::try_from(room).unwrap_or(u16::MAX);
            let name = kit::run_name_in(goal, id, width, p);
            if name.starts_with(' ') || name.width() > room {
                truncate_in(&kit::run_name_in(goal, id, u16::MAX, p), room, p.ascii)
            } else {
                name
            }
        }
        AlertWho::Project(name) => truncate_in(&fold(&one_line(name), p.ascii), room, p.ascii),
    }
}

/// Decision 10: one alert's lines at `width` columns. Line 1: the priority's glyph,
/// the who, ` › <t>` for a task alert, and the age flush right after at least one
/// space; the who is cut first, the task and the age never (but at a width that
/// cannot hold them). Then two spaces and the text, wrapped at `width − 2`, at most
/// two lines, the second cut with `…`. Glyph, who and task in the priority's style
/// (`theme::alert_style`), the age muted, the text in the default colour. Every
/// string passes `safe_text::one_line` here, whatever `app::alerts` did.
pub(crate) fn alert_lines(app: &App, alert: &Alert, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width);
    if width == 0 {
        return Vec::new();
    }
    let p = app.palette();
    let style = theme::alert_style(alert.priority, p);
    let mark = format!("{} ", theme::alert_glyph(alert.priority, p.ascii));
    let task = alert.task.as_deref().map_or_else(String::new, |t| {
        format!(" {} {}", glyph(Glyph::Separator, p.ascii), one_line(t))
    });
    let age = alert.age.map(age_text);
    let age_room = age.as_ref().map_or(0, |age| age.width() + 1);
    // Milestone 9.9 decision 23: the `you` badge, after the who, on the first line.
    let you = if alert.you { " you" } else { "" };
    let room = width.saturating_sub(mark.width() + task.width() + age_room + you.len());
    let who = who_text(&alert.who, room, p);
    let mut parts = vec![(mark, style), (who, style), (task, style)];
    if alert.you {
        parts.push((you.to_owned(), theme::role(Role::Attention, p)));
    }
    if let Some(age) = age {
        let used: usize = parts.iter().map(|(text, _)| text.width()).sum();
        let gap = width.saturating_sub(used + age.width()).max(1);
        parts.push((" ".repeat(gap), Style::default()));
        parts.push((age, theme::role(Role::Muted, p)));
    }
    let mut lines = vec![fitted(parts, width, p.ascii)];
    let text_width = width.saturating_sub(2);
    if text_width == 0 {
        return lines;
    }
    let text = fold(&one_line(&alert.text), p.ascii);
    let mut wrapped = kit::wrap_words(&text, text_width);
    if wrapped.len() > TEXT_LINES {
        // More follows the last line shown: it ends in the mark, cut to make room.
        // Its own text only — a word broken across lines never gains a space.
        wrapped.truncate(TEXT_LINES);
        let last = wrapped.pop().unwrap_or_default();
        let mark = theme::ellipsis(p);
        wrapped.push(truncate_in(&format!("{last}{mark}"), text_width, p.ascii));
    }
    lines.extend(
        wrapped
            .into_iter()
            .filter(|line| !line.is_empty())
            .map(|line| Line::from(vec![Span::raw("  "), Span::raw(line)])),
    );
    lines
}

/// `parts` as one line of at most `width` columns, cut from the right.
pub(crate) fn fitted(parts: Vec<(String, Style)>, width: usize, ascii: bool) -> Line<'static> {
    let mut left = width;
    let mut spans = Vec::new();
    for (text, style) in parts {
        if left == 0 || text.is_empty() {
            continue;
        }
        let text = truncate_in(&text, left, ascii);
        left = left.saturating_sub(text.width());
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

/// One frame's alerts and, while the box shows, each one's decision-10 lines at the
/// box's interior width: built once by `ui::draw` and shared by the sidebar's sizing,
/// the box, the Alerts view and the status bar's flag (final fix wave, task 5's
/// deferred minor).
pub(crate) struct Shown {
    pub all: Vec<Alert>,
    /// Each alert's lines, in `all`'s order; empty when `of` had no width.
    blocks: Vec<Vec<Line<'static>>>,
}

impl Shown {
    /// The alerts, and their lines at `width` when the box is drawn.
    pub(crate) fn of(app: &App, width: Option<u16>) -> Self {
        let all = alerts(app);
        let blocks = width.map_or_else(Vec::new, |width| {
            all.iter()
                .map(|alert| alert_lines(app, alert, width))
                .collect()
        });
        Shown { all, blocks }
    }

    /// Decision 9's `C`: every alert's line count, summed.
    pub(crate) fn lines(&self) -> usize {
        self.blocks.iter().map(Vec::len).sum()
    }
}

/// `lines_of` for `app`'s alerts, for the tests.
#[cfg(test)]
pub(crate) fn lines(app: &App, width: u16, rows: u16) -> Vec<Line<'static>> {
    lines_of(&Shown::of(app, Some(width)), width, rows, app.palette())
}

/// Decision 10's rows for the box's interior, `width` × `rows`, from a frame's
/// [`Shown`] built at `width`: whole alerts only. When they do not fit, the interior's
/// last row is `↓ <k> more` (muted), the rows a whole alert would not fill left blank
/// above it — unless no whole alert would then show and the first fits alone: it
/// shows, the title has the count.
fn lines_of(shown: &Shown, width: u16, rows: u16, p: Palette) -> Vec<Line<'static>> {
    let rows = usize::from(rows);
    if width == 0 || rows == 0 {
        return Vec::new();
    }
    let muted = theme::role(Role::Muted, p);
    let all = &shown.all;
    if all.is_empty() {
        let text = truncate_in("no alerts", usize::from(width), p.ascii);
        return vec![Line::from(Span::styled(text, muted))];
    }
    let blocks = shown.blocks.clone();
    // How many alerts, from the first, fit `budget` rows whole.
    let fit = |budget: usize| {
        let mut used = 0;
        blocks
            .iter()
            .take_while(|block| {
                used += block.len();
                used <= budget
            })
            .count()
    };
    let shown = fit(rows);
    if shown == blocks.len() || (fit(rows - 1) == 0 && shown > 0) {
        return blocks.into_iter().take(shown).flatten().collect();
    }
    let shown = fit(rows - 1);
    let (_, more) = kit::scroll_marks(0, all.len() - shown, p.ascii);
    let more = truncate_in(&more.unwrap_or_default(), usize::from(width), p.ascii);
    let mut out: Vec<Line<'static>> = blocks.into_iter().take(shown).flatten().collect();
    out.resize(rows - 1, Line::default());
    out.push(Line::from(Span::styled(more, muted)));
    out
}

pub fn render(frame: &mut Frame, app: &App, layout: &Layout) {
    let shown = Shown::of(app, Some(layout.alerts_inner.width));
    render_shown(frame, app, layout, &shown);
}

/// [`render`] with the frame's alerts, built at the box's interior width.
pub(crate) fn render_shown(frame: &mut Frame, app: &App, layout: &Layout, shown: &Shown) {
    if layout.alerts.height == 0 || layout.alerts.width == 0 {
        return;
    }
    let p = app.palette();
    let n = shown.all.len();
    // Decisions 1 and 11: the box is never focused and never accented; `C-b a` opens
    // the Alerts view in the main pane, which has the keys.
    let mut block = kit::pane_frame(title(n, p), false, p);
    if n > 0 && app.alerts_focus.is_none() {
        // ` <prefix> a open `: the key in the accent, the word muted, and one border
        // cell before the corner (§6.1's mockup).
        let key = fold(
            &one_line(&format!("{} a", app.settings.prefix_label)),
            p.ascii,
        );
        let hint = Line::from(vec![
            Span::raw(" "),
            Span::styled(key, theme::role(Role::Accent, p)),
            Span::styled(" open ", theme::role(Role::Muted, p)),
            Span::styled(
                theme::border_set(p.ascii).horizontal_bottom,
                theme::role(Role::Muted, p),
            ),
        ]);
        block = block.title_bottom(hint.right_aligned());
    }
    frame.render_widget(block, layout.alerts);
    let inner = layout.alerts_inner;
    let lines = lines_of(shown, inner.width, inner.height, p);
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
#[path = "alerts_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "alerts_box_tests.rs"]
mod box_tests;
#[cfg(test)]
#[path = "alerts_fixture.rs"]
pub(crate) mod fixture;
#[cfg(test)]
#[path = "alerts_frame_tests.rs"]
mod frame_tests;
