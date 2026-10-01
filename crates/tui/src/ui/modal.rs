use crate::app::{App, Modal, PendingAction, region::KeyRegion};
use crate::dialog::TextInput;
use crate::theme::{self, Palette, Role, role};
use crate::ui::dialog;
use crate::ui::kit::{self, Hint};
use proto::Status;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The help overlay's rows. Every hint that names the prefix key takes it from
/// `prefix_label` instead of a hard-coded `C-b` (decision 38).
fn help_lines(prefix_label: &str) -> Vec<(String, String)> {
    vec![
        (
            format!("{prefix_label} j / k"),
            "next / previous agent".to_string(),
        ),
        (
            format!("{prefix_label} 1-9"),
            "focus agent by number".to_string(),
        ),
        (format!("{prefix_label} c"), "new agent".to_string()),
        (format!("{prefix_label} ,"), "rename agent".to_string()),
        (format!("{prefix_label} R"), "restart agent".to_string()),
        (format!("{prefix_label} r"), "reconnect".to_string()),
        (format!("{prefix_label} m"), "conversation".to_string()),
        (format!("{prefix_label} g"), "start a goal".to_string()),
        (format!("{prefix_label} a"), "alerts".to_string()),
        (format!("{prefix_label} P"), "profile".to_string()),
        (format!("{prefix_label} S"), "settings".to_string()),
        (
            format!("{prefix_label} t"),
            "tree mode (j/k, h/l, Enter, Space, /)".to_string(),
        ),
        (
            format!("{prefix_label} T"),
            "tree overview (j/k, h/l, wheel, drag)".to_string(),
        ),
        (
            "i".to_string(),
            "in the overview: show / hide the inspector".to_string(),
        ),
        (
            ".".to_string(),
            "actions on the selected run, stage or task".to_string(),
        ),
        (format!("{prefix_label} < / >"), "sidebar width".to_string()),
        (format!("{prefix_label} x"), "kill agent".to_string()),
        (
            format!("{prefix_label} X"),
            "remove agent (and worktree)".to_string(),
        ),
        (format!("{prefix_label} s"), "toggle sidebar".to_string()),
        (
            format!("{prefix_label} d"),
            "detach (agents keep running)".to_string(),
        ),
        (
            format!("{prefix_label} Q"),
            "stop daemon and all agents".to_string(),
        ),
        (
            format!("{prefix_label} {prefix_label}"),
            format!("send a literal {prefix_label}"),
        ),
    ]
}

/// The help's close hint, drawn as the box's bottom title so the key lines keep every
/// row of a 24-row terminal (milestone 9.0.6 task 15).
const HELP_CLOSE: (&str, &str) = ("any key", "close this help");

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
    let accent = role(Role::Accent, p).fg.unwrap_or(p.accent);
    match modal {
        Modal::NewAgent(form) => return dialog::render_new_agent(frame, form, area, accent),
        Modal::Remove(confirm) => {
            return dialog::render_remove_confirm(frame, confirm, area, accent);
        }
        Modal::ForceRemove { name, message, .. } => {
            return dialog::render_force_remove(frame, name, message, area, accent);
        }
        Modal::EditTask(form) => return crate::ui::run_edit::render(frame, form, area, accent),
        Modal::StartGoal(form) => {
            return crate::ui::run_goal::render(frame, form, area, app.palette());
        }
        Modal::Confirm { message, action } => {
            return render_confirm(frame, message, action, area, app.palette());
        }
        Modal::Action(flow) => return crate::ui::action_menu::render(frame, app, flow, area),
        Modal::Help | Modal::Notice { .. } | Modal::Rename(_) => {}
    }
    let (title, body): (String, Vec<Line>) = match modal {
        Modal::Help => (
            " keys ".to_string(),
            help_lines(&app.settings.prefix_label)
                .into_iter()
                .map(|(key, what)| {
                    Line::from(vec![
                        Span::styled(format!("{key:<11}"), Style::default().fg(accent)),
                        Span::raw(what),
                    ])
                })
                .collect(),
        ),
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
                    Some(message) => Line::styled(
                        message.clone(),
                        Style::default().fg(theme::status_color(Status::Attention)),
                    ),
                    None => Line::raw(""),
                },
                Line::styled("Enter = rename    Esc = cancel", theme::muted()),
            ],
        ),
        Modal::Confirm { .. }
        | Modal::NewAgent(_)
        | Modal::Remove(_)
        | Modal::ForceRemove { .. }
        | Modal::EditTask(_)
        | Modal::StartGoal(_)
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
    let block = if matches!(modal, Modal::Help) {
        block.title_bottom(Line::from(vec![
            Span::styled(format!(" {}", HELP_CLOSE.0), Style::default().fg(accent)),
            Span::raw(format!("  {} ", HELP_CLOSE.1)),
        ]))
    } else {
        block
    };
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

    /// Milestone 9.0.5: the help lists `C-b a`, with the configured prefix.
    #[test]
    fn help_lists_the_alerts_key() {
        let lines = help_lines("C-a");
        assert!(
            lines.contains(&("C-a a".to_owned(), "alerts".to_owned())),
            "{lines:?}"
        );
    }

    /// Milestone 9.0.6 decision 43: the three new keys, with the configured prefix; and
    /// at 80×24 every help line and the close hint are drawn (fix round 1: the hint is
    /// the box's bottom title, so 22 key lines and the frame fill the 24 rows).
    #[test]
    fn help_lists_the_new_keys() {
        let want = [
            (".", "actions on the selected run, stage or task"),
            ("C-a P", "profile"),
            ("C-a S", "settings"),
        ];
        let lines = help_lines("C-a");
        for (key, what) in want {
            assert!(
                lines.contains(&(key.to_owned(), what.to_owned())),
                "{key}: {lines:?}"
            );
        }
        let settings = UiSettings {
            prefix_label: "C-a".into(),
            ..UiSettings::default()
        };
        let mut app = App::new(vec![], "/tmp".into(), settings);
        app.modal = Some(Modal::Help);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| render(frame, &app, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text: Vec<String> = (0..24)
            .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        assert_eq!(lines.len(), 22, "{lines:?}");
        for (key, what) in &lines {
            let row = format!("{key:<11}{what}");
            assert!(text.iter().any(|l| l.contains(&row)), "{row}: {text:#?}");
        }
        assert!(text[23].contains("any key  close this help"), "{text:#?}");
        assert!(!text.concat().contains("C-b"), "{text:#?}");
    }
}
