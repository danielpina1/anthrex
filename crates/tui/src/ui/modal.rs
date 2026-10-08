use crate::app::prompt::RenamePrompt;
use crate::app::{App, Modal, PendingAction};
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, role};
use crate::ui::dialog::{self, hint};
use crate::ui::kit::{self, Hint};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The rename row, `name  <text>█`: the input's text with a solid block at the
/// cursor's grapheme position (a reversed cell in ASCII), the box's own way of showing
/// a cursor (task M6.10 brief's mock), scrolled so the cursor stays in `width`.
fn rename_row(prompt: &RenamePrompt, width: u16, p: Palette) -> Line<'static> {
    const LABEL: &str = "name  ";
    let room = width.saturating_sub(LABEL.width() as u16 + 1);
    let (visible, column) = prompt.input.visible(room);
    let graphemes: Vec<&str> = visible.graphemes(true).collect();
    let at = usize::from(column).min(graphemes.len());
    let block = kit::cursor_block(ratatui::style::Style::default(), p);
    Line::from(vec![
        Span::styled(LABEL, role(Role::Muted, p)),
        Span::raw(one_line(&graphemes[..at].concat())),
        block,
        Span::raw(one_line(&graphemes[at..].concat())),
    ])
}

/// Milestone 9.0.7 decision 35: the rename box on the kit's grammar.
fn render_rename(frame: &mut Frame, prompt: &RenamePrompt, area: Rect, p: Palette) {
    let width = dialog::interior(area.width);
    let ellipsis = crate::theme::ellipsis(p);
    let body = vec![
        rename_row(prompt, width, p),
        match &prompt.error {
            Some(message) => Line::styled(
                kit::cut(&one_line(message), usize::from(width), ellipsis),
                role(Role::Failed, p),
            ),
            None => Line::raw(""),
        },
        kit::hints_joined(
            width,
            &[hint("⏎", "rename", 9), hint("esc", "cancel", 1)],
            " · ",
            p,
        ),
    ];
    let rect = kit::dialog_area(area, body.len() as u16);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(body).block(kit::dialog_frame("rename", false, p)),
        rect,
    );
}

/// Milestone 9.0.7 decision 35: the config notice on the kit's grammar, each problem
/// wrapped at 60; any key closes it.
fn render_notice(frame: &mut Frame, title: &str, lines: &[String], area: Rect, p: Palette) {
    let width = dialog::interior(area.width);
    let mut body: Vec<Line<'static>> = lines
        .iter()
        .flat_map(|line| kit::wrap_words(&one_line(line), usize::from(width)))
        .map(Line::raw)
        .collect();
    body.push(Line::raw(""));
    body.push(kit::hints_joined(
        width,
        &[hint("esc", "close", 1)],
        " · ",
        p,
    ));
    let rect = kit::dialog_area(area, body.len() as u16);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(body).block(kit::dialog_frame(title.trim(), false, p)),
        rect,
    );
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
    let hints = kit::hints_joined(width as u16, &[verb, esc], " · ", p);
    body.push(if destructive {
        kit::destructive(hints, "y", action.verb(), p)
    } else {
        hints
    });
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

/// Milestone 9.8 decision 39: a form's model picker, over the form.
fn form_picker(
    frame: &mut Frame,
    app: &App,
    picker: Option<&crate::app::model_picker::ModelPicker>,
    area: Rect,
) {
    if let Some(picker) = picker {
        let age = app.catalogs.updated_age(app.ticked_at);
        crate::ui::model_picker::render(frame, picker, age, area, app.palette());
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(modal) = &app.modal else {
        return;
    };
    let p = app.palette();
    match modal {
        Modal::NewAgent(form) => dialog::render_new_agent(frame, form, area, p),
        Modal::Remove(confirm) => dialog::render_remove_confirm(frame, confirm, area, p),
        Modal::ForceRemove { name, message, .. } => {
            dialog::render_force_remove(frame, name, message, area, p);
        }
        Modal::EditTask(form) => {
            crate::ui::run_edit::render(frame, form, area, p);
            form_picker(frame, app, form.picker.as_ref(), area);
        }
        Modal::StartGoal(form) => {
            crate::ui::run_goal::render(frame, form, area, p);
            form_picker(frame, app, form.picker.as_ref(), area);
        }
        Modal::Iterate(form) => crate::ui::run_iterate::render(frame, form, area, p),
        Modal::IdleMenu(menu) => crate::ui::idle_menu::render(frame, menu, area, p),
        Modal::Confirm { message, action } => render_confirm(frame, message, action, area, p),
        Modal::Action(flow) => crate::ui::action_menu::render(frame, app, flow, area),
        Modal::Help(view) => crate::ui::help::render(frame, app, view, area),
        Modal::Notice { title, lines } => render_notice(frame, title, lines, area, p),
        Modal::Rename(prompt) => render_rename(frame, prompt, area, p),
        Modal::DocNote(form) => crate::ui::doc_note::render(frame, form, area, p),
    }
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
