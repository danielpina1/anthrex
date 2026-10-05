//! M9.0.7.12: the help opens on its context's group and keeps its own keys
//! (decision 34), reached by the keys a user presses.

use super::super::runs::{app_with_runs, open_run_view};
use super::super::*;
use crate::app::help::HelpView;
use crate::app::region::KeyRegion;
use crate::app::screens::Screen;
use crate::app::stats::{StatsScreen, StatsState};
use crate::tree::run_fixtures::{RUN_ID, gate_fixture};
use crate::ui::alerts::fixture::three_runs;
use crate::ui::help::{group_tops, help_groups};
use ratatui::layout::Rect;

fn chord(app: &mut App, c: char) -> Vec<Effect> {
    prefix(app);
    press(app, KeyCode::Char(c), KeyModifiers::NONE)
}

fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

/// The gate fixture, its shell focused, the last frame 80×24.
fn gate_app() -> App {
    let (snap, windows) = gate_fixture();
    let mut app = app_with_runs(windows, snap);
    app.set_body_area(Rect::new(0, 0, 80, 23));
    app
}

fn scroll(app: &App) -> u16 {
    match &app.modal {
        Some(Modal::Help(HelpView { scroll, .. })) => *scroll,
        other => panic!("the help is not open: {other:?}"),
    }
}

/// The group names drawn inside the help's box on a `w`×`h` terminal, top to bottom.
fn drawn_headers(app: &App, w: u16, h: u16) -> Vec<String> {
    let names: Vec<&str> = help_groups("C-b").iter().map(|g| g.name).collect();
    let buffer = crate::ui::audit::draw(app, w, h);
    // Inside the 64-wide centred box, past its border and padding.
    let left = usize::from((w - 64) / 2 + 2);
    crate::ui::audit::rows(&buffer)
        .iter()
        .map(|row| {
            row.chars()
                .skip(left)
                .take(60)
                .collect::<String>()
                .trim()
                .to_string()
        })
        .filter(|text| names.contains(&text.as_str()))
        .collect()
}

/// At 80×24 and 120×40 (fix round 1: at every size), the context's group is the first
/// group drawn.
fn opens_on(mut app: App, group: &str, how: &str) {
    chord(&mut app, '?');
    assert!(
        matches!(app.modal, Some(Modal::Help(_))),
        "{how}: the help opened"
    );
    for (w, h) in [(80, 24), (120, 40)] {
        let headers = drawn_headers(&app, w, h);
        let first = headers.first().map(String::as_str);
        assert_eq!(first, Some(group), "{how} at {w}x{h}: {headers:?}");
    }
}

/// One assertion per row of decision 34's mapping.
#[test]
fn help_opens_on_the_contexts_group() {
    opens_on(gate_app(), "global", "the pane");
    opens_on(
        with(gate_app(), |a| {
            chord(a, 't');
        }),
        "sidebar",
        "the sidebar tree",
    );
    opens_on(
        with(gate_app(), |a| {
            chord(a, 'T');
        }),
        "sidebar",
        "the project overview",
    );
    opens_on(
        with(gate_app(), |a| open_run_view(a, RUN_ID)),
        "run view",
        "the run view",
    );
    opens_on(
        with(gate_app(), |a| {
            open_run_view(a, RUN_ID);
            tap(a, KeyCode::Char('p'));
            assert!(a.plan_review.is_some());
        }),
        "plan review",
        "the plan review",
    );
    // Final fix wave FW-76: the document gate screen's own group.
    opens_on(
        with(gate_app(), |a| {
            let run = &mut a.runs.runs[0];
            run.state = proto::RunState::AwaitingApproval;
            run.design = proto::DesignMode::Full;
            run.doc_gate =
                crate::ui::doc_gate::tests::design_run(proto::DocGateKind::Spec).doc_gate;
            a.open_doc_gate(RUN_ID);
            assert!(matches!(a.screen, Some(Screen::DocGate(_))));
        }),
        "document gate",
        "the document gate screen",
    );
    opens_on(
        with(three_runs(), |a| {
            chord(a, 'a');
            assert_eq!(a.key_region(), KeyRegion::Alerts);
        }),
        "alerts",
        "the Alerts view",
    );
    opens_on(
        with(gate_app(), |a| {
            chord(a, 'm');
            assert!(a.conversation.is_open());
        }),
        "conversation",
        "the conversation view",
    );
    opens_on(
        with(gate_app(), |a| {
            chord(a, 'P');
            assert!(matches!(a.screen, Some(Screen::Profile(_))));
        }),
        "profile",
        "the Profile screen",
    );
    opens_on(
        with(gate_app(), |a| {
            chord(a, 'S');
            assert!(matches!(a.screen, Some(Screen::Settings(_))));
        }),
        "settings",
        "the Settings screen",
    );
    opens_on(
        with(gate_app(), |a| {
            a.screen = Some(Screen::Stats(Box::new(StatsScreen {
                project: "/r/demo".into(),
                state: StatsState::Ready(Box::new(crate::ui::stats::tests::history())),
                scroll: 0,
                request: 4,
            })));
        }),
        "global",
        "the stats screen",
    );
}

