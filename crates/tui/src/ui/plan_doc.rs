//! Milestone 9.6 decisions 34 to 36 (DF §6.1): the plan review's Plan doc tab, drawn in
//! the review's own frame (`ui/plan_review.rs`, which hands it the body). The first row
//! is the gate screen's header, `Plan · run <id4> · v<n> of <m> · <reason>`, then the
//! review's `├─…─┤` rule, then `plan.md` as the gate screen draws a document
//! (`ui/doc_text.rs`: headings bold, a leading `R<n>` in the accent, fenced code dim),
//! scrolled by the tab's own scroll. Its coverage table is drawn as aligned columns,
//! `requirement  tasks`, and a requirement no task covers reads `⚠ not covered` in
//! `Attention` (the plan review's warning, milestone 9.0.7 decision 23). A submitted plan
//! covers every requirement, so the mark shows only a plan the engine has not checked.
//! While the document loads the body says `loading…`; a refusal is drawn in `Failed`.
//! Every text is cleaned (`safe_text`) and cut or wrapped to its area. Pure: `&App` in.

use super::{BAR, Placed, row};
use crate::app::App;
use crate::app::doc_gate::DocLoad;
use crate::app::region::KeyRegion;
use crate::safe_text::{multi_line, one_line};
use crate::theme::{Glyph, Palette, Role, ellipsis, glyph, role};
use crate::ui::doc_text::{MAX_LINES, doc_lines, plain_lines};
use crate::ui::kit::{self, wrap_words};
use proto::{DocGateInfo, DocGateKind, DocSeverity};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// The table's heading in `plan.md` (`daemon::run::design::plan_md`).
const COVERAGE: &str = "## Coverage";
/// The first column's label.
const REQUIREMENT: &str = "requirement";
/// Columns between the table's two columns.
const GAP: usize = 2;

/// Whether the review shows its Plan doc tab.
pub(crate) fn shown(app: &App) -> bool {
    app.plan_doc().is_some() && app.plan_doc_gate().is_some()
}

/// A table row's two cells: `| a | b |`; `None` for any other line.
fn cells(line: &str) -> Option<(String, String)> {
    let inner = line.trim().strip_prefix('|')?.strip_suffix('|')?;
    let mut parts = inner.split('|').map(str::trim);
    let (a, b) = (parts.next()?, parts.next()?);
    parts.next().is_none().then(|| (a.to_owned(), b.to_owned()))
}

/// The table's `|---|---|` row.
fn separator((a, b): &(String, String)) -> bool {
    let rule = |c: &str| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':'));
    rule(a) && rule(b)
}

/// `R<n>`: drawn in the accent.
fn is_requirement(id: &str) -> bool {
    id.strip_prefix('R')
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// One coverage row (or the table's head) at `width` columns: the id padded to `col`,
/// then the tasks wrapped under themselves, or `⚠ not covered`.
fn table_rows(
    (id, tasks): &(String, String),
    head: bool,
    (col, width): (usize, usize),
    p: Palette,
) -> Vec<Line<'static>> {
    let muted = role(Role::Muted, p);
    if head {
        let text = format!("{REQUIREMENT:<col$}{}tasks", " ".repeat(GAP));
        return vec![Line::styled(kit::cut(&text, width, ellipsis(p)), muted)];
    }
    let id_style = match is_requirement(id) {
        true => role(Role::Accent, p),
        false => Style::default(),
    };
    let id = kit::cut(id, col, ellipsis(p));
    let pad = " ".repeat(col.saturating_sub(id.width()) + GAP);
    let room = width.saturating_sub(col + GAP).max(1);
    if tasks == "none" || tasks.is_empty() {
        let mark = format!("{} not covered", glyph(Glyph::Warning, p.ascii));
        let line = Line::from(vec![
            Span::styled(id, id_style),
            Span::raw(pad),
            Span::styled(mark, role(Role::Attention, p)),
        ]);
        return vec![super::clip(line, width)];
    }
    let indent = " ".repeat(col + GAP);
    wrap_words(tasks, room)
        .into_iter()
        .enumerate()
        .map(|(i, part)| match i {
            0 => Line::from(vec![
                Span::styled(id.clone(), id_style),
                Span::raw(pad.clone()),
                Span::raw(part),
            ]),
            _ => Line::from(vec![Span::raw(indent.clone()), Span::raw(part)]),
        })
        .map(|line| super::clip(line, width))
        .collect()
}

