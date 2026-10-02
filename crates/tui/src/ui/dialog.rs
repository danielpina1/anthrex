//! Rendering for the new-agent form, the remove-confirm dialog and the dirty-worktree
//! force follow-up. `crates/tui/src/dialog.rs` (task M5.8) is the pure model this reads;
//! `ui::modal::render` dispatches `Modal::NewAgent`, `Modal::Remove` and
//! `Modal::ForceRemove` here instead of its own generic (title, body) rendering.
//!
//! The force follow-up's own lines say nothing about *what kind* of work the checkout
//! holds: the daemon's message is the only thing that knows (it may be a rebase, a
//! bisect, unreachable commits or plain uncommitted files — see
//! `daemon::worktree::DirtyReason`), so the title and the `f` line describe the worktree
//! as a whole. A prompt that said "changes" would be false for exactly the states whose
//! loss is worst.
//!
//! Neither the remove-confirm dialog nor the force follow-up says anything about the
//! agent's process. `WindowManager::remove_with_worktree`'s dirty check
//! (`crates/daemon/src/manager/remove.rs`) runs *before* the agent is signalled, so on
//! the common dirty-refusal path the agent is still running when the force prompt is on
//! screen; a prompt that claimed its process was already gone would be wrong exactly
//! when it matters. Both dialogs describe only what happens to the worktree's files,
//! which is true on every path.

use crate::dialog::{FormField, NewAgentForm, RemoveConfirm, TextInput};
use crate::safe_text::one_line;
use crate::theme::{Glyph, Palette, Role, fold, glyph, role};
use crate::ui::kit::{self, Hint};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
#[path = "dialog_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "dialog_kit_tests.rs"]
mod kit_tests;

/// The label column's width: the longest label, `directory` (and the edit form's `test
/// mode`), and two spaces.
pub(crate) const LABEL_WIDTH: usize = 11;
/// The focus marker's columns ahead of the label.
pub(crate) const MARKER_WIDTH: usize = 2;

pub(crate) fn hint(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    }
}

/// The hint row while a dialog waits on the daemon: `<what>… · esc close`, the status
/// dropped first when the row is too narrow for both, so `esc` stays (decision 33).
pub(crate) fn busy_hint(what: &str, width: u16, p: Palette) -> Line<'static> {
    let what = fold(what, p.ascii);
    let separator = fold(" · ", p.ascii);
    let esc = kit::hints_joined(width, &[hint("esc", "close", 1)], " · ", p);
    if what.width() + separator.width() + esc.width() > usize::from(width) {
        return esc;
    }
    let muted = role(Role::Muted, p);
    let mut spans = vec![Span::styled(what, muted), Span::styled(separator, muted)];
    spans.extend(esc.spans);
    Line::from(spans)
}

/// The interior's width in an area `width` wide: the dialog's 64 columns less the
/// borders and the padding, never past the 60 text wraps at.
pub(crate) fn interior(width: u16) -> u16 {
    width.min(kit::DIALOG_MAX).saturating_sub(4).min(kit::WRAP)
}

/// `body` in a `kit::dialog_frame` titled `title`, placed by `kit::dialog_area`.
fn render_dialog(
    frame: &mut Frame,
    title: &str,
    destructive: bool,
    body: Vec<Line<'static>>,
    area: Rect,
    p: Palette,
) -> Rect {
    let rect = kit::dialog_area(area, body.len() as u16);
    if rect.width == 0 || rect.height == 0 {
        return rect;
    }
    frame.render_widget(Clear, rect);
    let block = kit::dialog_frame(title, destructive, p);
    let inner = block.inner(rect);
    frame.render_widget(Paragraph::new(body).block(block), rect);
    inner
}

/// Greedy word-wrap capped at `max_lines`. Used for the new-agent form's inline error
/// (arbitrary validation or daemon text, decisions 32 and 33) and the force prompt's
/// dirty message (arbitrary git stderr via decision 14): neither is written for a fixed
/// width, so both need to fit whatever box this milestone draws them into.
fn wrap(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let extra = usize::from(!current.is_empty());
        if current.chars().count() + extra + word.chars().count() <= width {
            if extra == 1 {
                current.push(' ');
            }
            current.push_str(word);
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            if lines.len() == max_lines {
                return lines;
            }
        }
        if word.chars().count() > width {
            // A single word wider than the whole line: hard-cut it into `width`-wide
            // pieces across as many lines as it takes, continuing the remainder on the
            // lines that follow rather than dropping it. The word that hits this in
            // practice is the worktree path in the force-remove message, and macOS's
            // default data-directory path is long enough to hit it routinely — the
            // dialog exists to tell the user *which* worktree holds uncommitted work
            // immediately before offering to delete it, so losing the tail of that path
            // undercuts the one question it answers.
            let mut remainder: &str = word;
            loop {
                let take = remainder.chars().take(width).count();
                let split_at = remainder
                    .char_indices()
                    .nth(take)
                    .map(|(i, _)| i)
                    .unwrap_or(remainder.len());
                let (piece, rest) = remainder.split_at(split_at);
                lines.push(piece.to_string());
                if lines.len() == max_lines {
                    return lines;
                }
                if rest.is_empty() {
                    break;
                }
                remainder = rest;
            }
        } else {
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines.truncate(max_lines);
    lines
}

fn field_label(field: FormField) -> &'static str {
    match field {
        FormField::Runtime => "runtime",
        FormField::Name => "name",
        FormField::Directory => "directory",
        FormField::Worktree => "worktree",
        FormField::Branch => "branch",
        FormField::Model => "model",
        FormField::Prompt => "prompt",
    }
}

