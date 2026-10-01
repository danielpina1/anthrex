//! Milestone 9.0.6 decision 15: the action menu's input forms, drawn on the kit's
//! dialog grammar. Every string an agent, git, the daemon or the user wrote (the
//! question, the brief, a model name, what was typed) passes `safe_text` before it is
//! drawn. Pure: rendering reads the form.

use crate::app::actions::forms::{ActionForm, Brief, kind_word};
use crate::safe_text::{multi_line, one_line};
use crate::text_area::TextArea;
use crate::theme::{Palette, Role, role};
use crate::ui::kit::{self, Hint};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

/// Text-area rows of the answer and message forms; the override's reason has two.
pub const AREA_ROWS: u16 = 4;
const REASON_ROWS: u16 = 2;
/// The most rows the question takes.
const QUESTION_ROWS: usize = 5;
/// The brief's preview lines (decision 15).
const BRIEF_LINES: usize = 3;

fn hint(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    }
}

fn ellipsis(p: Palette) -> &'static str {
    if p.ascii { "..." } else { "…" }
}

fn dot(p: Palette) -> &'static str {
    if p.ascii { " - " } else { " · " }
}

/// The form's title: its action's label.
pub fn title(form: &ActionForm) -> String {
    form.info().label.clone()
}

/// `label` muted and padded to `label_w`, then `spans`.
fn row(label: &str, label_w: usize, mut spans: Vec<Span<'static>>, p: Palette) -> Line<'static> {
    let pad = " ".repeat(label_w.saturating_sub(label.chars().count()));
    let mut out = vec![Span::styled(format!("{label}{pad}"), role(Role::Muted, p))];
    out.append(&mut spans);
    Line::from(out)
}

fn indent(label_w: usize, spans: Vec<Span<'static>>) -> Line<'static> {
    let mut out = vec![Span::raw(" ".repeat(label_w))];
    out.extend(spans);
    Line::from(out)
}

/// `label  <lines>`: sanitised, each logical line wrapped at `value_w`, at most `max`
/// rows with a closing `…` when the text goes on.
fn block(
    label: &str,
    label_w: usize,
    text: &str,
    value_w: usize,
    max: usize,
    p: Palette,
) -> Vec<Line<'static>> {
    let mut rows: Vec<String> = Vec::new();
    for line in multi_line(text).lines() {
        let line = one_line(line);
        if line.trim().is_empty() && rows.is_empty() {
            continue;
        }
        rows.extend(kit::wrap_words(&line, value_w.max(1)));
    }
    let more = rows.len() > max;
    rows.truncate(max);
    if more && let Some(last) = rows.last_mut() {
        *last = kit::cut(&format!("{last} {}", ellipsis(p)), value_w, ellipsis(p));
    }
    let mut out = Vec::new();
    for (i, text) in rows.into_iter().enumerate() {
        let spans = vec![Span::raw(text)];
        out.push(if i == 0 {
            row(label, label_w, spans, p)
        } else {
            indent(label_w, spans)
        });
    }
    out
}

/// The text area as `rows + 2` lines (so the dialog does not jump when a scroll mark
/// appears), the first led by `label`, the rest indented.
fn area_rows(
    label: &str,
    label_w: usize,
    area: &TextArea,
    rows: u16,
    value_w: usize,
    p: Palette,
) -> Vec<Line<'static>> {
    let mut lines = kit::text_area(area, rows, value_w as u16, p);
    lines.resize(usize::from(rows) + 2, Line::default());
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                row(label, label_w, line.spans, p)
            } else {
                indent(label_w, line.spans)
            }
        })
        .collect()
}

fn choice_row(label: &str, label_w: usize, value: &str, p: Palette) -> Line<'static> {
    let style = role(Role::Accent, p).add_modifier(Modifier::BOLD);
    row(
        label,
        label_w,
        vec![Span::styled(kit::choice_in(value, p), style)],
        p,
    )
}

fn plural(n: u32) -> String {
    format!("{n} {}", if n == 1 { "commit" } else { "commits" })
}