fn with(mut app: App, change: impl FnOnce(&mut App)) -> App {
    change(&mut app);
    app
}

/// Decision 34's keys: `j` a line, PgDn a page, `G` nothing, `q` and Esc close, any
/// other key keeps the help open where it was; none reaches the shell.
#[test]
fn j_k_and_pages_scroll_and_esc_closes() {
    let mut app = gate_app();
    chord(&mut app, '?');
    assert_eq!(scroll(&app), 0);
    assert!(tap(&mut app, KeyCode::Char('j')).is_empty());
    assert_eq!(scroll(&app), 1);
    tap(&mut app, KeyCode::Down);
    assert_eq!(scroll(&app), 2);
    tap(&mut app, KeyCode::Char('k'));
    tap(&mut app, KeyCode::Up);
    tap(&mut app, KeyCode::Char('k'));
    assert_eq!(scroll(&app), 0, "k stops at the top");
    // At 80×24 the box has 21 rows of lines; a page is the rows between the marks.
    tap(&mut app, KeyCode::PageDown);
    assert_eq!(scroll(&app), 19);
    tap(&mut app, KeyCode::PageUp);
    assert_eq!(scroll(&app), 0);
    for code in [
        KeyCode::Char('G'),
        KeyCode::Char('x'),
        KeyCode::Char(' '),
        KeyCode::Char('a'),
        KeyCode::Enter,
        KeyCode::Left,
    ] {
        assert!(tap(&mut app, code).is_empty(), "{code:?}");
        assert_eq!(scroll(&app), 0, "{code:?} does nothing");
    }
    // The end: 112 lines from line 92 fill (task M9.6.18 added three, ruling T18-4; the
    // final fix wave's FW-76 thirteen) the 21 rows under the `↑` mark.
    for _ in 0..10 {
        tap(&mut app, KeyCode::PageDown);
    }
    assert_eq!(scroll(&app), 92);
    tap(&mut app, KeyCode::Char('j'));
    assert_eq!(scroll(&app), 92, "j stops where the view stops");
    tap(&mut app, KeyCode::Char('q'));
    assert!(app.modal.is_none(), "q closes");
    chord(&mut app, '?');
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert!(app.modal.is_none(), "Esc closes");
    assert_eq!(app.key_region(), KeyRegion::Pane);
}

#[test]
fn tab_jumps_between_groups() {
    let tops = group_tops(&help_groups("C-b"));
    assert_eq!(tops[..3], [0, 22, 30]);
    let mut app = gate_app();
    chord(&mut app, '?');
    tap(&mut app, KeyCode::Tab);
    assert_eq!(usize::from(scroll(&app)), tops[1], "global › sidebar");
    tap(&mut app, KeyCode::Tab);
    assert_eq!(usize::from(scroll(&app)), tops[2], "sidebar › run view");
    tap(&mut app, KeyCode::BackTab);
    assert_eq!(usize::from(scroll(&app)), tops[1], "back to sidebar");
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::BackTab);
    assert_eq!(
        usize::from(scroll(&app)),
        tops[1],
        "mid-group: its own header"
    );
    tap(&mut app, KeyCode::BackTab);
    tap(&mut app, KeyCode::BackTab);
    assert_eq!(scroll(&app), 0, "BackTab stops at the first group");
    // From mid-group, Tab goes to the next group's header.
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Tab);
    assert_eq!(usize::from(scroll(&app)), tops[1]);
    // Past the last top the view cannot scroll: Tab stops at the end.
    for _ in 0..12 {
        tap(&mut app, KeyCode::Tab);
    }
    assert_eq!(scroll(&app), 92);
}

