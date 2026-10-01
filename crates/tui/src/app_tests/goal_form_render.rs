//! Milestone 9.0.6 task 12: the goal form drawn (decision 39) at 80x24 and 120x40, in
//! ASCII and unicode.

use super::actions::tap;
use super::goal_form::{app, app_with_cache, focus, form, open_form, roster, typed};
use super::*;
use crate::run_goal::{GoalField, GoalForm};

/// The dialog's rows (the frame included), trailing spaces trimmed.
fn dialog_rows(app: &App, width: u16, height: u16) -> Vec<String> {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let w = width.min(64);
    let x0 = (width - w) / 2;
    (0..height)
        .map(|y| {
            (x0..x0 + w)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .filter(|row| row.starts_with(['+', '|', '┌', '│', '└']))
        .map(|row| row.trim_end().to_string())
        .collect()
}

/// `rows` of a 64-column ASCII dialog: a title row, bar rows padded to the frame, the
/// closing row.
fn ascii_dialog(title: &str, rows: &[&str]) -> Vec<String> {
    let top = format!("+ {title} ");
    let mut out = vec![format!("{top}{}+", "-".repeat(63 - top.len()))];
    out.extend(rows.iter().map(|r| format!("|{r:<62}|")));
    out.push(format!("+{}+", "-".repeat(62)));
    out
}

#[test]
fn goal_form_renders_at_80x24_and_120x40() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = app_with_cache(roster());
        app.settings.badges.ascii = true;
        app.set_terminal_size(w, h);
        open_form(&mut app);
        let want = ascii_dialog(
            "start a goal in /p/a",
            &[
                " > goal",
                "",
                "",
                "",
                "",
                "",
                "   runtime           < configured >",
                "   model             < default >",
                "   trust             < off >",
                "   approve at once   < off >",
                "   unconfined checks < off >",
                "",
                " enter start - tab next - esc cancel",
            ],
        );
        assert_eq!(dialog_rows(&app, w, h), want, "{w}x{h}");
    }
}

#[test]
fn the_unicode_form_draws_its_choices_and_custom_row() {
    let mut app = app_with_cache(roster());
    app.set_terminal_size(120, 40);
    open_form(&mut app);
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Right);
    focus(&mut app, GoalField::Model);
    tap(&mut app, KeyCode::Left);
    typed(&mut app, "x1");
    let rows = dialog_rows(&app, 120, 40).join("\n");
    assert!(rows.contains("‹ claude ›"), "{rows}");
    assert!(rows.contains("‹ custom… ›"), "{rows}");
    assert!(rows.contains("x1"), "{rows}");
    assert!(rows.contains("⏎ start · tab next · esc cancel"), "{rows}");
}

#[test]
fn the_goal_text_area_scrolls_and_hostile_text_is_not_drawn() {
    let mut app = app();
    app.set_terminal_size(80, 24);
    open_form(&mut app);
    for line in ["one", "two", "three", "four", "five", "six"] {
        app.on_paste(line.into());
        press(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL);
    }
    let before = dialog_rows(&app, 80, 24).len();
    let rows = dialog_rows(&app, 80, 24).join("\n");
    assert!(
        rows.contains("^ 3 more") || rows.contains("↑ 3 more"),
        "{rows}"
    );
    // The dialog keeps its height whatever the text.
    app.on_paste("seven".into());
    assert_eq!(dialog_rows(&app, 80, 24).len(), before);
    // A paste's control characters never reach the cells.
    app.on_paste("a\u{1b}[2Jb\u{202E}".into());
    let GoalForm { goal, .. } = form(&app);
    assert!(!goal.text().contains('\u{1b}') && !goal.text().contains('\u{202E}'));
}
