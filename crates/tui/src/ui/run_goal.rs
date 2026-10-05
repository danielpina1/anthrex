//! Milestone 9 decision 44 and 9.0.6 decision 39: rendering for the goal form
//! (`crate::run_goal`, the pure model), built on the kit's dialog grammar (decision 5):
//! one accented frame titled `start a goal in <project>`, lower-case labels, `‹ value ›`
//! choices, the goal in a four-row text area. Every glyph honours `Palette.ascii`. The
//! project, the model names and what was typed pass `safe_text`.
//!
//! Milestone 9.3 decision 7: this is the compact fallback, drawn below 60 columns or 16
//! rows; at least that, the large editor of `ui/goal_editor.rs` draws instead, with the
//! option rows built here. Both draw the orchestrator row (decision 25) and the confirm
//! page (decision 8). The compact hints read `^S start · tab next · esc cancel`: the text
//! has the editor's keys, so Enter there is a newline (task 9b fix round 1).

use crate::dialog::TextInput;
use crate::run_goal::{GoalField, GoalForm, OrchestratorRow, field_label};
use crate::safe_text::one_line;
use crate::theme::{Glyph, Palette, Role, dot_sep, ellipsis, glyph, role};
use crate::ui::goal_editor;
use crate::ui::kit::{self, Hint};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Rows of the goal's text area.
pub const GOAL_ROWS: u16 = 4;
/// What the empty goal field shows, muted (milestone 9.0.7 decision 35).
pub const PLACEHOLDER: &str = "what should the run achieve?";
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

/// A chosen row: its label and `‹ value ›` fitted to `width` columns ([`fitted_choice`]).
fn choice_line(
    form: &GoalForm,
    field: GoalField,
    value: &str,
    width: usize,
    p: Palette,
) -> Line<'static> {
    let mut spans = label(form, field, p);
    let style = if form.focus == field && !form.submitting {
        role(Role::Accent, p).add_modifier(Modifier::BOLD)
    } else {
        ratatui::style::Style::default()
    };
    spans.push(Span::styled(fitted_choice(value, width, p), style));
    Line::from(spans)
}

/// `‹ value ›` in `width` columns (final fix wave C-m6): a value too long loses whole
/// words from its end and takes the ellipsis, keeping the closing chevron, so
/// `(fresh session)` never ends mid-word; a first word longer than the room is cut
/// inside it.
fn fitted_choice(value: &str, width: usize, p: Palette) -> String {
    let value = one_line(value);
    // The chevrons and their spaces: `‹ ` and ` ›`.
    let room = width.saturating_sub(4);
    if value.width() <= room {
        return kit::choice_in(&value, p);
    }
    let dots = ellipsis(p);
    let mut kept = String::new();
    for word in value.split(' ') {
        let next = if kept.is_empty() {
            word.to_string()
        } else {
            format!("{kept} {word}")
        };
        if next.width() + dots.width() > room {
            break;
        }
        kept = next;
    }
    let text = if kept.is_empty() {
        kit::cut(&value, room, dots)
    } else {
        format!("{kept}{dots}")
    };
    kit::choice_in(&text, p)
}

/// A row whose value is shown but not chosen here, muted (decision 25): a continued
/// chain's runtime and model, or the active chain's line, cut to `width` columns.
fn muted_line(
    form: &GoalForm,
    field: GoalField,
    value: String,
    width: usize,
    p: Palette,
) -> Line<'static> {
    let mut spans = label(form, field, p);
    spans.push(Span::styled(
        kit::cut(&value, width, ellipsis(p)),
        role(Role::Muted, p),
    ));
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

/// The empty goal's first row: [`PLACEHOLDER`] muted, cut to `width`, its first cell
/// the cursor's (reversed) while focused (milestone 9.0.7 decision 35).
pub(crate) fn placeholder(width: usize, focused: bool, p: Palette) -> Line<'static> {
    let text = kit::cut(PLACEHOLDER, width.saturating_sub(1), ellipsis(p));
    let at = (text.char_indices().nth(usize::from(focused))).map_or(text.len(), |(i, _)| i);
    let reversed = ratatui::style::Style::default().add_modifier(Modifier::REVERSED);
    Line::from(vec![
        Span::styled(text[..at].to_string(), reversed),
        Span::styled(text[at..].to_string(), role(Role::Muted, p)),
    ])
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

/// The goal's text area width in a terminal `width` wide (0: not yet drawn), which its
/// Up and Down move by (`GoalForm::on_key_in`).
pub fn goal_width(width: u16) -> u16 {
    let width = width.min(kit::DIALOG_MAX).saturating_sub(4).min(kit::WRAP);
    width.saturating_sub((MARK_W + LABEL_W) as u16)
}

