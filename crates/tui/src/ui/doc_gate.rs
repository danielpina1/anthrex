//! Milestone 9.6 decisions 34 to 36 (DF §6.1): the document gate screen, drawn over the
//! body as the other full-body screens are (milestone 9.0.6 decision 33), its frame the
//! one accented while it has the keys (9.0.7 decision 1). The title is the header,
//! `<Kind> · run <id4> · v<n> of <m> · <reason>`. While the orchestrator revises, the
//! first row says `revising v<n+1>… (your note: "<note head 60>")` (a read-back's note
//! is anthrex's, not called the user's). At 100 columns or more the document is on the
//! left and the Review panel on the right; narrower, the document takes the width and
//! the panel is one row, `<n> changes · <m> disputed findings (f)`. `d`, `f` and `g`
//! show the diff, the findings and the drafts (side by side when wide, stacked
//! narrower) in the document's place. The last rows are the message line. Every text
//! passes `safe_text` (`ui/doc_text.rs`), every row is cut or wrapped to its area, so a
//! hostile document stays inside it at every size. Pure: `&App` in.

use crate::actions_request::short_id;
use crate::app::doc_gate::{DocGateScreen, DocPane, Tone, gate_doc, kind_title};
use crate::app::{App, region::KeyRegion};
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, dot, ellipsis, role};
use crate::ui::doc_text::plain_lines;
use crate::ui::kit::{self, Hint, cut};
use proto::{DocGateInfo, DocGateKind, RunInfo};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

/// DF §6.1: from this many columns the Review panel sits beside the document, and the
/// drafts side by side.
pub const WIDE: u16 = 100;
/// The head of the user's note the revising row shows.
pub const NOTE_HEAD: usize = 60;
#[path = "doc_gate_panes.rs"]
mod panes;
pub(crate) use panes::{pane_lines, panel_lines, panel_row};

/// The message line wraps to at most this many rows.
const MESSAGE_ROWS: usize = 3;

fn hint(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    }
}

fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn run_of<'a>(app: &'a App, s: &DocGateScreen) -> Option<&'a RunInfo> {
    app.runs.runs.iter().find(|r| r.run_id == s.run_id)
}

/// The run's gate while it is the screen's own version, for the panel.
fn gate_of<'a>(app: &'a App, s: &DocGateScreen) -> Option<&'a DocGateInfo> {
    let gate = run_of(app, s)?.doc_gate.as_ref()?;
    (gate.kind == s.kind && gate.version == s.version).then_some(gate)
}

/// The status bar's hints while the screen has the keys: the gate's keys (rethink at
/// the brainstorm gate, back at the spec gate, `g` at the brainstorm gate), only `x`
/// while the orchestrator revises, scrolling and `esc` in a view.
pub(crate) fn hints(app: &App, s: &DocGateScreen) -> Vec<Hint> {
    let esc = hint("esc", "close", u8::MAX);
    // Ruling T17-2: a closed gate leaves only `q` and Esc; in a view, Esc first goes
    // back to the document (final fix wave FW-56).
    if app.doc_gate_closed().is_some() {
        return match s.pane {
            DocPane::Document => vec![esc],
            _ => vec![hint("esc", "back", u8::MAX)],
        };
    }
    if s.pane != DocPane::Document {
        return vec![hint("j/k", "scroll", 6), hint("esc", "back", u8::MAX)];
    }
    if app.doc_revising().is_some() {
        return vec![hint("x", "reject", 9), hint("j/k", "scroll", 6), esc];
    }
    let brainstorm = s.kind == DocGateKind::Brainstorm;
    let mut out = vec![
        hint("a", "approve", 9),
        hint("c", "changes", 8),
        hint("e", "edit", 6),
    ];
    if brainstorm {
        out.push(hint("r", "rethink", 5));
    } else {
        out.push(hint("b", "back", 5));
    }
    out.push(hint("x", "reject", 9));
    out.push(hint("d", "diff", 4));
    out.push(hint("f", "findings", 4));
    if brainstorm {
        out.push(hint("g", "drafts", 3));
    }
    out.push(hint("j/k", "scroll", 2));
    out.push(esc);
    out
}