/// Source lines between table rows, as the gate screen draws a document; a trailing
/// blank line (which `str::lines` drops from the joined text) is kept.
fn chunk_lines(chunk: &[&str], width: u16, p: Palette) -> Vec<Line<'static>> {
    let mut out = doc_lines(&chunk.join("\n"), width, p);
    if chunk.last().is_some_and(|l| l.is_empty()) {
        out.push(Line::default());
    }
    out
}

/// `plan.md`'s rows at `width` columns: the document as the gate screen draws one, its
/// coverage table as aligned columns with uncovered requirements marked.
pub(crate) fn lines(text: &str, width: u16, p: Palette) -> Vec<Line<'static>> {
    let width_n = usize::from(width).max(1);
    let cleaned = multi_line(text);
    let all: Vec<String> = cleaned.lines().map(one_line).collect();
    let cut = all.len().saturating_sub(MAX_LINES);
    let source = &all[..all.len().min(MAX_LINES)];
    // Final fix wave FW-78 (WB-D m4): the engine's table is the last `## Coverage`; an
    // earlier one is a brief's own text.
    let start = source.iter().rposition(|l| l.trim_end() == COVERAGE);
    let table: Vec<(String, String)> = start
        .map(|at| source[at..].iter().filter_map(|l| cells(l)).collect())
        .unwrap_or_default();
    let col = (table.iter())
        .filter(|c| !separator(c))
        .skip(1)
        .map(|(id, _)| id.width())
        .chain([REQUIREMENT.len()])
        .max()
        .unwrap_or(0)
        .min(width_n / 3);
    let mut out = Vec::new();
    let mut chunk: Vec<&str> = Vec::new();
    let mut head = true;
    for (i, line) in source.iter().enumerate() {
        let parsed = start.filter(|at| i > *at).and_then(|_| cells(line));
        match parsed {
            Some(cells) => {
                out.extend(chunk_lines(&chunk, width, p));
                chunk.clear();
                if !separator(&cells) {
                    out.extend(table_rows(&cells, head, (col, width_n), p));
                    head = false;
                }
            }
            None => chunk.push(line),
        }
    }
    out.extend(chunk_lines(&chunk, width, p));
    if cut > 0 {
        out.push(Line::styled(
            format!("[cut: {cut} more lines]"),
            role(Role::Muted, p),
        ));
    }
    out
}

/// Decision 16 (the final fix wave's FW-19 check): the gate version's review lines,
/// above the document: why it went unreviewed (`Muted`: information, never
/// `Attention`, milestone 9.0.7 principle 1), then its disputed findings, each
/// `<id> <severity> at <place>` and its text; nothing when it has neither. A user's or the engine's later version carries the review's lines.
fn review_lines(gate: &DocGateInfo, width: u16, p: Palette) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if let Some(why) = &gate.not_reviewed {
        let text = format!("not reviewed: {why}");
        out.extend(plain_lines(&text, width, role(Role::Muted, p)));
    }
    if !gate.disputed.is_empty() {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        out.push(Line::styled("disputed findings", bold));
        for f in &gate.disputed {
            let severity = match f.severity {
                DocSeverity::Blocking => "blocking",
                DocSeverity::Minor => "minor",
            };
            let mut head = format!("  {} {severity}", one_line(&f.id));
            if !f.place.is_empty() {
                head.push_str(&format!(" at {}", one_line(&f.place)));
            }
            out.push(Line::raw(kit::cut(&head, usize::from(width), ellipsis(p))));
            let text = plain_lines(&f.text, width.saturating_sub(4), Style::default());
            out.extend(text.into_iter().map(|l| {
                let mut spans = vec![Span::raw("    ")];
                spans.extend(l.spans);
                Line::from(spans)
            }));
        }
    }
    if !out.is_empty() {
        out.push(Line::default());
    }
    out
}

