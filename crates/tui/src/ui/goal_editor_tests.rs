//! Milestone 9.3 task 9b: the goal dialog drawn (decisions 7, 25 and 33): large from
//! 60×16, compact below it with the status bar's `widen the terminal for the editor`,
//! the footer's drops, the orchestrator row's texts and the hostile-text rule. Every
//! frame is drawn whole through `audit::draw`.

use super::*;
use crate::app::{App, Modal};
use crate::settings::UiSettings;
use crate::text_area::TextArea;
use crate::theme::Palette;
use crate::ui::audit;
use proto::{IdleOrchestrator, RunState, Runtime};

/// An app over an empty session with `form` open, in ASCII or not.
fn app_with(form: GoalForm, ascii: bool) -> App {
    let mut app = App::new(vec![], "/tmp".into(), UiSettings::default());
    app.settings.badges.ascii = ascii;
    app.modal = Some(Modal::StartGoal(Box::new(form)));
    app
}

fn form() -> GoalForm {
    GoalForm::new("/p/a".into())
}

pub(crate) fn idle(fresh: bool) -> IdleOrchestrator {
    IdleOrchestrator {
        chain: "o-3f9a".into(),
        project: "/p/a".into(),
        after_run: "r-20261001-3f9a".into(),
        outcome: RunState::Accepted,
        runtime: Runtime::Claude,
        model: "claude-opus-5-5".into(),
        window_id: (!fresh).then_some(7),
        fresh,
        runs: 1,
    }
}