/// The option rows (decision 7's order) for `width` interior columns: runtime, model
/// (and the custom model's text), orchestrator, delivery, design (milestone 9.6), trust,
/// approve at once and unconfined checks. Continuing a chain, its runtime and model show muted.
pub(crate) fn option_lines(form: &GoalForm, width: u16, p: Palette) -> Vec<Line<'static>> {
    let value_w = usize::from(width).saturating_sub(MARK_W + LABEL_W);
    let mut body = Vec::new();
    if let Some(idle) = form.continues() {
        let model = if idle.model.is_empty() {
            "default"
        } else {
            idle.model.as_str()
        };
        for (field, value) in [
            (GoalField::Runtime, idle.runtime.label()),
            (GoalField::Model, model),
        ] {
            body.push(muted_line(
                form,
                field,
                kit::choice_in(value, p),
                value_w,
                p,
            ));
        }
    } else {
        let runtime = match form.runtime {
            Some(runtime) => runtime.label(),
            None => "configured",
        };
        body.push(choice_line(form, GoalField::Runtime, runtime, value_w, p));
        let options = form.model_options();
        let shown = options
            .get(form.model_at())
            .map_or("default", String::as_str);
        let shown = if p.ascii && shown == "custom…" {
            "custom..."
        } else {
            shown
        };
        body.push(choice_line(form, GoalField::Model, shown, value_w, p));
    }
    if form.custom_shown() {
        let focused = form.focus == GoalField::Model && !form.submitting;
        body.push(input_line(&form.custom, value_w, focused));
    }
    // Decision 33: the chain id and the run's short id are drawn sanitised.
    body.push(match form.orchestrator_row() {
        OrchestratorRow::Choice(value) => {
            choice_line(form, GoalField::Orchestrator, &value, value_w, p)
        }
        OrchestratorRow::Busy(text) => {
            muted_line(form, GoalField::Orchestrator, one_line(&text), value_w, p)
        }
    });
    // Milestone 9.2 ruling R-13: `configured` is the repo profile's `[delivery] mode`.
    let delivery = match form.delivery {
        None => "configured",
        Some(proto::DeliveryMode::Local) => "local",
        Some(proto::DeliveryMode::Pr) => "pr",
    };
    body.push(choice_line(form, GoalField::Delivery, delivery, value_w, p));
    // Milestone 9.6: `configured` is `[orchestrator.design].default`, the daemon's.
    let design = match form.design {
        None => "configured",
        Some(proto::DesignMode::Full) => "full",
        Some(proto::DesignMode::Off) => "off",
    };
    body.push(choice_line(form, GoalField::Design, design, value_w, p));
    body.push(choice_line(
        form,
        GoalField::Trust,
        on_off(form.trust_project),
        value_w,
        p,
    ));
    body.push(choice_line(
        form,
        GoalField::Yes,
        on_off(form.yes),
        value_w,
        p,
    ));
    body.push(choice_line(
        form,
        GoalField::UnconfinedChecks,
        on_off(form.unconfined_checks),
        value_w,
        p,
    ));
    body
}

/// The dialog's rows for `width` interior columns: the fields, the error, a blank row
/// and the hints.
pub fn body(form: &GoalForm, width: u16, p: Palette) -> Vec<Line<'static>> {
    let width = width.min(kit::WRAP);
    let value_w = usize::from(width).saturating_sub(MARK_W + LABEL_W);
    let mut body = Vec::new();

    // The text area takes `rows + 2` lines, so the dialog keeps its height when a
    // scroll mark appears.
    let focused = form.focus == GoalField::Goal && !form.submitting;
    let mut area = kit::text_area_focus(&form.goal, GOAL_ROWS, value_w as u16, focused, p);
    if form.goal.is_empty() {
        area[0] = placeholder(value_w, focused, p);
    }
    area.resize(usize::from(GOAL_ROWS) + 2, Line::default());
    // The label sits on the first text row, below a leading `↑ n more` mark.
    let first = usize::from(kit::starts_with_mark(&area));
    for (i, line) in area.into_iter().enumerate() {
        let mut spans = if i == first {
            label(form, GoalField::Goal, p)
        } else {
            vec![indent()]
        };
        spans.extend(line.spans);
        body.push(Line::from(spans));
    }

    body.extend(option_lines(form, width, p));

    if let Some(error) = &form.error {
        body.push(Line::styled(
            kit::cut(&one_line(error), usize::from(width), ellipsis(p)),
            role(Role::Failed, p),
        ));
    }
    body.push(Line::raw(""));
    if form.submitting {
        body.push(Line::styled(starting_text(form, p), role(Role::Muted, p)));
        body.push(kit::hints_joined(
            width,
            &[hint("esc", "close", 1)],
            dot_sep(p),
            p,
        ));
    } else {
        let keys = [
            hint("^S", "start", 9),
            hint("tab", "next", 6),
            hint("esc", "cancel", 1),
        ];
        body.push(kit::hints_joined(width, &keys, dot_sep(p), p));
    }
    body
}

/// The row while the goal is sent: `starting…`, and for a new orchestrator `triage can
/// take minutes`; a continued goal is not triaged (decision 22). Both dialogs draw it
/// (final fix wave C-m7).
pub(crate) fn starting_text(form: &GoalForm, p: Palette) -> String {
    if form.continues().is_some() {
        format!("starting{}", ellipsis(p))
    } else {
        format!("starting{} triage can take minutes", ellipsis(p))
    }
}

/// The goal dialog over the whole terminal `area`: the large editor from 60×16
/// (`ui/goal_editor.rs`), else this compact layout; the confirm page in the dialog's
/// place while it is open.
pub fn render(frame: &mut Frame, form: &GoalForm, area: Rect, p: Palette) {
    if goal_editor::is_large(area.width, area.height) {
        return goal_editor::render(frame, form, area, p);
    }
    let width = area.width.min(kit::DIALOG_MAX).saturating_sub(4);
    let body = body(form, width, p);
    let rect = kit::dialog_area(area, body.len() as u16);
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Clear, rect);
    if form.discarding {
        return goal_editor::render_discard(frame, rect, p);
    }
    let title = title(form, width.saturating_sub(2), p);
    frame.render_widget(
        Paragraph::new(body).block(kit::dialog_frame(&title, false, p)),
        rect,
    );
}