/// The tab's body rows at `width` columns: the gate's review lines, then the document,
/// `loading…`, or why it is not.
fn body_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let p = app.palette();
    let mut out = (app.plan_doc_gate())
        .map(|(_, gate)| review_lines(gate, width, p))
        .unwrap_or_default();
    out.extend(document_lines(app, width, p));
    out
}

/// The document's rows at `width` columns, `loading…`, or why it is not.
fn document_lines(app: &App, width: u16, p: Palette) -> Vec<Line<'static>> {
    match app.plan_doc().map(|d| &d.load) {
        Some(DocLoad::Ready(doc)) => lines(&doc.text, width, p),
        Some(DocLoad::Failed(why)) => plain_lines(why, width, role(Role::Failed, p)),
        Some(DocLoad::Loading(_)) | None => vec![Line::styled(
            format!("loading{}", ellipsis(p)),
            role(Role::Muted, p),
        )],
    }
}

/// The frame's interior at `body` and the document's area in it (below the header and
/// the rule), indented by the review's bar column.
fn areas(body: Rect) -> (Rect, Rect) {
    let inner = Rect {
        x: body.x.saturating_add(1),
        y: body.y.saturating_add(1),
        width: body.width.saturating_sub(2),
        height: body.height.saturating_sub(2),
    };
    let doc = Rect {
        x: inner.x.saturating_add(BAR),
        y: inner.y.saturating_add(2),
        width: inner.width.saturating_sub(BAR),
        height: inner.height.saturating_sub(2),
    };
    (inner, doc)
}

/// The last first row of the document at `body`: its rows less its area's height.
pub(crate) fn max_scroll(app: &App, body: Rect) -> usize {
    let (_, doc) = areas(body);
    body_lines(app, doc.width)
        .len()
        .saturating_sub(usize::from(doc.height))
}

/// The tab inside the frame at `body`, row by row (the rule on the frame's sides).
pub(crate) fn placed(app: &App, body: Rect) -> Vec<Placed> {
    let mut out = Vec::new();
    let (Some(doc), Some((run_id, gate))) = (app.plan_doc(), app.plan_doc_gate()) else {
        return out;
    };
    let p = app.palette();
    let (inner, area) = areas(body);
    let text = crate::ui::doc_gate::header_of(app, (run_id, DocGateKind::Plan, gate.version), p);
    let header = Rect {
        x: inner.x.saturating_add(BAR),
        width: inner.width.saturating_sub(BAR),
        ..inner
    };
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let header_line = Line::styled(
        kit::cut(&text, usize::from(header.width), ellipsis(p)),
        bold,
    );
    row(&mut out, header, inner.y, header_line);
    if body.width >= 2 && inner.height >= 2 {
        let keys_here = app.key_region() == KeyRegion::Review;
        let border = role(if keys_here { Role::Accent } else { Role::Muted }, p);
        let (left, line, right) = match p.ascii {
            true => ("|", "-", "|"),
            false => ("├", "─", "┤"),
        };
        let rule = format!("{left}{}{right}", line.repeat(usize::from(body.width) - 2));
        row(&mut out, body, inner.y + 1, Line::styled(rule, border));
    }
    let lines = body_lines(app, area.width);
    let height = usize::from(area.height);
    let scroll = doc.scroll.min(lines.len().saturating_sub(height));
    for (n, line) in lines.into_iter().skip(scroll).take(height).enumerate() {
        let y = area.y.saturating_add(u16::try_from(n).unwrap_or(u16::MAX));
        row(&mut out, area, y, line);
    }
    out
}

#[cfg(test)]
#[path = "plan_doc_tests.rs"]
mod tests;