/// The cells of `rect` in `buffer`, row by row, trailing spaces trimmed.
fn cells(buffer: &ratatui::buffer::Buffer, rect: Rect) -> Vec<String> {
    (rect.y..rect.bottom())
        .map(|y| {
            let row: String = (rect.x..rect.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            row.trim_end().to_string()
        })
        .collect()
}

/// The large dialog's rows in a `w`×`h` frame.
fn large_rows(app: &App, w: u16, h: u16) -> Vec<String> {
    let buffer = audit::draw(app, w, h);
    cells(&buffer, dialog_rect(Rect::new(0, 0, w, h)))
}

/// A `width`-column dialog frame titled `title` around `rows` (each padded by one
/// column on both sides), in the palette's border set.
pub(crate) fn framed(title: &str, width: usize, rows: &[String], ascii: bool) -> Vec<String> {
    let (tl, tr, bl, br, h, v) = if ascii {
        ("+", "+", "+", "+", "-", "|")
    } else {
        ("┌", "┐", "└", "┘", "─", "│")
    };
    let top = format!("{tl} {title} ");
    let mut out = vec![format!(
        "{top}{}{tr}",
        h.repeat(width - 1 - top.chars().count())
    )];
    for row in rows {
        out.push(format!("{v} {row:<w$} {v}", w = width - 4));
    }
    out.push(format!("{bl}{}{br}", h.repeat(width - 2)));
    out
}

/// The empty form's interior with `text_rows` text rows.
fn empty_interior(text_rows: usize, ascii: bool) -> Vec<String> {
    let choice = |v: &str| {
        if ascii {
            format!("< {v} >")
        } else {
            format!("‹ {v} ›")
        }
    };
    let mut rows = vec!["what should the run achieve?".to_string()];
    rows.resize(text_rows + 1, String::new());
    for (label, value) in [
        ("model", "role table"),
        ("effort", "role table"),
        ("orchestrator", "new"),
        ("delivery", "configured"),
        ("design", "configured"),
        ("trust", "off"),
        ("approve at once", "off"),
        ("unconfined checks", "off"),
    ] {
        rows.push(format!("  {label:<18}{}", choice(value)));
    }
    let dot = if ascii { "-" } else { "·" };
    rows.push(format!("ln 1, col 1 {dot} 0 / 16,384"));
    rows.push("^S start  Tab options  ^K cut  ^U paste  Esc cancel".into());
    rows
}

/// Milestone 9.6 task 18 (changed expectation): the design row takes a text row, 9 at
/// 80×24 and 25 at 120×40.
#[test]
fn the_dialog_is_large_at_80x24_and_120x40() {
    for ascii in [false, true] {
        for (w, h, width, text_rows) in [(80, 24, 76, 9), (120, 40, 116, 25)] {
            let app = app_with(form(), ascii);
            assert_eq!(
                text_view(&form(), w, h),
                EditorView {
                    width: width - 4,
                    rows: text_rows
                },
                "{w}x{h}"
            );
            let want = framed(
                "start a goal in /p/a",
                usize::from(width),
                &empty_interior(usize::from(text_rows), ascii),
                ascii,
            );
            assert_eq!(large_rows(&app, w, h), want, "{w}x{h} ascii {ascii}");
            let buffer = audit::draw(&app, w, h);
            // `rows − 2` high: one margin row, then the status bar.
            let rect = dialog_rect(Rect::new(0, 0, w, h));
            assert_eq!((rect.x, rect.y, rect.height), (2, 0, h - 2));
            assert_eq!(audit::accented_frames(&buffer, app.palette()), 1);
            assert_eq!(audit::stray_accent(&buffer, app.palette()), None);
            if ascii {
                assert_eq!(audit::first_non_ascii(&buffer), None, "{w}x{h}");
            }
        }
    }
}

/// Today's compact rows (milestone 9.0.6's drawing, `GOAL_ROWS` text rows) with the
/// orchestrator row decision 25 adds, in ASCII.
fn compact_interior() -> Vec<String> {
    let mut rows = vec!["> goal              what should the run achieve?".to_string()];
    rows.resize(6, String::new());
    for (label, value) in [
        ("model", "role table"),
        ("effort", "role table"),
        ("orchestrator", "new"),
        ("delivery", "configured"),
        ("design", "configured"),
        ("trust", "off"),
        ("approve at once", "off"),
        ("unconfined checks", "off"),
    ] {
        rows.push(format!("  {label:<18}< {value} >"));
    }
    rows.push(String::new());
    rows.push("^S start - tab next - esc cancel".into());
    rows
}

/// Milestone 9.6 task 18 (changed expectation): the design row makes the compact
/// dialog 18 rows high.
#[test]
fn below_60_by_16_the_dialog_keeps_its_compact_layout() {
    let app = app_with(form(), true);
    // 59x24: 59 wide (the 64-column dialog cut to the terminal), 18 high, centred.
    let buffer = audit::draw(&app, 59, 24);
    let want = framed("start a goal in /p/a", 59, &compact_interior(), true);
    assert_eq!(cells(&buffer, Rect::new(0, 3, 59, 18)), want);
    assert_eq!(audit::accented_frames(&buffer, app.palette()), 1);
    // 80x15: the 64-column dialog, cut to the terminal's 15 rows.
    let buffer = audit::draw(&app, 80, 15);
    let mut want = framed("start a goal in /p/a", 64, &compact_interior()[..13], true);
    let last = want.len() - 1;
    want[last] = format!("+{}+", "-".repeat(62));
    assert_eq!(cells(&buffer, Rect::new(8, 0, 64, 15)), want);
    // The keys move by the compact text area.
    assert_eq!(
        text_view(&form(), 59, 24),
        EditorView {
            width: goal_width(59),
            rows: GOAL_ROWS
        }
    );
    assert!(!is_large(59, 24) && !is_large(80, 15) && is_large(60, 16));
}

#[test]
fn the_status_bar_says_widen_the_terminal() {
    let app = app_with(form(), false);
    let bar = |w, h| audit::rows(&audit::draw(&app, w, h))[usize::from(h) - 1].clone();
    assert_eq!(
        bar(59, 24),
        " DIALOG  widen the terminal for the editor  esc back"
    );
    // The compact dialog is 18 rows tall (milestone 9.6's design row; changed
    // expectation) and, as before 9.3, drawn over the whole frame: from 19 rows the bar
    // shows under it.
    assert_eq!(bar(80, 19), " DIALOG  esc back");
    assert_eq!(
        bar(59, 19),
        " DIALOG  widen the terminal for the editor  esc back"
    );
    assert_eq!(bar(80, 24), " DIALOG  esc back");
    assert_eq!(bar(60, 16), " DIALOG  esc back");
}

/// Review m1: the note goes before `esc back` does; `esc` never drops.
#[test]
fn the_widen_note_never_costs_esc() {
    let app = app_with(form(), false);
    let bar = |w: u16| audit::rows(&audit::draw(&app, w, 24))[23].clone();
    for w in 44..=51 {
        assert_eq!(bar(w), " DIALOG  esc back", "{w} columns");
    }
    assert_eq!(
        bar(52),
        " DIALOG  widen the terminal for the editor  esc back"
    );
}

#[test]
fn the_footer_drops_entries_from_the_right() {
    let text = |width| footer(&FOOTER, width, Palette::PLAIN).to_string();
    assert_eq!(
        text(51),
        "^S start  Tab options  ^K cut  ^U paste  Esc cancel"
    );
    assert_eq!(text(50), "^S start  Tab options  ^K cut  ^U paste");
    assert_eq!(text(30), "^S start  Tab options  ^K cut");
    assert_eq!(text(8), "^S start");
    assert_eq!(text(7), "");
}

/// The large dialog's orchestrator row at 120x40.
fn orchestrator_row(form: GoalForm) -> String {
    let rows = large_rows(&app_with(form, false), 120, 40);
    rows.into_iter()
        .find(|r| r.contains("orchestrator "))
        .expect("an orchestrator row")
}

#[test]
fn the_orchestrator_row_reads_each_state() {
    let mut f = form();
    f.set_chains(Some(idle(false)), None);
    assert!(f.continuing, "continue is the default");
    let row = orchestrator_row(f.clone());
    assert!(
        row.contains("  orchestrator      ‹ continue o-3f9a (after 3f9a) › "),
        "{row}"
    );
    f.continuing = false;
    assert!(orchestrator_row(f.clone()).contains("  orchestrator      ‹ new › "));
    let mut f = form();
    f.set_chains(Some(idle(true)), None);
    assert!(
        orchestrator_row(f).contains("  orchestrator      ‹ continue o-3f9a (fresh session) ›")
    );
    let mut f = form();
    f.set_chains(None, Some(("o-3f9a".into(), "77b2".into())));
    assert!(orchestrator_row(f).contains(
        "  orchestrator      o-3f9a is working on run 77b2; this goal gets a new orchestrator"
    ));
}

/// Decision 33: visible carriers in the project path, the chain id and the goal are
/// drawn without them. Mutant: the active chain's `one_line` in `option_lines` removed
/// (its row then carries them; `theme::choice` cleans the continue row on its own).
#[test]
fn goal_dialog_text_is_sanitised() {
    let carriers = "x\u{200D}y\u{202E}z";
    let mut f = GoalForm::new(format!("/p/{carriers}").into());
    f.goal = TextArea::unclean(&format!("go {carriers}"));
    f.set_chains(None, Some((format!("o-{carriers}"), "3f9a".into())));
    let rows = large_rows(&app_with(f.clone(), false), 120, 40);
    assert!(
        rows[0].starts_with("┌ start a goal in /p/xyz ─"),
        "{}",
        rows[0]
    );
    assert_eq!(rows[1], format!("│ {:<112} │", "go xyz"));
    let busy =
        "  orchestrator      o-xyz is working on run 3f9a; this goal gets a new orchestrator";
    assert!(rows.contains(&format!("│ {busy:<112} │")), "{rows:?}");
    f.set_chains(
        Some(IdleOrchestrator {
            chain: format!("o-{carriers}"),
            ..idle(false)
        }),
        None,
    );
    let rows = large_rows(&app_with(f, false), 120, 40);
    let choice = "  orchestrator      ‹ continue o-xyz (after 3f9a) ›";
    assert!(rows.contains(&format!("│ {choice:<112} │")), "{rows:?}");
    for row in rows {
        assert_eq!(crate::safe_text::tests::first_hostile(&row), None, "{row}");
    }
}

/// Final fix wave C-m8: a short id is cleaned before it is cut, as the idle row cleans
/// it, so a carrier among a run id's last four characters never shortens the drawn
/// `<h4>`: the continue row's `after` and the active chain's run.
#[test]
fn short_ids_are_cleaned_before_they_are_cut() {
    let mut f = form();
    f.set_chains(
        Some(IdleOrchestrator {
            after_run: "r-20261001-3f\u{200D}9\u{202E}a".into(),
            ..idle(false)
        }),
        None,
    );
    let rows = large_rows(&app_with(f, false), 120, 40);
    let choice = "  orchestrator      ‹ continue o-3f9a (after 3f9a) ›";
    assert!(rows.contains(&format!("│ {choice:<112} │")), "{rows:?}");

    let mut app = app_with(form(), false);
    let mut run = crate::tree::run_fixtures::run(
        "r-20261003-77\u{200D}b\u{202E}2",
        "/p/a",
        RunState::Running,
    );
    run.chain = Some("o-3f9a".into());
    let snap = crate::tree::run_fixtures::snapshot(100, vec![run]);
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    let rows = large_rows(&app, 120, 40);
    let busy =
        "  orchestrator      o-3f9a is working on run 77b2; this goal gets a new orchestrator";
    assert!(rows.contains(&format!("│ {busy:<112} │")), "{rows:?}");
}

/// Review m2: at the 16-row minimum, with an error showing, the large editor still
/// keeps one text row (the options take the rest).
#[test]
fn the_large_text_area_keeps_a_row() {
    let mut f = form();
    f.error = Some("type a goal first".into());
    f.goal = TextArea::editor("hello");
    assert_eq!(text_view(&f, 60, 16), EditorView { width: 52, rows: 1 });
    let rows = large_rows(&app_with(f, false), 60, 16);
    assert!(rows[1].starts_with("│ hello"), "{rows:?}");
    // The kit draws no row for none (task 9a's re-review): why the view keeps one.
    assert!(kit::editor(&TextArea::editor("hello"), 0, 40, true).is_empty());
}

/// Final fix wave C-m1 (carried N1): at 60×16 with an error the body is one row too
/// tall, so the position row goes and the footer stays: `^S start` is still the hint
/// for retrying. One row taller, both are drawn. Milestone 9.8 (changed expectation):
/// the custom model's text row is gone (the picker types it), so at 60×16 only the
/// position row goes, the blank row above the options stays, and at 60×17 both are
/// drawn (milestone 9.6's two-rows-too-tall case no longer arises).
#[test]
fn the_footer_outlasts_the_position_row() {
    let mut f = form();
    f.error = Some("type a goal first".into());
    f.goal = TextArea::editor("hello");
    let footer = "^S start  Tab options  ^K cut  ^U paste  Esc cancel";
    let position = "ln 1, col 6";
    for ascii in [false, true] {
        let rows = large_rows(&app_with(f.clone(), ascii), 60, 16);
        let interior = &rows[1..rows.len() - 1];
        assert_eq!(interior.len(), 12);
        assert!(interior[11].contains(footer), "{rows:?}");
        assert!(interior[10].contains("type a goal first"), "{rows:?}");
        assert!(interior[2].contains("model"), "the blank row: {rows:?}");
        assert!(!rows.iter().any(|r| r.contains(position)), "{rows:?}");
        let rows = large_rows(&app_with(f.clone(), ascii), 60, 17);
        let interior = &rows[1..rows.len() - 1];
        assert!(interior[12].contains(footer), "{rows:?}");
        assert!(interior[11].contains(position), "{rows:?}");
    }
}

/// The row of a `w`×`h` frame holding `needle`, trimmed, from the whole buffer.
fn row_with(app: &App, w: u16, h: u16, needle: &str) -> String {
    let buffer = audit::draw(app, w, h);
    let rows = cells(&buffer, Rect::new(0, 0, w, h));
    let row = rows.into_iter().find(|r| r.contains(needle));
    row.unwrap_or_else(|| panic!("no row with {needle:?}"))
}

/// Final fix wave C-m6: a continue choice too long for the row loses whole words, then
/// takes the ellipsis, and keeps its closing chevron: at 60×16 (the large dialog's
/// 32-column value) and at 50×24 (the compact one's 26), in both palettes.
#[test]
fn a_long_continue_choice_is_cut_at_a_word() {
    let mut f = form();
    f.set_chains(Some(idle(true)), None);
    for (ascii, open, close, dots) in [(false, "‹", "›", "…"), (true, "<", ">", "...")] {
        let app = app_with(f.clone(), ascii);
        let row = row_with(&app, 60, 16, "orchestrator ");
        let want = format!("  orchestrator      {open} continue o-3f9a (fresh{dots} {close}");
        assert!(row.contains(&format!("{want} ")), "{row:?}");
        let row = row_with(&app, 50, 24, "orchestrator ");
        let want = format!("  orchestrator      {open} continue o-3f9a{dots} {close}");
        assert!(row.contains(&want), "{row:?}");
    }
    // What fits is drawn whole.
    let mut f = form();
    f.set_chains(Some(idle(false)), None);
    let row = row_with(&app_with(f, false), 60, 16, "orchestrator ");
    assert!(
        row.contains("  orchestrator      ‹ continue o-3f9a (after 3f9a) ›"),
        "{row:?}"
    );
}

/// Final fix wave C-m7: while a goal is sent, both dialogs say triage can take minutes
/// for a new orchestrator only; a continued goal is not triaged (decision 22).
#[test]
fn only_a_new_goal_waits_on_triage() {
    let mut f = form();
    f.goal = TextArea::editor("go");
    f.set_chains(Some(idle(false)), None);
    f.submitting = true;
    let mut new = f.clone();
    new.continuing = false;
    for (w, h) in [(120, 40), (50, 24)] {
        let row = row_with(&app_with(new.clone(), false), w, h, "starting");
        assert!(
            row.contains("starting… triage can take minutes"),
            "{w}x{h}: {row:?}"
        );
        let row = row_with(&app_with(f.clone(), true), w, h, "starting");
        assert!(row.contains("starting..."), "{w}x{h}: {row:?}");
        assert!(!row.contains("triage"), "{w}x{h}: {row:?}");
    }
}

/// Review m6: the continued chain's model, daemon text, is drawn cleaned.
#[test]
fn a_continued_models_carriers_are_drawn_cleaned() {
    let mut f = form();
    f.set_chains(
        Some(IdleOrchestrator {
            model: "opus-x\u{200D}y\u{202E}z".into(),
            ..idle(false)
        }),
        None,
    );
    let rows = large_rows(&app_with(f, false), 120, 40);
    let model = "  model             ‹ Claude · opus-xyz ›";
    assert!(rows.contains(&format!("│ {model:<112} │")), "{rows:?}");
    for row in rows {
        assert_eq!(crate::safe_text::tests::first_hostile(&row), None, "{row}");
    }
}
