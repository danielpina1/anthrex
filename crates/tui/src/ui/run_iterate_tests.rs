//! Milestone 9.3 task 10a: the iterate dialog drawn (decisions 7, 32 and 33): large from
//! 60×16 with its exact title, prompt and footer, compact below it, and the hostile-text
//! rule. Every frame is drawn whole through `audit::draw`.

use super::*;
use crate::app::{App, Modal};
use crate::run_iterate::IterateForm;
use crate::settings::UiSettings;
use crate::ui::audit;
use crate::ui::goal_editor::tests::framed;

pub(crate) const RUN: &str = "r-20261001-3f9a";

/// An app over an empty session with `form` open, in ASCII or not.
fn app_with(form: IterateForm, ascii: bool) -> App {
    let mut app = App::new(vec![], "/tmp".into(), UiSettings::default());
    app.settings.badges.ascii = ascii;
    app.modal = Some(Modal::Iterate(form));
    app
}

fn form() -> IterateForm {
    IterateForm::new(RUN.into(), 2)
}

/// The cells of `rect` in `buffer`, row by row, trailing spaces trimmed.
pub(crate) fn cells(buffer: &ratatui::buffer::Buffer, rect: Rect) -> Vec<String> {
    (rect.y..rect.bottom())
        .map(|y| {
            let row: String = (rect.x..rect.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            row.trim_end().to_string()
        })
        .collect()
}

/// The interior of the empty dialog with `text_rows` text rows.
fn empty_interior(text_rows: usize, ascii: bool) -> Vec<String> {
    let mut rows = vec!["what should change or be added?".to_string()];
    rows.resize(text_rows + 1, String::new());
    let dot = if ascii { "-" } else { "·" };
    rows.push(format!("ln 1, col 1 {dot} 0 / 16,384"));
    rows.push("^S start  ^K cut  ^U paste  Esc cancel".into());
    rows
}

#[test]
fn the_iterate_dialog_renders_at_80x24_and_120x40() {
    for ascii in [false, true] {
        for (w, h, width, text_rows) in [(80, 24, 76, 17), (120, 40, 116, 33)] {
            let app = app_with(form(), ascii);
            assert_eq!(
                text_view(&form(), w, h),
                EditorView {
                    width: width - 4,
                    rows: text_rows
                },
                "{w}x{h}"
            );
            let title = match ascii {
                false => "iterate run 3f9a · round 2",
                true => "iterate run 3f9a - round 2",
            };
            let want = framed(
                title,
                usize::from(width),
                &empty_interior(usize::from(text_rows), ascii),
                ascii,
            );
            let buffer = audit::draw(&app, w, h);
            let rect = dialog_rect(Rect::new(0, 0, w, h));
            assert_eq!((rect.x, rect.y, rect.height), (2, 0, h - 2));
            assert_eq!(cells(&buffer, rect), want, "{w}x{h} ascii {ascii}");
            // The prompt is muted; the status bar stays under the dialog.
            assert_eq!(
                buffer[(4, 1)].fg,
                crate::theme::fg(crate::theme::Role::Muted)
            );
            let bar = cells(&buffer, Rect::new(0, h - 1, w, 1));
            assert_eq!(bar, vec![" DIALOG  esc back".to_string()]);
            assert_eq!(audit::accented_frames(&buffer, app.palette()), 1);
            assert_eq!(audit::stray_accent(&buffer, app.palette()), None);
            if ascii {
                assert_eq!(audit::first_non_ascii(&buffer), None, "{w}x{h}");
            }
        }
    }
}

/// Below 60×16 the compact dialog (decision 7's fallback), and the status bar says to
/// widen the terminal; an error row and `starting…` while the request waits.
#[test]
fn below_60_by_16_the_iterate_dialog_is_compact() {
    let mut form = form();
    form.text = crate::text_area::TextArea::editor("more");
    let app = app_with(form.clone(), true);
    assert_eq!(
        text_view(&form, 59, 24),
        EditorView {
            width: 55,
            rows: GOAL_ROWS
        }
    );
    let buffer = audit::draw(&app, 59, 24);
    let rows = [
        "what should change or be added?",
        "more",
        "",
        "",
        "",
        "ln 1, col 5 - 4 / 16,384",
        "^S start - esc cancel",
    ]
    .map(String::from);
    let want = framed("iterate run 3f9a - round 2", 59, &rows, true);
    let rect = kit::dialog_area(Rect::new(0, 0, 59, 24), 7);
    assert_eq!(cells(&buffer, rect), want);
    let bar = cells(&buffer, Rect::new(0, 23, 59, 1));
    assert_eq!(
        bar,
        vec![" DIALOG  widen the terminal for the editor  esc back".to_string()]
    );
    assert_eq!(audit::accented_frames(&buffer, app.palette()), 1);
    assert_eq!(audit::first_non_ascii(&buffer), None);

    let mut waiting = form;
    waiting.error = Some("run 3f9a is running; iterate it when it completes".into());
    waiting.submitting = true;
    let buffer = audit::draw(&app_with(waiting, true), 59, 24);
    let rows = [
        "what should change or be added?",
        "more",
        "",
        "",
        "",
        "run 3f9a is running; iterate it when it completes",
        "starting...",
        "esc close",
    ]
    .map(String::from);
    let want = framed("iterate run 3f9a - round 2", 59, &rows, true);
    let rect = kit::dialog_area(Rect::new(0, 0, 59, 24), 8);
    assert_eq!(cells(&buffer, rect), want);
}

/// The error row takes a text row, so the dialog keeps its height; and the text keeps
/// at least one row (the goal dialog's review m2).
#[test]
fn the_error_row_takes_a_text_row() {
    let mut form = form();
    form.error = Some("refused".into());
    assert_eq!(text_view(&form, 80, 24).rows, 16);
    assert_eq!(text_view(&form, 60, 16).rows, 8);
    assert_eq!(text_view(&form, 60, 4).rows, GOAL_ROWS);
    let tiny = dialog_rect(Rect::new(0, 0, 60, 16));
    assert_eq!(tiny.height, 14);
}

/// Decision 33: the run id's carriers never reach the title, and the short id is the
/// cleaned id's last four characters; the daemon's refusal is drawn cleaned too.
/// Mutants: `title`'s `one_line` removed (the short id is cut before cleaning: `cd`
/// shows), and the error row's `one_line` removed (the carriers are drawn), each red.
#[test]
fn iterate_dialog_text_is_sanitised() {
    let mut form = IterateForm::new("run-ab\u{200D}c\u{202E}d".into(), 3);
    form.error = Some("run x\u{200D}y\u{202E}z refused".into());
    let app = app_with(form, false);
    let buffer = audit::draw(&app, 80, 24);
    let rows = cells(&buffer, dialog_rect(Rect::new(0, 0, 80, 24)));
    assert!(
        rows[0].starts_with("┌ iterate run abcd · round 3 ─"),
        "{rows:#?}"
    );
    assert_eq!(
        rows[18].trim_end_matches('│').trim_end(),
        "│ run xyz refused"
    );
    for row in &rows {
        assert_eq!(crate::safe_text::tests::first_hostile(row), None, "{row}");
    }
}
