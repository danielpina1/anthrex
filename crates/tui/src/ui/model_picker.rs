//! Milestone 9.8 (MR §5.2, decision 38): the model picker, drawn as a kit dialog over
//! whatever opened it (the Settings screen's table, the goal form, the task edit form).
//! Its title names what it chooses for, and on the right `r refresh · updated <age>
//! ago`, the age given by the caller (preflight F10: the renderer never reads a clock).
//! Per runtime a header, then each model: the current one `●`, the label in 17
//! columns, the description in 38, the efforts; `─────`, `custom…`, a blank row and
//! the keys. On a narrow terminal the description column gives way first, a cut text
//! ending in `…`. Every catalog string was cleaned by the state (`one_line`, trimmed);
//! the frame and `glyphs` fold for ASCII. Pure: the picker and the palette in.

use crate::app::model_picker::{
    CustomModel, ModelPicker, NO_EFFORT, NOT_REPORTED, PickerEntry, PickerFor,
};
use crate::theme::{Palette, Role, ellipsis, role};
use crate::ui::kit::{self, DIALOG_MAX, cut};
use crate::ui::models_table::glyphs;
use proto::Runtime;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

/// The label's columns and the description's (MR §5.2 with the brief's widths).
pub const LABEL_W: usize = 17;
pub const DESCRIPTION_W: usize = 38;
/// The widest the dialog grows: the lines at full width, the padding and the border.
pub const PICKER_MAX: u16 = 74;
/// Columns left free on each side of the dialog, so the screen under it shows.
const MARGIN: u16 = 8;
/// The selection mark, the space, the current mark, the space.
const LEAD_W: usize = 4;