/// The header (exact): `<Kind> · run <id4> · v<n> of <m> · <reason>`, `<m>` the gate
/// versions of its document, the reason left out while empty.
pub(crate) fn header(app: &App, s: &DocGateScreen, p: Palette) -> String {
    header_of(app, (&s.run_id, s.kind, s.version), p)
}

/// [`header`] for `run_id`'s `kind` gate at `version` (milestone 9.6 task 18: the plan
/// review's Plan doc tab has the same header).
pub(crate) fn header_of(
    app: &App,
    (run_id, kind, version): (&str, DocGateKind, u32),
    p: Palette,
) -> String {
    let d = dot(p);
    let id = short_id(&one_line(run_id)).to_owned();
    let docs = (app.runs.runs.iter())
        .find(|r| r.run_id == run_id)
        .map(|r| r.docs.as_slice())
        .unwrap_or_default();
    let doc = gate_doc(kind);
    let of = docs
        .iter()
        .filter(|i| i.kind == doc)
        .count()
        .max(version as usize);
    let reason = (docs.iter())
        .find(|i| i.kind == doc && i.version == version)
        .map(|i| one_line(&i.reason))
        .filter(|r| !r.is_empty());
    let mut text = format!("{} {d} run {id} {d} v{version} of {of}", kind_title(kind));
    if let Some(reason) = reason {
        text.push_str(&format!(" {d} {reason}"));
    }
    text
}

/// The revising row (exact): `revising v<n+1>… (your note: "<note head 60>")`, the
/// note anthrex's own (`read_back`) without `your note:`.
pub(crate) fn revising_text(gate: &DocGateInfo, p: Palette) -> Option<String> {
    let note = gate.revising.as_deref()?;
    let head: String = one_line(note).chars().take(NOTE_HEAD).collect();
    let e = ellipsis(p);
    let next = gate.version + 1;
    Some(match gate.revising_cause.is_users() {
        true => format!("revising v{next}{e} (your note: \"{head}\")"),
        false => format!("revising v{next}{e} ({head})"),
    })
}

/// The message line's rows at `width`, at most [`MESSAGE_ROWS`]: a closed gate's
/// `the <kind> gate is closed` (ruling T17-2), else the screen's message.
fn message_lines(app: &App, s: &DocGateScreen, width: u16, p: Palette) -> Vec<Line<'static>> {
    let closed = app.doc_gate_closed().map(|text| (Tone::Note, text));
    let Some((tone, text)) = closed.as_ref().or(s.message.as_ref()) else {
        return Vec::new();
    };
    let style = match tone {
        Tone::Done => role(Role::Done, p),
        Tone::Refused => role(Role::Failed, p),
        Tone::Note => role(Role::Muted, p),
    };
    let mut lines = plain_lines(text, width, style);
    if lines.len() > MESSAGE_ROWS {
        lines.truncate(MESSAGE_ROWS);
        if let Some(last) = lines.last_mut() {
            let text: String = last.spans.iter().map(|s| s.content.as_ref()).collect();
            let room = usize::from(width).saturating_sub(ellipsis(p).width());
            *last = Line::styled(format!("{}{}", cut(&text, room, ""), ellipsis(p)), style);
        }
    }
    lines
}

/// Where the screen draws, inside its frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Areas {
    pub revising: Rect,
    pub body: Rect,
    /// The Review panel and the rule left of it (wide, the document pane only).
    pub panel: Option<(Rect, Rect)>,
    /// The narrow panel row.
    pub panel_row: Option<Rect>,
    pub message: Rect,
}

/// The interior of the screen's frame over `area`.
fn interior(area: Rect, p: Palette) -> Rect {
    kit::screen_frame("", true, p).inner(area)
}