/// The form's rows, then a blank row and its hints, for a dialog `width` columns wide.
pub fn body(form: &ActionForm, width: u16, p: Palette) -> Vec<Line<'static>> {
    let width = width.min(kit::WRAP);
    let mut body = Vec::new();
    let mut keys = vec![hint("⏎", "continue", 9)];
    match form {
        ActionForm::Answer(f) => {
            let lw = 8;
            let vw = usize::from(width).saturating_sub(lw);
            body.push(row(
                "task",
                lw,
                vec![Span::raw(kit::cut(&one_line(&f.task), vw, ellipsis(p)))],
                p,
            ));
            if !f.question.trim().is_empty() {
                body.extend(block("asked", lw, &f.question, vw, QUESTION_ROWS, p));
            }
            match &f.brief {
                Brief::Loading => {
                    body.push(row(
                        "brief",
                        lw,
                        vec![Span::styled(
                            format!("loading{}", ellipsis(p)),
                            role(Role::Muted, p),
                        )],
                        p,
                    ));
                }
                Brief::Failed(why) => {
                    body.extend(block("brief", lw, why, vw, 1, p).into_iter().map(|mut l| {
                        l.spans
                            .iter_mut()
                            .skip(1)
                            .for_each(|s| s.style = role(Role::Failed, p));
                        l
                    }));
                }
                Brief::Ready(lines) => {
                    for (i, line) in lines.iter().take(BRIEF_LINES).enumerate() {
                        let text = Span::raw(kit::cut(&one_line(line), vw, ellipsis(p)));
                        body.push(if i == 0 {
                            row("brief", lw, vec![text], p)
                        } else {
                            indent(lw, vec![text])
                        });
                    }
                }
            }
            body.extend(area_rows("answer", lw, &f.text, AREA_ROWS, vw, p));
            keys.push(hint("^J", "newline", 5));
        }
        ActionForm::Message(f) => {
            let lw = 8;
            let vw = usize::from(width).saturating_sub(lw);
            body.push(row(
                "to",
                lw,
                vec![Span::raw(kit::cut(&one_line(&f.to), vw, ellipsis(p)))],
                p,
            ));
            body.push(choice_row("kind", lw, kind_word(f.kind), p));
            body.extend(area_rows("message", lw, &f.text, AREA_ROWS, vw, p));
            keys.push(hint("tab", "kind", 6));
            keys.push(hint("^J", "newline", 5));
        }
        ActionForm::Override(f) => {
            let lw = 8;
            let vw = usize::from(width).saturating_sub(lw);
            body.extend(area_rows("reason", lw, &f.reason, REASON_ROWS, vw, p));
            if f.blank {
                body.push(indent(
                    lw,
                    vec![Span::styled("type a reason first", role(Role::Failed, p))],
                ));
            }
        }
        ActionForm::Resume(f) => {
            let lw = 12;
            body.push(choice_row(
                "rebaseline",
                lw,
                if f.rebaseline { "on" } else { "off" },
                p,
            ));
            if let Some(moved) = &f.moved {
                let seven = |s: &str| one_line(&s.chars().take(7).collect::<String>());
                let arrow = if p.ascii { "->" } else { "→" };
                let text = format!(
                    "base moved {} {arrow} {} ({}); rebaselining re-runs tier 3 against the new base",
                    seven(&moved.from),
                    seven(&moved.to),
                    plural(moved.total)
                );
                for line in kit::wrap_words(&text, usize::from(width)) {
                    body.push(Line::styled(line, role(Role::Muted, p)));
                }
            }
            keys.push(hint("tab", "toggle", 6));
        }
        ActionForm::Promote(f) => {
            let lw = 14;
            let shown = f
                .options
                .get(f.at)
                .map(|o| o.label.as_str())
                .unwrap_or("configured");
            body.push(choice_row("orchestrator", lw, shown, p));
            keys.push(hint("tab", "next", 6));
        }
    }
    keys.push(hint("esc", "back", 1));
    body.push(Line::raw(""));
    body.push(kit::hints_joined(width, &keys, dot(p), p));
    body
}

#[cfg(test)]
#[path = "action_forms_tests.rs"]
mod tests;
