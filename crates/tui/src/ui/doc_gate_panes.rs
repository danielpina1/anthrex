//! The document gate screen's rows (`ui/doc_gate.rs`): the Review panel, its one-row
//! narrow form, and each pane's body (the document, `d`'s diff, `f`'s findings, `g`'s
//! drafts, side by side when wide). Every text passes `safe_text` through
//! `ui/doc_text.rs` or `one_line`, and every row is wrapped or cut to its width. Pure.

use super::{bold, gate_of};
use crate::app::App;
use crate::app::doc_gate::{DocGateScreen, DocLoad, DocPane, DraftLoad};
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, dot, ellipsis, fold, role};
use crate::ui::doc_text::{diff_lines, doc_lines, plain_lines};
use crate::ui::kit::cut;
use proto::{DocGateInfo, DocSeverity, DocView};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// The narrow panel row (exact): `<n> changes · <m> disputed findings (f)`.
pub(crate) fn panel_row(gate: Option<&DocGateInfo>, p: Palette) -> String {
    let (n, m) = gate.map_or((0, 0), |g| (g.changes_summary.len(), g.disputed.len()));
    format!("{n} changes {} {m} disputed findings (f)", dot(p))
}

/// The Review panel's rows at `width`: the changes from the previous version, the
/// disputed findings with the orchestrator's answers, at the brainstorm gate the
/// report's counts and approaches, and whether the version was reviewed.
pub(crate) fn panel_lines(app: &App, s: &DocGateScreen, width: u16) -> Vec<Line<'static>> {
    let p = app.palette();
    let muted = role(Role::Muted, p);
    let mut out = vec![Line::styled("Review", bold())];
    let Some(gate) = gate_of(app, s) else {
        // A closed gate says so on the message line (ruling T17-2); nothing loads here.
        if app.doc_gate_closed().is_none() {
            out.push(Line::styled(fold("loading…", p.ascii), muted));
        }
        return out;
    };
    let indent = |text: &str, style: Style, depth: u16| -> Vec<Line<'static>> {
        let pad = " ".repeat(usize::from(depth));
        plain_lines(text, width.saturating_sub(depth), style)
            .into_iter()
            .map(|l| {
                let mut spans = vec![Span::raw(pad.clone())];
                spans.extend(l.spans);
                Line::from(spans)
            })
            .collect()
    };
    out.push(Line::default());
    out.push(Line::styled("changes", bold()));
    if gate.changes_summary.is_empty() {
        let none = if s.version == 1 {
            "first version"
        } else {
            "none"
        };
        out.extend(indent(none, muted, 2));
    }
    for change in &gate.changes_summary {
        out.extend(indent(change, Style::default(), 2));
    }
    out.push(Line::default());
    out.push(Line::styled("disputed findings", bold()));
    if gate.disputed.is_empty() {
        out.extend(indent("none", muted, 2));
    }
    let answers = s
        .doc
        .ready()
        .map(|d| d.findings.as_slice())
        .unwrap_or_default();
    for f in &gate.disputed {
        let (word, style) = severity(f.severity, p);
        let mut line = vec![
            Span::raw("  "),
            Span::styled(one_line(&f.id), bold()),
            Span::raw(" "),
            Span::styled(word, style),
        ];
        if !f.place.is_empty() {
            line.push(Span::raw(format!(" at {}", one_line(&f.place))));
        }
        out.push(cut_line(Line::from(line), width, p));
        out.extend(indent(&f.text, Style::default(), 4));
        let answer = (answers.iter())
            .find(|(a, _)| a.id == f.id)
            .and_then(|(_, a)| a.clone());
        match answer {
            Some(a) => out.extend(indent(&format!("answer: {a}"), muted, 4)),
            None => out.extend(indent("answer: kept", muted, 4)),
        }
    }
    if let Some(report) = &gate.report {
        out.push(Line::default());
        out.push(Line::styled("report", bold()));
        let counts = format!(
            "{} agree {} {} disagree",
            report.agree,
            dot(p),
            report.disagree
        );
        out.extend(indent(&counts, Style::default(), 2));
        for a in &report.approaches {
            let text = format!("{} [{}]", one_line(&a.name), one_line(&a.tag));
            out.extend(indent(&text, Style::default(), 2));
        }
    }
    if let Some(why) = &gate.not_reviewed {
        out.push(Line::default());
        out.extend(indent(
            &format!("not reviewed: {why}"),
            role(Role::Attention, p),
            0,
        ));
    }
    if gate.same_runtime {
        out.push(Line::default());
        let text = "reviewed on the orchestrator's own runtime: no peer runtime was installed";
        out.extend(indent(text, muted, 0));
    }
    out
}

fn severity(s: DocSeverity, p: Palette) -> (&'static str, Style) {
    match s {
        DocSeverity::Blocking => ("blocking", role(Role::Failed, p)),
        DocSeverity::Minor => ("minor", role(Role::Muted, p)),
    }
}

/// `line` cut to `width` columns, each span sanitised (spans are already one line).
fn cut_line(line: Line<'static>, width: u16, p: Palette) -> Line<'static> {
    let mut left = usize::from(width);
    let mut spans = Vec::new();
    for span in line.spans {
        if left == 0 {
            break;
        }
        let text = one_line(&span.content);
        let text = cut(&text, left, ellipsis(p));
        left = left.saturating_sub(text.width());
        spans.push(Span::styled(text, span.style));
    }
    Line::from(spans)
}

/// A load's rows: `loading…`, the failure in `Failed`, or `ready`'s.
fn load_lines(
    load: Option<&DocLoad>,
    width: u16,
    p: Palette,
    ready: impl FnOnce(&DocView) -> Vec<Line<'static>>,
) -> Vec<Line<'static>> {
    match load {
        None | Some(DocLoad::Loading(_)) => {
            vec![Line::styled(
                fold("loading…", p.ascii),
                role(Role::Muted, p),
            )]
        }
        Some(DocLoad::Failed(text)) => plain_lines(text, width, role(Role::Failed, p)),
        Some(DocLoad::Ready(doc)) => ready(doc),
    }
}

