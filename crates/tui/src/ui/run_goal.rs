//! Milestone 9 decision 44 and 9.0.6 decision 39: rendering for the goal form
//! (`crate::run_goal`, the pure model), built on the kit's dialog grammar (decision 5):
//! one accented frame titled `start a goal in <project>`, lower-case labels, `‹ value ›`
//! choices, the goal in a four-row text area, hints `⏎ start · tab next · esc cancel`.
//! Every glyph honours `Palette.ascii`. The project, the model names and what was typed
//! pass `safe_text`.

use crate::dialog::TextInput;
use crate::run_goal::{GoalField, GoalForm, GoalModel, field_label};
use crate::safe_text::one_line;
use crate::theme::{Glyph, Palette, Role, glyph, role};
use crate::ui::kit::{self, Hint};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

/// Rows of the goal's text area.
pub const GOAL_ROWS: u16 = 4;
/// The focus marker's columns, then the label's.
const MARK_W: usize = 2;
const LABEL_W: usize = 18;

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

/// `marker label` for `field`: the focused field's label is accented and bold and led
/// by the selection glyph (a cue that does not depend on colour).
fn label(form: &GoalForm, field: GoalField, p: Palette) -> Vec<Span<'static>> {
    let focused = form.focus == field && !form.submitting;
    let (mark, style) = if focused {
        (
            glyph(Glyph::Selection, p.ascii),
            role(Role::Accent, p).add_modifier(Modifier::BOLD),
        )
    } else {
        (" ", role(Role::Muted, p))
    };
    vec![
        Span::styled(format!("{mark:<MARK_W$}"), style),
        Span::styled(format!("{:<LABEL_W$}", field_label(field)), style),
    ]
}

fn indent() -> Span<'static> {
    Span::raw(" ".repeat(MARK_W + LABEL_W))
}

fn choice_line(form: &GoalForm, field: GoalField, value: &str, p: Palette) -> Line<'static> {
    let mut spans = label(form, field, p);
    let style = if form.focus == field && !form.submitting {
        role(Role::Accent, p).add_modifier(Modifier::BOLD)
    } else {
        ratatui::style::Style::default()
    };
    spans.push(Span::styled(kit::choice_in(value, p), style));
    Line::from(spans)
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// The custom model's text on one line, the cursor a reversed cell when `focused`.
fn input_line(input: &TextInput, width: usize, focused: bool) -> Line<'static> {
    let (visible, column) = input.visible(width as u16);
    let graphemes: Vec<&str> = visible.graphemes(true).collect();
    let at = usize::from(column).min(graphemes.len());
    let before = graphemes[..at].concat();
    let mut spans = vec![indent(), Span::raw(one_line(&before))];
    if focused {
        let under = graphemes.get(at).copied().unwrap_or(" ");
        let after = graphemes.get(at + 1..).map(|g| g.concat());
        spans.push(Span::styled(
            one_line(under),
            ratatui::style::Style::default().add_modifier(Modifier::REVERSED),
        ));
        spans.push(Span::raw(one_line(&after.unwrap_or_default())));
    } else {
        spans.push(Span::raw(one_line(&graphemes[at..].concat())));
    }
    Line::from(spans)
}

/// The form's title: `start a goal in <project>`, the project cut to fit `width`.
pub fn title(form: &GoalForm, width: u16, p: Palette) -> String {
    let project = one_line(&form.project.display().to_string());
    kit::cut(
        &format!("start a goal in {project}"),
        usize::from(width),
        ellipsis(p),
    )
}

/// The dialog's rows for `width` interior columns: the fields, the error, a blank row
/// and the hints.
pub fn body(form: &GoalForm, width: u16, p: Palette) -> Vec<Line<'static>> {
    let width = width.min(kit::WRAP);
    let value_w = usize::from(width).saturating_sub(MARK_W + LABEL_W);
    let mut body = Vec::new();

    // The text area takes `rows + 2` lines, so the dialog keeps its height when a
    // scroll mark appears.
    let mut area = kit::text_area(&form.goal, GOAL_ROWS, value_w as u16, p);
    area.resize(usize::from(GOAL_ROWS) + 2, Line::default());
    for (i, line) in area.into_iter().enumerate() {
        let mut spans = if i == 0 {
            label(form, GoalField::Goal, p)
        } else {
            vec![indent()]
        };
        spans.extend(line.spans);
        body.push(Line::from(spans));
    }

    let runtime = match form.runtime {
        Some(runtime) => runtime.label(),
        None => "configured",
    };
    body.push(choice_line(form, GoalField::Runtime, runtime, p));

    let options = form.model_options();
    let shown = options
        .get(form.model_at())
        .map_or("default", String::as_str);
    let shown = if p.ascii && shown == "custom…" {
        "custom..."
    } else {
        shown
    };
    body.push(choice_line(form, GoalField::Model, shown, p));
    if let GoalModel::Custom(input) = &form.model {
        let focused = form.focus == GoalField::Model && !form.submitting;
        body.push(input_line(input, value_w, focused));
    }

    body.push(choice_line(
        form,
        GoalField::Trust,
        on_off(form.trust_project),
        p,
    ));
    body.push(choice_line(form, GoalField::Yes, on_off(form.yes), p));
    body.push(choice_line(
        form,
        GoalField::UnconfinedChecks,
        on_off(form.unconfined_checks),
        p,
    ));

    if let Some(error) = &form.error {
        body.push(Line::styled(
            kit::cut(&one_line(error), usize::from(width), ellipsis(p)),
            role(Role::Failed, p),
        ));
    }
    body.push(Line::raw(""));
    if form.submitting {
        body.push(Line::styled(
            format!("starting{} triage can take minutes", ellipsis(p)),
            role(Role::Muted, p),
        ));
        body.push(kit::hints_joined(
            width,
            &[hint("esc", "close", 1)],
            dot(p),
            p,
        ));
    } else {
        let keys = [
            hint("⏎", "start", 9),
            hint("tab", "next", 6),
            hint("esc", "cancel", 1),
        ];
        body.push(kit::hints_joined(width, &keys, dot(p), p));
    }
    body
}

pub fn render(frame: &mut Frame, form: &GoalForm, area: Rect, p: Palette) {
    let width = area.width.min(kit::DIALOG_MAX).saturating_sub(4);
    let body = body(form, width, p);
    let rect = kit::dialog_area(area, body.len() as u16);
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Clear, rect);
    let title = title(form, width.saturating_sub(2), p);
    frame.render_widget(
        Paragraph::new(body).block(kit::dialog_frame(&title, false, p)),
        rect,
    );
}
