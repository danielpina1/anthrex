use crate::app::{App, Modal, PendingAction, region::KeyRegion};
use crate::dialog::TextInput;
use crate::theme::{Palette, Role, role};
use crate::ui::dialog;
use crate::ui::kit::{self, Hint};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// `input`'s text with a solid block drawn at the cursor's grapheme position — the
/// rename box's own way of showing a cursor (task M6.10 brief's mock), rather than the
/// new-agent form's hardware cursor (`ui/dialog.rs`'s `render_new_agent`), because this
/// box is a single line inside the generic (title, body) modal path below, which has
/// nowhere to report a cursor position back to.
fn text_with_cursor_block(input: &TextInput) -> String {
    let graphemes: Vec<&str> = input.text().graphemes(true).collect();
    let cursor = input.cursor().min(graphemes.len());
    let mut out = String::with_capacity(input.text().len() + 3);
    out.push_str(&graphemes[..cursor].concat());
    out.push('█');
    out.push_str(&graphemes[cursor..].concat());
    out
}

/// `text` as it fits `width` columns: one line when it fits, else broken at spaces
/// (a word wider than `width` keeps its own line, and the box cuts it).
fn wrapped(text: &str, width: usize) -> Vec<Line<'static>> {
    if text.width() <= width {
        return vec![Line::raw(text.to_string())];
    }
    let mut lines = vec![];
    let mut line = String::new();
    for word in text.split(' ') {
        if !line.is_empty() && line.width() + 1 + word.width() > width {
            lines.push(Line::raw(std::mem::take(&mut line)));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    lines.push(Line::raw(line));
    lines
}

/// A confirm on the kit's dialog grammar (milestone 9.0.6 decision 5): the message
/// wrapped at 60 columns with no control, separator or bidi character (it may quote a hold
/// id or a daemon text), a blank line, the key hints. A destructive confirm's title and
/// verb draw in `Failed` and its hint offers only `y`.
fn render_confirm(
    frame: &mut Frame,
    message: &str,
    action: &PendingAction,
    area: Rect,
    p: Palette,
) {
    let destructive = action.destructive();
    let width = usize::from(area.width.min(kit::DIALOG_MAX).saturating_sub(4));
    let mut body = wrapped(
        &crate::safe_text::one_line(message),
        width.min(usize::from(kit::WRAP)),
    );
    body.push(Line::raw(""));
    let key = if destructive { "y" } else { "⏎" };
    let verb = Hint {
        key: key.to_string(),
        word: action.verb().to_string(),
        priority: 9,
    };
    let esc = Hint {
        key: "esc".to_string(),
        word: "back".to_string(),
        priority: 1,
    };
    let mut hints = kit::hints_joined(width as u16, &[verb, esc], " · ", p);
    if destructive {
        // Style the key and the verb by what they are, not where they sit.
        let failed = role(Role::Failed, p);
        for span in hints.spans.iter_mut() {
            if span.content == "y" || span.content == action.verb() {
                span.style = failed;
            }
        }
    }
    body.push(hints);
    let title = if destructive {
        action.verb()
    } else {
        "confirm"
    };
    let rect = kit::dialog_area(area, body.len() as u16);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(body).block(kit::dialog_frame(title, destructive, p)),
        rect,
    );
}

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

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(modal) = &app.modal else {
        return;
    };
    let p = app.palette();
    match modal {
        Modal::NewAgent(form) => return dialog::render_new_agent(frame, form, area, p),
        Modal::Remove(confirm) => {
            return dialog::render_remove_confirm(frame, confirm, area, p);
        }
        Modal::ForceRemove { name, message, .. } => {
            return dialog::render_force_remove(frame, name, message, area, p);
        }
        Modal::EditTask(form) => return crate::ui::run_edit::render(frame, form, area, p),
        Modal::StartGoal(form) => {
            return crate::ui::run_goal::render(frame, form, area, app.palette());
        }
        Modal::Confirm { message, action } => {
            return render_confirm(frame, message, action, area, app.palette());
        }
        Modal::Action(flow) => return crate::ui::action_menu::render(frame, app, flow, area),
        Modal::Help(view) => return crate::ui::help::render(frame, app, view, area),
        Modal::Notice { .. } | Modal::Rename(_) => {}
    }
    let (title, body): (String, Vec<Line>) = match modal {
        Modal::Notice { title, lines } => (
            title.clone(),
            lines.iter().map(|line| Line::raw(line.clone())).collect(),
        ),
        // Task M6.10 brief's mock: the input line, then the error (or a blank line
        // when there is none, keeping the box a constant three body lines whether or
        // not `error` is set), then the hint.
        Modal::Rename(prompt) => (
            " rename ".to_string(),
            vec![
                Line::raw(text_with_cursor_block(&prompt.input)),
                match &prompt.error {
                    Some(message) => Line::styled(message.clone(), role(Role::Failed, p)),
                    None => Line::raw(""),
                },
                Line::styled("Enter = rename    Esc = cancel", role(Role::Muted, p)),
            ],
        ),
        Modal::Confirm { .. }
        | Modal::NewAgent(_)
        | Modal::Remove(_)
        | Modal::ForceRemove { .. }
        | Modal::EditTask(_)
        | Modal::StartGoal(_)
        | Modal::Help(_)
        | Modal::Action(_) => {
            unreachable!("handled and returned from above")
        }
    };
    let width = body.iter().map(Line::width).max().unwrap_or(0).max(30) as u16 + 4;
    let height = body.len() as u16 + 2;
    let rect = centered(area, width, height);
    frame.render_widget(Clear, rect);
    let keys_here = app.key_region() == KeyRegion::Dialog;
    let block = kit::pane_frame(Line::from(title.trim().to_owned()), keys_here, p);
    frame.render_widget(Paragraph::new(body).block(block), rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::PendingAction;
    use crate::settings::UiSettings;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn reject() -> PendingAction {
        PendingAction::RejectRun("add-reset-3f9a".into())
    }

    fn confirm_lines(message: &str, action: PendingAction, width: u16, height: u16) -> Vec<String> {
        let mut app = App::new(vec![], "/tmp".into(), UiSettings::default());
        app.modal = Some(Modal::Confirm {
            message: message.into(),
            action,
        });
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &app, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    /// The box's inner text, one line per row, borders and padding trimmed.
    fn inner(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .filter(|l| l.contains('│'))
            .map(|l| l.trim().trim_matches('│').trim().to_string())
            .collect()
    }

    #[test]
    fn a_long_confirm_wraps_inside_its_box_at_80_columns() {
        let message = "Reject run add-reset-3f9a? Its branches and worktrees are removed; \
                       salvage refs are kept.";
        let lines = confirm_lines(message, reject(), 80, 24);
        let inner = inner(&lines);
        let text = inner.iter().filter(|l| !l.is_empty()).cloned();
        let joined = text.collect::<Vec<_>>().join(" ");
        assert!(joined.starts_with(message), "{lines:#?}");
        assert!(joined.ends_with("y reject · esc back"));
        let box_rows: Vec<_> = lines.iter().filter(|l| l.contains('│')).collect();
        assert!(
            box_rows.iter().all(|l| l.trim_end().ends_with('│')),
            "{lines:#?}"
        );
    }

    /// M9.15 review: a confirm quoting a hold id carries none of `safe_text`'s hostile
    /// characters into the lines it draws.
    #[test]
    fn a_confirm_quoting_a_hold_id_is_sanitised() {
        let bad = crate::safe_text::tests::hostile_text();
        let message = format!("Reject hold epic:{bad} of run r? Its 1 task is cancelled.");
        let text = confirm_lines(&message, reject(), 200, 24).join("");
        assert!(text.contains("Reject hold epic:a b"), "{text:?}");
        assert_eq!(
            crate::safe_text::tests::first_hostile(&text),
            None,
            "{text:?}"
        );
    }

    #[test]
    fn a_short_confirm_stays_on_one_line() {
        let lines = confirm_lines("Kill 'a'?", PendingAction::Kill(1), 80, 24);
        assert_eq!(inner(&lines), vec!["Kill 'a'?", "", "y kill · esc back"]);
    }
}
