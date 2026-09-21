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
use crate::theme;
use proto::{Runtime, Status};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

#[cfg(test)]
#[path = "dialog_tests.rs"]
mod tests;

/// The label column's width, the milestone brief's `### Rendering:` section's "labels 11
/// columns" — prose, not a numbered design decision.
const LABEL_WIDTH: usize = 11;
/// `"› "` or `"  "` ahead of the label.
const MARKER_WIDTH: usize = 2;

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
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
        FormField::Runtime => "Runtime",
        FormField::Name => "Name",
        FormField::Directory => "Directory",
        FormField::Worktree => "Worktree",
        FormField::Branch => "Branch",
        FormField::Model => "Model",
        FormField::Prompt => "Prompt",
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

fn runtime_span(current: Runtime, candidate: Runtime, label: &'static str) -> Span<'static> {
    if current == candidate {
        Span::styled(
            format!("[{label}]"),
            Style::default().add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw(label)
    }
}

/// The milestone brief's `### Rendering:` section's new-agent block — prose, not a
/// numbered design decision. Width `min(66, area.width - 2)`; the hardware cursor goes
/// to the focused text field's cursor, and nothing else places it (`ui/terminal.rs`
/// already suppresses the PTY cursor while a modal is open).
pub fn render_new_agent(frame: &mut Frame, form: &NewAgentForm, area: Rect) {
    let fields = form.visible_fields();
    let width = 66u16.min(area.width.saturating_sub(2)).max(4);
    let content_width = width.saturating_sub(2) as usize;
    let value_width = content_width.saturating_sub(MARKER_WIDTH + LABEL_WIDTH);

    let error_lines = form
        .error
        .as_deref()
        .map(|message| wrap(&format!("✕ {message}"), content_width, 3))
        .unwrap_or_default();

    let hint = if form.submitting {
        if form.worktree {
            "creating the worktree…"
        } else {
            "creating…"
        }
    } else {
        "Tab next · Shift-Tab back · Enter create · Esc cancel"
    };

    let height = (fields.len() + 1 + error_lines.len() + 1) as u16 + 2;
    let rect = centered(area, width, height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(Line::from(Span::styled(" new agent ", theme::title())));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let mut cursor: Option<(u16, u16)> = None;

    for (row, field) in fields.iter().enumerate() {
        let y = inner.y + row as u16;
        if y >= inner.y + inner.height {
            break;
        }
        let focused = form.focus == *field;
        let marker = if focused { "› " } else { "  " };
        let label_style = if focused {
            Style::default().fg(theme::ACCENT)
        } else {
            Style::default()
        };
        let mut spans = vec![
            Span::styled(marker, label_style),
            Span::styled(
                format!("{:<width$}", field_label(*field), width = LABEL_WIDTH),
                label_style,
            ),
        ];
        match field {
            FormField::Runtime => {
                spans.push(runtime_span(form.runtime, Runtime::Claude, "claude"));
                spans.push(Span::raw("  "));
                spans.push(runtime_span(form.runtime, Runtime::Codex, "codex"));
                spans.push(Span::raw("  "));
                spans.push(runtime_span(form.runtime, Runtime::Shell, "shell"));
            }
            FormField::Worktree => {
                spans.push(Span::raw(format!(
                    "[{}] create a git worktree",
                    if form.worktree { "x" } else { " " }
                )));
            }
            _ => {
                let input = text_input_for(form, *field);
                let (visible, col) = input.visible(value_width as u16);
                if *field == FormField::Name && input.text().is_empty() {
                    spans.push(Span::styled(
                        format!("automatic ({}-N)", form.runtime.label()),
                        theme::muted(),
                    ));
                } else {
                    spans.push(Span::raw(visible));
                }
                if focused {
                    cursor = Some((inner.x + (MARKER_WIDTH + LABEL_WIDTH) as u16 + col, y));
                }
            }
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                y,
                height: 1,
                ..inner
            },
        );
    }

    let mut y = inner.y + fields.len() as u16 + 1;
    for line in &error_lines {
        if y >= inner.y + inner.height {
            break;
        }
        frame.render_widget(
            Paragraph::new(Line::styled(
                line.clone(),
                Style::default().fg(theme::status_color(Status::Attention)),
            )),
            Rect {
                y,
                height: 1,
                ..inner
            },
        );
        y += 1;
    }
    if y < inner.y + inner.height {
        frame.render_widget(
            Paragraph::new(Line::styled(hint, theme::muted())),
            Rect {
                y,
                height: 1,
                ..inner
            },
        );
    }

    if let Some((x, y)) = cursor {
        frame.set_cursor_position((x, y));
    }
}

fn render_box(frame: &mut Frame, title: &str, body: Vec<Line<'static>>, area: Rect) {
    let width = body.iter().map(Line::width).max().unwrap_or(0).max(30) as u16 + 4;
    let height = body.len() as u16 + 2;
    let rect = centered(area, width, height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(Line::from(Span::styled(title, theme::title())));
    frame.render_widget(Paragraph::new(body).block(block), rect);
}

/// Decision 35's remove-confirm rendering. `confirm.branch` is `None` for a window this
/// daemon made no worktree for, which drops the checkbox line and its hint entirely
/// (the wireframe's "for a window without a worktree" case).
pub fn render_remove_confirm(frame: &mut Frame, confirm: &RemoveConfirm, area: Rect) {
    let mut lines = vec![Line::raw(format!("Remove '{}'?", confirm.name))];
    if let Some(branch) = &confirm.branch {
        lines.push(Line::raw(""));
        lines.push(Line::raw(format!(
            "[{}] also remove worktree {branch}",
            if confirm.remove_worktree { "x" } else { " " }
        )));
        // Decision 12's M5.3 note: ignored files are deleted by a plain removal too,
        // same as tracked ones; only the branch survives. Nothing here claims anything
        // about the agent's process (see the module doc comment).
        lines.push(Line::styled(
            "    the branch is kept; ignored files go too",
            theme::muted(),
        ));
    }
    lines.push(Line::raw(""));
    let hint = if confirm.branch.is_some() {
        "Space toggle · y / Enter remove · n / Esc"
    } else {
        "y / Enter remove · n / Esc"
    };
    lines.push(Line::styled(hint, theme::muted()));
    render_box(frame, " remove ", lines, area);
}

const FORCE_WRAP_WIDTH: usize = 50;

/// The dirty-tree force-or-keep follow-up (decision 36). `message` is the daemon's own
/// text (decision 24), which already names the worktree's path and what removing it would
/// destroy, shown exactly as given, wrapped to fit the box.
///
/// `name` is on screen because `f` here deletes a checkout with `--force` and the user has
/// to be able to see *which agent's*. The whole-branch review's finding 1 was a force
/// prompt that named one window in its message while its `f` key targeted another; the
/// wiring that made that possible is fixed in `app/modal_keys.rs`, and this line is what
/// would have made it visible on screen rather than only in a test.
pub fn render_force_remove(frame: &mut Frame, name: &str, message: &str, area: Rect) {
    let mut lines: Vec<Line<'static>> = vec![Line::from(vec![
        Span::raw("agent "),
        Span::styled(
            format!("'{name}'"),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ])];
    lines.extend(
        wrap(message, FORCE_WRAP_WIDTH, 4)
            .into_iter()
            .map(Line::raw),
    );
    if lines.len() == 1 {
        lines.push(Line::raw(message.to_string()));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("f  ", Style::default().fg(theme::ACCENT)),
        Span::raw("force: delete the worktree and everything in it"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("k  ", Style::default().fg(theme::ACCENT)),
        Span::raw("keep the worktree, remove the window"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("n  ", Style::default().fg(theme::ACCENT)),
        Span::raw("cancel"),
    ]));
    render_box(frame, " worktree holds work ", lines, area);
}