fn text_input_for(form: &NewAgentForm, field: FormField) -> &TextInput {
    match field {
        FormField::Name => &form.name,
        FormField::Directory => &form.dir,
        FormField::Branch => &form.branch,
        FormField::Model => &form.model,
        FormField::Prompt => &form.prompt,
        FormField::Runtime | FormField::Worktree => {
            unreachable!("Runtime and Worktree are not text fields")
        }
    }
}

/// `marker label`: the focused field's label is accented and bold and led by the
/// selection glyph, the others muted (the goal form's grammar).
fn label(field: FormField, focused: bool, p: Palette) -> Vec<Span<'static>> {
    let (mark, style) = if focused {
        (
            glyph(Glyph::Selection, p.ascii),
            role(Role::Accent, p).add_modifier(Modifier::BOLD),
        )
    } else {
        (" ", role(Role::Muted, p))
    };
    vec![
        Span::styled(format!("{mark:<MARKER_WIDTH$}"), style),
        Span::styled(format!("{:<LABEL_WIDTH$}", field_label(field)), style),
    ]
}

/// Milestone 9.0.7 decision 35: the new-agent form on the kit's grammar, `kit::
/// dialog_area` wide (at most 64), titled `new agent`: one row a field, the error
/// (`✗ <message>`, wrapped at 60, at most three lines), a blank row and the hints. The
/// hardware cursor goes to the focused text field's cursor, and nothing else places it
/// (`ui/terminal.rs` already suppresses the PTY cursor while a modal is open).
pub fn render_new_agent(frame: &mut Frame, form: &NewAgentForm, area: Rect, p: Palette) {
    let width = interior(area.width);
    let value_w = usize::from(width).saturating_sub(MARKER_WIDTH + LABEL_WIDTH);
    let mut body = Vec::new();
    let mut cursor = None;
    for field in form.visible_fields() {
        let focused = form.focus == field && !form.submitting;
        let mut spans = label(field, focused, p);
        let chosen = if focused {
            role(Role::Accent, p).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        match field {
            FormField::Runtime => {
                spans.push(Span::styled(
                    kit::choice_in(form.runtime.label(), p),
                    chosen,
                ));
            }
            FormField::Worktree => {
                let value = if form.worktree { "on" } else { "off" };
                spans.push(Span::styled(kit::choice_in(value, p), chosen));
                spans.push(Span::raw(" create a git worktree"));
            }
            _ => {
                let input = text_input_for(form, field);
                let (visible, column) = input.visible(value_w as u16);
                // The column on the text as drawn, sanitised (a dropped character
                // takes none).
                let before: String = visible.graphemes(true).take(usize::from(column)).collect();
                let column = one_line(&before).width() as u16;
                if field == FormField::Name && input.text().is_empty() {
                    spans.push(Span::styled(
                        format!("automatic ({}-N)", form.runtime.label()),
                        role(Role::Muted, p),
                    ));
                } else {
                    spans.push(Span::raw(one_line(&visible)));
                }
                if focused {
                    cursor = Some((body.len() as u16, column));
                }
            }
        }
        body.push(Line::from(spans));
    }
    if let Some(message) = &form.error {
        let text = format!("{} {}", glyph(Glyph::Failed, p.ascii), one_line(message));
        for line in wrap(&text, usize::from(width), 3) {
            body.push(Line::styled(line, role(Role::Failed, p)));
        }
    }
    body.push(Line::raw(""));
    body.push(if form.submitting {
        let what = if form.worktree {
            "creating the worktree…"
        } else {
            "creating…"
        };
        busy_hint(what, width, p)
    } else {
        let keys = [
            hint("⏎", "create", 9),
            hint("tab", "next", 5),
            hint("esc", "cancel", 1),
        ];
        kit::hints_joined(width, &keys, " · ", p)
    });
    let inner = render_dialog(frame, "new agent", false, body, area, p);
    if let Some((row, column)) = cursor
        && row < inner.height
        && value_w > 0
    {
        let x = inner.x + (MARKER_WIDTH + LABEL_WIDTH) as u16 + column;
        frame.set_cursor_position((x, inner.y + row));
    }
}

/// Decision 35's remove confirm, destructive (milestone 9.0.7 decision 35: the title
/// and the `y remove` hint in `Failed`, `y` only). `confirm.branch` is `None` for a
/// window this daemon made no worktree for, which drops the worktree choice and its
/// hint entirely.
pub fn render_remove_confirm(frame: &mut Frame, confirm: &RemoveConfirm, area: Rect, p: Palette) {
    let width = interior(area.width);
    let muted = role(Role::Muted, p);
    let question = format!("Remove '{}'?", one_line(&confirm.name));
    let mut body: Vec<Line<'static>> = kit::wrap_words(&question, usize::from(width))
        .into_iter()
        .map(Line::raw)
        .collect();
    let mut keys = Vec::new();
    if let Some(branch) = &confirm.branch {
        let choice = kit::choice_in(if confirm.remove_worktree { "yes" } else { "no" }, p);
        let lead = "also remove worktree ";
        let room = usize::from(width).saturating_sub(lead.width() + 2 + choice.width());
        let branch = crate::ui::tree_view::truncate_in(&one_line(branch), room, p.ascii);
        body.push(Line::raw(""));
        body.push(Line::raw(format!("{lead}{branch}  {choice}")));
        // Decision 12's M5.3 note: ignored files are deleted by a plain removal too,
        // same as tracked ones; only the branch survives. Nothing here claims anything
        // about the agent's process (see the module doc comment).
        body.push(Line::styled(
            "the branch is kept; ignored files go too",
            muted,
        ));
        keys.push(hint("space", "toggle", 5));
    }
    body.push(Line::raw(""));
    keys.push(hint("y", "remove", 9));
    keys.push(hint("esc", "cancel", 1));
    body.push(kit::destructive(
        kit::hints_joined(width, &keys, " · ", p),
        "y",
        "remove",
        p,
    ));
    render_dialog(frame, "remove", true, body, area, p);
}

const FORCE_WRAP_WIDTH: usize = 50;

/// The message's line budget: `DirtyReason`'s longest sentence, wrapped at
/// [`FORCE_WRAP_WIDTH`] and prefixed with `worktree <path> `, needs more than the 4 lines
/// this prompt used to allow. Measured against a realistic path
/// (`<worktrees_root>/<project>-<hash8>/<branch-dir>`, ~79 columns) and every
/// `DirtyReason` sentence, the six messages run 177–193 columns; 4 × 50 = 200 looked
/// sufficient by raw character count, but the path alone is one unbroken "word" that
/// consumes two of those four lines once `wrap` stopped dropping its tail (wave B), which
/// left only two lines — 100 columns — for the sentence that says what forcing away
/// destroys. 6 lines keeps `FORCE_WRAP_WIDTH` and the box's width unchanged (a
/// same-width, taller box, not a wider one) while raising the budget to 300 columns,
/// comfortably above the longest measured message with room for a longer path or branch
/// name than the ones measured. See `dialog_tests.rs` for the rendered proof.
const FORCE_MAX_LINES: usize = 6;

/// The dirty-tree force-or-keep follow-up (decision 36). `message` is the daemon's own
/// text (decision 24), which already names the worktree's path and what removing it would
/// destroy, shown exactly as given, wrapped to fit the box.
///
/// `name` is on screen because `f` here deletes a checkout with `--force` and the user has
/// to be able to see *which agent's*. The whole-branch review's finding 1 was a force
/// prompt that named one window in its message while its `f` key targeted another; the
/// wiring that made that possible is fixed in `app/modal_keys.rs`, and this line is what
/// would have made it visible on screen rather than only in a test.
pub fn render_force_remove(frame: &mut Frame, name: &str, message: &str, area: Rect, p: Palette) {
    let width = usize::from(interior(area.width));
    let mut body: Vec<Line<'static>> = vec![Line::from(vec![
        Span::raw("agent "),
        Span::styled(
            format!("'{}'", one_line(name)),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ])];
    let message = one_line(message);
    let wrapped = wrap(&message, FORCE_WRAP_WIDTH.min(width), FORCE_MAX_LINES);
    if wrapped.is_empty() {
        body.push(Line::raw(message));
    }
    body.extend(wrapped.into_iter().map(Line::raw));
    body.push(Line::raw(""));
    let key = |k: &'static str, r: Role, words: &'static str| {
        Line::from(vec![Span::styled(k, role(r, p)), Span::raw(words)])
    };
    body.push(key(
        "f  ",
        Role::Failed,
        "force: delete the worktree and everything in it",
    ));
    body.push(key(
        "k  ",
        Role::Accent,
        "keep the worktree, remove the window",
    ));
    body.push(key("n  ", Role::Accent, "cancel"));
    render_dialog(frame, "worktree holds work", true, body, area, p);
}
