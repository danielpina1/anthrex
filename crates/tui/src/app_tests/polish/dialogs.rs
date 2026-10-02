//! M9.0.7.13: the goal field's placeholder and the task edit form's brief as a text
//! area (decision 35), reached by the keys a user presses.

use super::super::gate::{form as edit_form, gate, open_form as open_edit, tap};
use super::super::goal_form::{app_with_cache, open_form as open_goal, roster};
use super::super::*;
use crate::run_edit::EditField;
use crate::theme::{Palette, Role, role};
use crate::ui::audit;
use proto::{PlanEdit, RunRequest};

fn draw(app: &App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    audit::draw(app, w, h)
}

fn muted() -> ratatui::style::Color {
    role(Role::Muted, Palette::PLAIN).fg.expect("a colour")
}

const PLACEHOLDER: &str = "what should the run achieve?";

#[test]
fn the_goal_field_shows_its_placeholder_while_empty() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = app_with_cache(roster());
        app.set_terminal_size(w, h);
        open_goal(&mut app);
        let buffer = draw(&app, w, h);
        let found = audit::find(&buffer, PLACEHOLDER);
        let &(x, y) = found
            .first()
            .unwrap_or_else(|| panic!("{w}x{h}:\n{}", audit::rows(&buffer).join("\n")));
        // On the goal's first row, after its label; muted past the cursor's cell.
        assert!(audit::rows(&buffer)[usize::from(y)].contains("goal"));
        assert_eq!(buffer[(x + 1, y)].fg, muted(), "{w}x{h}");

        assert!(press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE).is_empty());
        let buffer = draw(&app, w, h);
        assert!(
            audit::find(&buffer, PLACEHOLDER).is_empty(),
            "gone once typed"
        );
        assert!(!audit::find(&buffer, "goal              x").is_empty());
    }
    // In ASCII too.
    let mut app = app_with_cache(roster());
    app.settings.badges.ascii = true;
    app.set_terminal_size(80, 24);
    open_goal(&mut app);
    let buffer = draw(&app, 80, 24);
    assert!(!audit::find(&buffer, PLACEHOLDER).is_empty());
}

/// The rows of the brief: the label's row and the text area's three rows under it, in
/// the value column.
fn brief_rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    let rows = audit::rows(buffer);
    let at = rows
        .iter()
        .position(|r| r.contains("brief      "))
        .unwrap_or_else(|| panic!("no brief row:\n{}", rows.join("\n")));
    let byte = rows[at].find("brief").unwrap();
    // The label column is 11 wide (`ui::dialog::LABEL_WIDTH`).
    let column = rows[at][..byte].chars().count() + 11;
    rows[at..at + 4]
        .iter()
        .map(|r| {
            // The value column: the dialog's 60 interior columns less the label's 13.
            let cells: String = r.chars().skip(column).take(47).collect();
            cells.trim_end().to_string()
        })
        .collect()
}

#[test]
fn the_edit_brief_is_four_rows_and_sends_newlines() {
    let mut app = gate();
    open_edit(&mut app, "t1");
    super::super::gate::focus(&mut app, EditField::Brief);
    let buffer = draw(&app, 80, 24);
    assert_eq!(brief_rows(&buffer), vec!["Line one", "Line two", "", ""]);
    assert!(audit::find(&buffer, "↵").is_empty(), "no newline mark");

    // Ctrl-J is a newline; three more lines fill and scroll the four rows.
    for c in ['x', 'y', 'z'] {
        assert!(press(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL).is_empty());
        assert!(tap(&mut app, KeyCode::Char(c)).is_empty());
    }
    let buffer = draw(&app, 80, 24);
    let rows = audit::rows(&buffer).join("\n");
    assert!(rows.contains("↑ 1 more"), "{rows}");
    assert_eq!(brief_rows(&buffer)[1..], ["x", "y", "z"], "{rows}");

    // Up moves a row inside the brief, not to the field above.
    assert!(tap(&mut app, KeyCode::Up).is_empty());
    assert_eq!(edit_form(&app).focus, EditField::Brief);
    let effects = tap(&mut app, KeyCode::Enter);
    let [
        Effect::Send(proto::ClientMsg::RunTagged {
            request: RunRequest::Edit { edits, .. },
            ..
        }),
    ] = &effects[..]
    else {
        panic!("one edit: {effects:?}");
    };
    let [PlanEdit::AmendTask { brief, .. }] = &edits[..] else {
        panic!("one amend: {edits:?}");
    };
    assert_eq!(brief.as_deref(), Some("Line one\nLine two\nx\ny\nz"));
}

#[test]
fn edit_choices_have_ascii_twins() {
    let mut app = gate();
    app.settings.badges =
        crate::ui::badge::BadgeSet::from_config(&app.settings.badges_config, true);
    open_edit(&mut app, "t1");
    for (w, h) in [(80, 24), (120, 40)] {
        let buffer = draw(&app, w, h);
        let rows = audit::rows(&buffer).join("\n");
        assert_eq!(audit::first_non_ascii(&buffer), None, "{rows}");
        assert!(rows.contains("> runtime    < claude >"), "{rows}");
        assert!(rows.contains("  effort     < medium >"), "{rows}");
        assert!(rows.contains("enter save - tab next"), "{rows}");
    }
    let ascii = Palette {
        ascii: true,
        ..Palette::PLAIN
    };
    let form = edit_form(&app);
    assert_eq!(
        form.value_parts_in(EditField::Size, ascii),
        ("< M >".to_string(), None)
    );
    assert_eq!(
        form.value_parts_in(EditField::Size, Palette::PLAIN),
        ("‹ M ›".to_string(), None)
    );
}