/// `5s`, `2m`, `3h`: how long ago the catalogs were received.
pub fn age_text(age: Duration) -> String {
    let secs = age.as_secs();
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

/// `choose model · <what it chooses for>`.
pub fn title(target: PickerFor) -> String {
    let what = match target {
        PickerFor::Row(role) => role.label(),
        PickerFor::Fallback(role) => format!("{} · if it struggles", role.label()),
        PickerFor::Brainstorm(0) => "brainstorm · first".to_string(),
        PickerFor::Brainstorm(_) => "brainstorm · second".to_string(),
        PickerFor::Goal => "the goal's orchestrator".to_string(),
        PickerFor::TaskEdit => "this task".to_string(),
    };
    format!("choose model · {what}")
}

/// The right of the top border, in `room` columns: `r refresh`, and `updated <age>
/// ago` once a catalog arrived; the key goes first when both do not fit (the footer
/// names it too), then the age.
fn right_title(age: Option<Duration>, room: usize) -> String {
    let updated = age.map(|age| format!("updated {} ago", age_text(age)));
    let options = match &updated {
        Some(u) => vec![format!("r refresh · {u}"), u.clone()],
        None => vec!["r refresh".to_string()],
    };
    (options.into_iter())
        .find(|o| o.width() + 2 <= room)
        .unwrap_or_default()
}

/// `text` padded to `width`, cut with `…` when it does not fit in `width - 1`, so a
/// space always follows it.
fn column(text: &str, width: usize, p: Palette) -> String {
    let text = if text.width() >= width {
        cut(text, width.saturating_sub(1), ellipsis(p))
    } else {
        text.to_string()
    };
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

fn efforts_shown(efforts: &str) -> String {
    if efforts == NO_EFFORT {
        format!("effort {NO_EFFORT}")
    } else {
        efforts.to_string()
    }
}

/// The custom model's row, in place of `custom…` while it is open: the runtime to
/// choose, then the name typed (and why it is not a model).
fn custom_lines(c: &CustomModel, p: Palette) -> Vec<Line<'static>> {
    let lead = format!("    custom{}   ", ellipsis(p));
    if !c.naming {
        let runtimes: Vec<String> = [Runtime::Claude, Runtime::Codex]
            .iter()
            .map(|r| {
                if *r == c.runtime {
                    kit::choice_in(r.label(), p)
                } else {
                    r.label().to_string()
                }
            })
            .collect();
        return vec![Line::raw(glyphs(
            &format!("{lead}runtime {}", runtimes.join(" / ")),
            p,
        ))];
    }
    let name = crate::safe_text::one_line(c.name.text());
    let mut out = vec![Line::from(vec![
        Span::raw(glyphs(&format!("{lead}{}: {name}", c.runtime.label()), p)),
        kit::cursor_block(role(Role::Accent, p), p),
    ])];
    if let Some(error) = &c.error {
        let text = format!("    ✗ {}", crate::safe_text::one_line(error));
        out.push(Line::styled(glyphs(&text, p), role(Role::Failed, p)));
    }
    out
}

/// The entries' lines for `width` interior columns, and the selected entry's line.
pub(crate) fn lines(picker: &ModelPicker, width: usize, p: Palette) -> (Vec<Line<'static>>, usize) {
    let efforts_w = (picker.entries.iter())
        .filter_map(|e| match e {
            PickerEntry::Model { efforts, .. } => Some(efforts_shown(efforts).width()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let description_w = DESCRIPTION_W.min(width.saturating_sub(LEAD_W + LABEL_W + efforts_w));
    let none_current = picker.current.is_none();
    let (mut out, mut at) = (Vec::new(), 0);
    let mut missing = false;
    for (i, entry) in picker.entries.iter().enumerate() {
        let selected = i == picker.selected;
        let mark = |current: bool| {
            format!(
                "{} {} ",
                if selected { "▸" } else { " " },
                if current { "●" } else { " " }
            )
        };
        // A model's description is dimmed apart when the catalog does not list it.
        let mut unlisted = None;
        let (text, dim) = match entry {
            PickerEntry::Header {
                text,
                missing: gone,
                ..
            } => {
                missing = *gone;
                (format!("  {text}"), *gone)
            }
            PickerEntry::Model {
                label,
                description,
                efforts,
                current,
                ..
            } => {
                let lead = format!("{}{}", mark(*current), column(label, LABEL_W, p));
                let rest = format!(
                    "{}{}",
                    column(description, description_w, p),
                    efforts_shown(efforts)
                );
                if description == NOT_REPORTED {
                    unlisted = Some(lead.clone());
                }
                (format!("{lead}{rest}"), missing)
            }
            PickerEntry::NoFallback => (format!("{}none", mark(none_current)), false),
            PickerEntry::RoleTable(text) => (format!("{}{text}", mark(none_current)), false),
            PickerEntry::Custom => {
                out.push(Line::raw(glyphs("  ─────", p)));
                if let Some(c) = &picker.custom {
                    at = out.len();
                    out.extend(custom_lines(c, p));
                    continue;
                }
                (
                    format!("{}custom{}   type any model name", mark(false), ellipsis(p)),
                    false,
                )
            }
        };
        let style = if selected && picker.custom.is_none() {
            role(Role::Accent, p).add_modifier(Modifier::BOLD)
        } else if dim {
            role(Role::Muted, p)
        } else {
            Style::default()
        };
        if selected {
            at = out.len();
        }
        let text = cut(&glyphs(&text, p), width, ellipsis(p));
        let text = text.trim_end().to_string();
        let line = match unlisted.map(|lead| glyphs(&lead, p)) {
            Some(lead) if text.starts_with(&lead) => {
                let rest = text[lead.len()..].to_string();
                Line::from(vec![
                    Span::styled(lead, style),
                    Span::styled(rest, role(Role::Muted, p)),
                ])
            }
            _ => Line::styled(text, style),
        };
        out.push(line);
    }
    (out, at)
}

/// The keys: the list's, or the custom model's step.
fn footer(picker: &ModelPicker, width: usize, p: Palette) -> Line<'static> {
    let text = match &picker.custom {
        None => " j/k move  ⏎ select  r refresh  esc cancel",
        Some(c) if !c.naming => " ←/→ runtime  ⏎ next  esc back",
        Some(_) => " ⏎ select  esc back",
    };
    Line::styled(
        cut(&glyphs(text, p), width, ellipsis(p)),
        role(Role::Muted, p),
    )
}

/// The dialog's width in a terminal `width` wide.
fn dialog_width(width: u16) -> u16 {
    PICKER_MAX.min(width.saturating_sub(2 * MARGIN).max(DIALOG_MAX.min(width)))
}

/// Draws `picker` over `area`, `age` after the catalogs were received.
pub fn render(
    frame: &mut Frame,
    picker: &ModelPicker,
    age: Option<Duration>,
    area: Rect,
    p: Palette,
) {
    let width = dialog_width(area.width);
    let interior = usize::from(width.saturating_sub(4));
    let (entries, at) = lines(picker, interior, p);
    // The border's two rows, the blank row and the keys.
    let room = usize::from(area.height.saturating_sub(4));
    let mut body = kit::window(entries, at, room, p);
    body.push(Line::default());
    body.push(footer(picker, interior, p));
    let height = (body.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let muted = role(Role::Muted, p);
    let title = title(picker.target);
    // The border's corners, the title with its spaces, and one rule between.
    let room = usize::from(width).saturating_sub(2 + title.width() + 2 + 1);
    let mut block = kit::dialog_frame(&title, false, p);
    let right = right_title(age, room);
    if !right.is_empty() {
        let right = format!(" {} ", glyphs(&right, p));
        block = block.title_top(Line::from(Span::styled(right, muted)).right_aligned());
    }
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(body).block(block), rect);
}

#[cfg(test)]
#[path = "model_picker_tests.rs"]
pub(crate) mod tests;