/// One draft's column: its label bold, then its text, or `(no draft: …)` with the
/// daemon's text when it could not be shown.
fn draft_lines(d: &DraftLoad, width: u16, p: Palette) -> Vec<Line<'static>> {
    let title = format!("{} {} draft v{}", one_line(&d.label), dot(p), d.version);
    let mut out = vec![Line::styled(
        cut(&title, usize::from(width), ellipsis(p)),
        bold(),
    )];
    out.extend(match &d.load {
        DocLoad::Failed(text) => {
            plain_lines(&format!("(no draft: {text})"), width, role(Role::Muted, p))
        }
        load => load_lines(Some(load), width, p, |doc| doc_lines(&doc.text, width, p)),
    });
    out
}

/// Two columns side by side, `lw` and `rw` wide, a muted `│` between.
fn side_by_side(
    left: Vec<Line<'static>>,
    right: Vec<Line<'static>>,
    lw: u16,
    p: Palette,
) -> Vec<Line<'static>> {
    let rule = if p.ascii { "|" } else { "│" };
    let rows = left.len().max(right.len());
    let mut left = left.into_iter();
    let mut right = right.into_iter();
    (0..rows)
        .map(|_| {
            let l = left.next().unwrap_or_default();
            let pad = usize::from(lw).saturating_sub(l.width());
            let mut spans = l.spans;
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(format!(" {rule} "), role(Role::Muted, p)));
            spans.extend(right.next().unwrap_or_default().spans);
            Line::from(spans)
        })
        .collect()
}

/// The body's rows for the screen's pane at `width`; `wide` sets the drafts side by
/// side.
pub(crate) fn pane_lines(
    app: &App,
    s: &DocGateScreen,
    width: u16,
    wide: bool,
) -> Vec<Line<'static>> {
    let p = app.palette();
    let muted = role(Role::Muted, p);
    match s.pane {
        DocPane::Document => {
            load_lines(Some(&s.doc), width, p, |doc| doc_lines(&doc.text, width, p))
        }
        DocPane::Diff => {
            let title = match s.version {
                1 => "diff".to_owned(),
                n => format!("diff against v{}", n - 1),
            };
            let mut out = vec![Line::styled(title, bold())];
            out.extend(load_lines(s.diff.as_ref(), width, p, |doc| {
                match &doc.diff {
                    Some(diff) => diff_lines(diff, width, p),
                    None => plain_lines(
                        &format!(
                            "v{} is the first version: there is no earlier one",
                            s.version
                        ),
                        width,
                        muted,
                    ),
                }
            }));
            out
        }
        DocPane::Findings => {
            let mut out = vec![Line::styled(format!("findings of v{}", s.version), bold())];
            out.extend(load_lines(Some(&s.doc), width, p, |doc| {
                findings_lines(doc, width, p)
            }));
            out
        }
        DocPane::Drafts => {
            let drafts = s.drafts.as_deref().unwrap_or_default();
            if drafts.is_empty() {
                let text = "no brainstorm draft is stored for this version";
                return plain_lines(text, width, muted);
            }
            if wide && drafts.len() == 2 {
                let lw = width.saturating_sub(3) / 2;
                let rw = width.saturating_sub(3).saturating_sub(lw);
                let left = draft_lines(&drafts[0], lw, p);
                let right = draft_lines(&drafts[1], rw, p);
                return side_by_side(left, right, lw, p);
            }
            let mut out = Vec::new();
            for (i, d) in drafts.iter().enumerate() {
                if i > 0 {
                    out.push(Line::default());
                }
                out.extend(draft_lines(d, width, p));
            }
            out
        }
    }
}

/// `f`: every finding with its severity, place, text and the orchestrator's answer.
fn findings_lines(doc: &DocView, width: u16, p: Palette) -> Vec<Line<'static>> {
    let muted = role(Role::Muted, p);
    if doc.findings.is_empty() {
        return plain_lines("no findings", width, muted);
    }
    let mut out = Vec::new();
    for (f, answer) in &doc.findings {
        let (word, style) = severity(f.severity, p);
        let mut line = vec![
            Span::styled(one_line(&f.id), bold()),
            Span::raw(" "),
            Span::styled(word, style),
        ];
        if !f.place.is_empty() {
            line.push(Span::raw(format!(" at {}", one_line(&f.place))));
        }
        out.push(cut_line(Line::from(line), width, p));
        let inner = width.saturating_sub(2);
        let pad = |lines: Vec<Line<'static>>| {
            lines.into_iter().map(|l| {
                let mut spans = vec![Span::raw("  ")];
                spans.extend(l.spans);
                Line::from(spans)
            })
        };
        out.extend(pad(plain_lines(&f.text, inner, Style::default())));
        let answer = match answer {
            Some(a) => format!("answer: {a}"),
            None => "no answer".to_owned(),
        };
        out.extend(pad(plain_lines(&answer, inner, muted)));
        out.push(Line::default());
    }
    out
}
