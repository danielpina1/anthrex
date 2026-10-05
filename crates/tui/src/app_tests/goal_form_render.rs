//! Milestone 9.0.6 task 12: the goal form drawn (decision 39) at 80x24 and 120x40, in
//! ASCII and unicode. Milestone 9.3 decision 7 (changed expectations): from 60×16 the
//! dialog is the large editor, so the compact drawing's tests run at 59×24.

use super::actions::tap;
use super::goal_form::{app, app_with_cache, focus, form, open_form, roster, typed};
use super::*;
use crate::run_goal::{GoalField, GoalForm};

/// The compact dialog's rows (the frame included), trailing spaces trimmed.
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

/// `rows` of a `width`-column ASCII dialog: a title row, bar rows padded to the frame,
/// the closing row.
fn ascii_dialog(title: &str, width: usize, rows: &[&str]) -> Vec<String> {
    let top = format!("+ {title} ");
    let mut out = vec![format!("{top}{}+", "-".repeat(width - 1 - top.len()))];
    out.extend(rows.iter().map(|r| format!("|{r:<w$}|", w = width - 2)));
    out.push(format!("+{}+", "-".repeat(width - 2)));
    out
}

/// Milestone 9.3 (changed expectation): at 80x24 and 120x40 the large editor, its text
/// 10 and 26 rows, the options under a blank row, the position row and the footer.
/// Milestone 9.6 task 18 (changed expectation): 9 and 25 rows, the design row added.
#[test]
fn goal_form_renders_at_80x24_and_120x40() {
    for (w, h, width, text) in [(80u16, 24u16, 76usize, 9usize), (120, 40, 116, 25)] {
        let mut app = app_with_cache(roster());
        app.settings.badges.ascii = true;
        app.set_terminal_size(w, h);
        open_form(&mut app);
        let buffer = crate::ui::audit::draw(&app, w, h);
        let rows: Vec<String> = crate::ui::audit::rows(&buffer)[..h as usize - 2]
            .iter()
            .map(|r| r.chars().skip(2).take(width).collect())
            .collect();
        let pad = |r: &str| format!("| {r:<w$} |", w = width - 4);
        let top = "+ start a goal in /p/a ";
        assert_eq!(
            rows[0],
            format!("{top}{}+", "-".repeat(width - 1 - top.len()))
        );
        assert_eq!(rows[1], pad("what should the run achieve?"));
        assert!(rows[2..=text].iter().all(|r| *r == pad("")), "{w}x{h}");
        assert_eq!(rows[text + 1], pad(""));
        assert_eq!(rows[text + 2], pad("  runtime           < configured >"));
        assert_eq!(rows[text + 4], pad("  orchestrator      < new >"));
        assert_eq!(rows[text + 6], pad("  design            < configured >"));
        assert_eq!(rows[text + 9], pad("  unconfined checks < off >"));
        assert_eq!(rows[text + 10], pad("ln 1, col 1 - 0 / 16,384"));
        assert_eq!(
            rows[text + 11],
            pad("^S start  Tab options  ^K cut  ^U paste  Esc cancel")
        );
        assert_eq!(rows[text + 12], format!("+{}+", "-".repeat(width - 2)));
    }
}

/// Milestone 9.3: below 60 columns the compact drawing, today's rows with the
/// orchestrator row.
#[test]
fn goal_form_renders_compact_at_59x24() {
    let mut app = app_with_cache(roster());
    app.settings.badges.ascii = true;
    app.set_terminal_size(59, 24);
    open_form(&mut app);
    let want = ascii_dialog(
        "start a goal in /p/a",
        59,
        &[
            " > goal              what should the run achieve?",
            "",
            "",
            "",
            "",
            "",
            "   runtime           < configured >",
            "   model             < default >",
            "   orchestrator      < new >",
            "   delivery          < configured >",
            "   design            < configured >",
            "   trust             < off >",
            "   approve at once   < off >",
            "   unconfined checks < off >",
            "",
            " ^S start - tab next - esc cancel",
        ],
    );
    // 18 rows (milestone 9.6's design row; changed expectation), centred in 24; the
    // dialog is the terminal's whole width.
    let rows = crate::ui::audit::rows(&crate::ui::audit::draw(&app, 59, 24));
    assert_eq!(rows[3..21], want);
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
    let rows = crate::ui::audit::rows(&crate::ui::audit::draw(&app, 120, 40)).join("\n");
    assert!(rows.contains("‹ claude ›"), "{rows}");
    assert!(rows.contains("‹ custom… ›"), "{rows}");
    assert!(rows.contains("x1"), "{rows}");
    // Milestone 9.3 (changed expectation): the large editor's footer.
    assert!(
        rows.contains("^S start  Tab options  ^K cut  ^U paste  Esc cancel"),
        "{rows}"
    );
}

/// Milestone 9.3 (changed expectation): the compact dialog's scroll marks, at 59x24.
#[test]
fn the_goal_text_area_scrolls_and_hostile_text_is_not_drawn() {
    let mut app = app();
    app.set_terminal_size(59, 24);
    open_form(&mut app);
    for line in ["one", "two", "three", "four", "five", "six"] {
        app.on_paste(line.into());
        press(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL);
    }
    let before = dialog_rows(&app, 59, 24).len();
    let rows = dialog_rows(&app, 59, 24).join("\n");
    assert!(
        rows.contains("^ 3 more") || rows.contains("↑ 3 more"),
        "{rows}"
    );
    // The dialog keeps its height whatever the text.
    app.on_paste("seven".into());
    assert_eq!(dialog_rows(&app, 59, 24).len(), before);
    // A paste's control characters never reach the cells.
    app.on_paste("a\u{1b}[2Jb\u{202E}".into());
    let GoalForm { goal, .. } = form(&app);
    assert!(!goal.text().contains('\u{1b}') && !goal.text().contains('\u{202E}'));
}

/// Milestone 9.3 (changed expectation): the compact dialog's label, at 59x24.
#[test]
fn the_goal_label_sits_on_the_first_text_row_and_the_cursor_only_when_focused() {
    let mut app = app();
    app.set_terminal_size(59, 24);
    open_form(&mut app);
    for line in ["one", "two", "three", "four", "five", "six"] {
        app.on_paste(line.into());
        press(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL);
    }
    let rows = dialog_rows(&app, 59, 24);
    let mark = rows
        .iter()
        .position(|r| r.contains("↑ 3 more"))
        .unwrap_or_else(|| panic!("{rows:?}"));
    assert!(!rows[mark].contains("goal"), "{rows:?}");
    assert!(rows[mark + 1].contains("▌ goal"), "{rows:?}");

    // The kit draws the cursor only for a focused area.
    let p = app.palette();
    let reversed = |focused| {
        crate::ui::kit::text_area_focus(&form(&app).goal, 4, 40, focused, p)
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|s| {
                s.style
                    .add_modifier
                    .contains(ratatui::style::Modifier::REVERSED)
            })
    };
    assert!(reversed(true));
    assert!(!reversed(false));
    focus(&mut app, GoalField::Runtime);
    // Not focused: the goal's rows carry no reversed cell on screen.
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(59, 24)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let any_reversed = buffer
        .content()
        .iter()
        .any(|c| c.modifier.contains(ratatui::style::Modifier::REVERSED));
    assert!(!any_reversed);
}