pub(crate) fn areas(app: &App, s: &DocGateScreen, area: Rect) -> Areas {
    let p = app.palette();
    let inner = interior(area, p);
    let mut rest = inner;
    let take_top = |rest: &mut Rect, n: u16| {
        let n = n.min(rest.height);
        let top = Rect { height: n, ..*rest };
        rest.y += n;
        rest.height -= n;
        top
    };
    let take_bottom = |rest: &mut Rect, n: u16| {
        let n = n.min(rest.height);
        rest.height -= n;
        Rect {
            y: rest.y + rest.height,
            height: n,
            ..*rest
        }
    };
    let revising = take_top(&mut rest, u16::from(app.doc_revising().is_some()));
    let message_rows = message_lines(app, s, inner.width, p).len() as u16;
    let message = take_bottom(&mut rest, message_rows);
    let wide = area.width >= WIDE;
    let mut out = Areas {
        revising,
        message,
        ..Areas::default()
    };
    if s.pane == DocPane::Document && !wide {
        out.panel_row = Some(take_bottom(&mut rest, 1));
    }
    if s.pane == DocPane::Document && wide {
        let panel_w = (rest.width / 3)
            .clamp(30, 44)
            .min(rest.width.saturating_sub(2));
        let body_w = rest.width.saturating_sub(panel_w + 1);
        let body = Rect {
            width: body_w,
            ..rest
        };
        let rule = Rect {
            x: rest.x + body_w,
            width: 1.min(rest.width),
            ..rest
        };
        // One column of padding right of the rule.
        let panel = Rect {
            x: rule.x + rule.width + 1.min(panel_w),
            width: panel_w.saturating_sub(1),
            ..rest
        };
        out.body = body;
        out.panel = Some((rule, panel));
    } else {
        out.body = rest;
    }
    out
}

/// The largest useful `scroll` over `body`, by the renderer's own rows, so the keys
/// stop where the view stops.
pub(crate) fn max_scroll(app: &App, s: &DocGateScreen, body: Rect) -> usize {
    let a = areas(app, s, body);
    let lines = pane_lines(app, s, a.body.width, body.width >= WIDE);
    kit::last_top(lines.len(), usize::from(a.body.height))
}

pub fn render(frame: &mut Frame, app: &App, s: &DocGateScreen, area: Rect) {
    let p = app.palette();
    // The one accented border is the dialog's while one is open (milestone 9.0.6
    // decision 5).
    let keys_here = app.key_region() == KeyRegion::Screen;
    let block = kit::screen_frame(&header(app, s, p), keys_here, p);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let a = areas(app, s, area);
    if a.body.width == 0 && a.revising.width == 0 {
        return;
    }
    if let Some(text) = app.doc_revising().and_then(|g| revising_text(g, p)) {
        let line = Line::styled(
            cut(&text, usize::from(a.revising.width), ellipsis(p)),
            role(Role::Working, p),
        );
        frame.render_widget(Paragraph::new(line), a.revising);
    }
    let lines = pane_lines(app, s, a.body.width, area.width >= WIDE);
    let shown = kit::from_top(lines, s.scroll, usize::from(a.body.height), p);
    frame.render_widget(Paragraph::new(shown), a.body);
    if let Some((rule, panel)) = a.panel {
        let glyph = if p.ascii { "|" } else { "│" };
        let rows: Vec<Line> = (0..rule.height)
            .map(|_| Line::styled(glyph, role(Role::Muted, p)))
            .collect();
        frame.render_widget(Paragraph::new(rows), rule);
        let lines = panel_lines(app, s, panel.width);
        let shown = kit::from_top(lines, 0, usize::from(panel.height), p);
        frame.render_widget(Paragraph::new(shown), panel);
    }
    if let Some(row) = a.panel_row {
        let text = panel_row(gate_of(app, s), p);
        let line = Line::styled(
            cut(&text, usize::from(row.width), ellipsis(p)),
            role(Role::Muted, p),
        );
        frame.render_widget(Paragraph::new(line), row);
    }
    let message = message_lines(app, s, a.message.width, p);
    frame.render_widget(Paragraph::new(message), a.message);
}

#[cfg(test)]
#[path = "doc_gate_tests.rs"]
pub(crate) mod tests;