/// Task 6's ruling: `C-b ?` works over the Alerts view, and closing the help gives the
/// view back with its selection.
#[test]
fn the_help_over_the_alerts_view_gives_it_back() {
    let mut app = three_runs();
    chord(&mut app, 'a');
    tap(&mut app, KeyCode::Char('j'));
    let selected = app.alerts_focus.as_ref().and_then(|f| f.selected.clone());
    assert!(selected.is_some());
    chord(&mut app, '?');
    assert_eq!(app.key_region(), KeyRegion::Dialog);
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Esc);
    assert!(app.modal.is_none());
    assert_eq!(app.key_region(), KeyRegion::Alerts);
    assert_eq!(
        app.alerts_focus.as_ref().and_then(|f| f.selected.clone()),
        selected
    );
}

/// Fix round 1: opened on a group whose name lies past the last page (`settings` at
/// 120×40), the view holds it at the top: `j` and PgDn stop there, `k` leaves it, and
/// the rows under the last group are blank.
#[test]
fn an_opened_group_past_the_end_holds_its_place() {
    let mut app = gate_app();
    app.set_body_area(Rect::new(0, 0, 120, 39));
    chord(&mut app, 'S');
    chord(&mut app, '?');
    let settings = group_tops(&help_groups("C-b"))[9];
    assert_eq!(usize::from(scroll(&app)), settings);
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::PageDown);
    assert_eq!(usize::from(scroll(&app)), settings, "j and PgDn stop there");
    tap(&mut app, KeyCode::Char('k'));
    assert_eq!(usize::from(scroll(&app)), settings - 1);
    tap(&mut app, KeyCode::Char('j'));
    assert_eq!(drawn_headers(&app, 120, 40), ["settings"]);
    let rows = crate::ui::audit::rows(&crate::ui::audit::draw(&app, 120, 40));
    assert!(rows[2].contains("│ settings "), "{rows:#?}");
    assert!(rows[10].contains("esc          back (asks before discarding changes)"));
    assert_eq!(
        rows[11]
            .chars()
            .skip(29)
            .take(62)
            .collect::<String>()
            .trim(),
        ""
    );
    // Opened on `global`, the end is the last page's, as before.
    let mut app = gate_app();
    chord(&mut app, '?');
    for _ in 0..12 {
        tap(&mut app, KeyCode::Tab);
    }
    assert_eq!(scroll(&app), 92);
}

/// Minor 1: Shift+Tab goes back a group, whether it arrives as BackTab or as Tab with
/// Shift held (`app/action_forms.rs`).
#[test]
fn shift_tab_goes_back_a_group() {
    let tops = group_tops(&help_groups("C-b"));
    for back in [
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
    ] {
        let mut app = gate_app();
        chord(&mut app, '?');
        tap(&mut app, KeyCode::Tab);
        tap(&mut app, KeyCode::Tab);
        assert_eq!(usize::from(scroll(&app)), tops[2]);
        app.on_key(back);
        assert_eq!(usize::from(scroll(&app)), tops[1], "{back:?}");
    }
}

/// Minor 5: closing the help over the Profile screen, the Settings screen and the plan
/// review gives each back as it was, with the keys.
#[test]
fn closing_the_help_gives_back_the_screens_and_the_review() {
    let mut app = gate_app();
    chord(&mut app, 'P');
    tap(&mut app, KeyCode::Tab);
    let before = app.screen.clone();
    chord(&mut app, '?');
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Esc);
    assert!(app.modal.is_none());
    assert_eq!(app.screen, before, "the Profile screen, on its Profile tab");
    assert_eq!(app.key_region(), KeyRegion::Screen);

    let mut app = gate_app();
    chord(&mut app, 'S');
    let before = app.screen.clone();
    assert!(matches!(before, Some(Screen::Settings(_))));
    chord(&mut app, '?');
    tap(&mut app, KeyCode::Char('q'));
    assert_eq!(app.screen, before);
    assert_eq!(app.key_region(), KeyRegion::Screen);

    let mut app = gate_app();
    open_run_view(&mut app, RUN_ID);
    tap(&mut app, KeyCode::Char('p'));
    tap(&mut app, KeyCode::Char('j'));
    let before = app.plan_review.clone();
    assert!(before.is_some());
    chord(&mut app, '?');
    tap(&mut app, KeyCode::PageDown);
    tap(&mut app, KeyCode::Esc);
    assert_eq!(app.plan_review, before, "the review, its selection kept");
    assert_eq!(app.key_region(), KeyRegion::Review);
}
