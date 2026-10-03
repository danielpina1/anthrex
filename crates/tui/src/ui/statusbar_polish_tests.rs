//! Milestone 9.0.7 task 7: the status bar's badges, the flag's key and the hints that
//! drop by priority (decisions 31–33), with 9.0.6's badge-precedence tests moved here
//! from `statusbar_kit_tests.rs` (task 7 step 0).

use super::kit_tests::{bar, draw, gate_app, prefix, press};
use crate::actions_request::ActionTarget;
use crate::app::{App, Modal, ReviewTarget, TreeInput};
use crate::settings::UiSettings;
use crate::theme::{Role, role};
use crate::tree::alert_fixtures::at;
use crate::tree::orch_fixtures::hold;
use crate::tree::run_fixtures::{RUN_ID, gate_fixture, pty, snapshot, three_task_fixture};
use crossterm::event::{KeyCode, KeyModifiers};
use proto::{DaemonMsg, HoldState, RunReply, RunState, RunsSnapshot, Status, WindowInfo};
use ratatui::buffer::Buffer;

const NONE: KeyModifiers = KeyModifiers::NONE;

fn app_with(snap: RunsSnapshot, windows: Vec<WindowInfo>, w: u16, h: u16) -> App {
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    app.set_terminal_size(w, h);
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    app
}

fn open_menu_on_the_run(app: &mut App) {
    app.open_actions((RUN_ID.into(), ActionTarget::Run), None);
    assert!(matches!(app.modal, Some(Modal::Action(_))));
}

/// The column where `text` starts on the bottom row (every cell one column wide);
/// `None` for an empty `text` (final fix wave, task 7's deferred minor 4).
fn column_of(buffer: &Buffer, width: u16, height: u16, text: &str) -> Option<u16> {
    let row: Vec<String> = (0..width)
        .map(|x| buffer[(x, height - 1)].symbol().to_string())
        .collect();
    let len = text.chars().count();
    if len == 0 {
        return None;
    }
    (0..row.len().saturating_sub(len - 1))
        .find_map(|x| (row[x..x + len].concat() == text).then_some(x as u16))
}

/// The gate run's view, its planning twin's, or a running run with an awaiting hold.
fn run_view_app(state: Option<RunState>, with_hold: bool, w: u16, h: u16) -> App {
    let (mut snap, windows) = gate_fixture();
    if let Some(state) = state {
        snap.runs[0].state = state;
    }
    if with_hold {
        snap.runs[0].holds = vec![hold("epic:e1", HoldState::Awaiting, &["t1"])];
    }
    let mut app = app_with(snap, windows, w, h);
    app.open_run_view(RUN_ID.into());
    assert!(app.run_view.is_some());
    app
}

/// Preflight F10, task 10: ` MENU ` while the action menu is open, above ` PLAN `.
#[test]
fn the_menu_badge_sits_above_plan() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = gate_app(w, h);
        app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate);
        assert!(bar(&app, w, h).starts_with(" PLAN "), "{w}");
        app.open_actions(
            (RUN_ID.into(), crate::actions_request::ActionTarget::Run),
            None,
        );
        assert!(matches!(app.modal, Some(Modal::Action(_))));
        assert!(
            bar(&app, w, h).starts_with(" MENU "),
            "{w}: {}",
            bar(&app, w, h)
        );
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(bar(&app, w, h).starts_with(" PLAN "), "{w}");
    }
}

/// Preflight F10, task 13: ` PROFILE ` while the Profile screen is open, above ` PLAN `
/// and below ` MENU `; its hints are the screen's.
#[test]
fn the_profile_badge_sits_above_plan() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = gate_app(w, h);
        app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate);
        assert!(bar(&app, w, h).starts_with(" PLAN "), "{w}");
        let _ = app.open_profile_on("/r/demo".into(), false);
        let text = bar(&app, w, h);
        assert!(text.starts_with(" PROFILE "), "{w}: {text}");
        assert!(text.contains("d detect"), "{w}: {text}");
        assert!(text.contains("esc back"), "{w}: {text}");
        app.open_actions(
            (RUN_ID.into(), crate::actions_request::ActionTarget::Run),
            None,
        );
        assert!(bar(&app, w, h).starts_with(" MENU "), "{w}");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(bar(&app, w, h).starts_with(" PROFILE "), "{w}");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(bar(&app, w, h).starts_with(" PLAN "), "{w}");
    }
}

/// Preflight F10, task 14: ` SETTINGS ` while the Settings screen is open, above ` PLAN `
/// and below ` MENU `; its hints are the screen's.
#[test]
fn the_settings_badge_sits_above_plan() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = gate_app(w, h);
        prefix(&mut app);
        press(&mut app, KeyCode::Char('S'), KeyModifiers::SHIFT);
        let text = bar(&app, w, h);
        assert!(text.starts_with(" SETTINGS "), "{w}: {text}");
        assert!(text.contains("esc back"), "{w}: {text}");
        app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate);
        assert!(bar(&app, w, h).starts_with(" SETTINGS "), "{w}");
        app.open_actions(
            (RUN_ID.into(), crate::actions_request::ActionTarget::Run),
            None,
        );
        assert!(bar(&app, w, h).starts_with(" MENU "), "{w}");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(bar(&app, w, h).starts_with(" SETTINGS "), "{w}");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(bar(&app, w, h).starts_with(" PLAN "), "{w}");
    }
}

/// Preflight F10, task 15: ` STATS ` while the stats screen is open, above ` PLAN ` and
/// below ` MENU `; its hints are the screen's.
#[test]
fn the_stats_badge_sits_above_plan() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = gate_app(w, h);
        app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate);
        assert!(bar(&app, w, h).starts_with(" PLAN "), "{w}");
        let _ = app.open_stats("/r/demo".into());
        let text = bar(&app, w, h);
        assert!(text.starts_with(" STATS "), "{w}: {text}");
        assert!(text.contains("j/k scroll"), "{w}: {text}");
        assert!(text.contains("esc back"), "{w}: {text}");
        app.open_actions(
            (RUN_ID.into(), crate::actions_request::ActionTarget::Run),
            None,
        );
        assert!(bar(&app, w, h).starts_with(" MENU "), "{w}");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(bar(&app, w, h).starts_with(" STATS "), "{w}");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(bar(&app, w, h).starts_with(" PLAN "), "{w}");
    }
}

/// Decision 31 (followups "From M9.0.6.16"): while the action menu is open the bar
/// reads ` MENU ` and `esc back`, never the hints of the mode underneath; the menu
/// draws its own key line.
#[test]
fn the_menu_shows_only_esc() {
    let (w, h) = (120, 40);
    // Over the sidebar tree.
    let mut app = gate_app(w, h);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('t'), NONE);
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert!(
        bar(&app, w, h).contains("space fold"),
        "{}",
        bar(&app, w, h)
    );
    open_menu_on_the_run(&mut app);
    assert_eq!(bar(&app, w, h), " MENU  esc back");
    // Over the run view, opened by its own `.`.
    let mut app = run_view_app(None, false, w, h);
    press(&mut app, KeyCode::Char('.'), NONE);
    assert!(matches!(app.modal, Some(Modal::Action(_))));
    assert_eq!(bar(&app, w, h), " MENU  esc back");
    // Over the plan review; Esc gives the review its hints back.
    let mut app = gate_app(w, h);
    app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate);
    open_menu_on_the_run(&mut app);
    // Milestone 9.5 decision 42: the review's alert count stays (no key).
    assert_eq!(bar(&app, w, h), " MENU  ⚑ 1  esc back");
    press(&mut app, KeyCode::Esc, NONE);
    let text = bar(&app, w, h);
    assert!(text.starts_with(" PLAN  ⚑ 1  a approve"), "{text:?}");
}

/// Decision 31 and spec §6.6: the sidebar tree, the project overview and the run view
/// wear distinct badges, and typing a filter in any of them wears ` FILTER `.
#[test]
fn badges_tell_tree_overview_and_run_apart() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = gate_app(w, h);
        let starts = |app: &App, badge: &str| {
            let text = bar(app, w, h);
            assert!(text.starts_with(badge), "{w}: {badge:?} in {text:?}");
        };
        prefix(&mut app);
        press(&mut app, KeyCode::Char('t'), NONE);
        starts(&app, " TREE ");
        press(&mut app, KeyCode::Char('/'), NONE);
        starts(&app, " FILTER ");
        press(&mut app, KeyCode::Esc, NONE);
        starts(&app, " TREE ");
        press(&mut app, KeyCode::Esc, NONE);
        assert!(app.tree_input.is_none());

        prefix(&mut app);
        press(&mut app, KeyCode::Char('T'), NONE);
        assert!(app.overview && app.run_view.is_none());
        starts(&app, " OVERVIEW ");
        assert!(!bar(&app, w, h).contains(" TREE "), "{w}");
        press(&mut app, KeyCode::Char('/'), NONE);
        starts(&app, " FILTER ");
        press(&mut app, KeyCode::Esc, NONE);
        starts(&app, " OVERVIEW ");

        // A run opened from the overview.
        let key = crate::tree::NodeKey::Run(RUN_ID.into());
        let rows = crate::tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
        app.tree.select(&rows, key);
        press(&mut app, KeyCode::Char('l'), NONE);
        assert!(app.run_view.is_some());
        starts(&app, " RUN ");
        press(&mut app, KeyCode::Char('/'), NONE);
        starts(&app, " FILTER ");
        press(&mut app, KeyCode::Esc, NONE);
        starts(&app, " RUN ");
        // Back out of the run view: the overview's badge again.
        press(&mut app, KeyCode::Esc, NONE);
        assert!(app.run_view.is_none() && app.overview);
        starts(&app, " OVERVIEW ");
    }
}

/// Decision 32: with the sidebar hidden, `⚑ <n> <prefix> a` follows the badge, the
/// glyph and count in the top alert's role and the key in the accent; it survives
/// every hint drop.
#[test]
fn the_flag_names_its_key_when_the_sidebar_is_hidden() {
    let window = || vec![pty(1, "shell", "/r/demo", Status::Idle)];
    let halted = || {
        snapshot(
            1,
            vec![
                at("a", RunState::Halted, 1),
                at("b", RunState::Halted, 1),
                at("c", RunState::Halted, 1),
            ],
        )
    };
    let (w, h) = (80, 24);
    let mut app = app_with(halted(), window(), w, h);
    assert!(!bar(&app, w, h).contains("C-b a"), "the sidebar shows");
    app.sidebar_visible = false;
    let text = bar(&app, w, h);
    assert!(text.starts_with(" ⚑ 3 C-b a  "), "{text:?}");
    let buffer = draw(&app, w, h);
    let p = app.palette();
    let flag = column_of(&buffer, w, h, "⚑ 3").unwrap();
    let style = crate::theme::alert_style(3, p);
    for x in flag..flag + 3 {
        assert_eq!(buffer[(x, h - 1)].fg, style.fg.unwrap(), "{x}");
    }
    let key = column_of(&buffer, w, h, "C-b a").unwrap();
    for x in key..key + 5 {
        assert_eq!(
            buffer[(x, h - 1)].fg,
            role(Role::Accent, p).fg.unwrap(),
            "{x}"
        );
    }

    // Only P4 alerts: `✓` in `Done`.
    let done = snapshot(
        1,
        vec![
            at("a", RunState::Complete, 1),
            at("b", RunState::Complete, 1),
        ],
    );
    let mut app = app_with(done, window(), w, h);
    app.sidebar_visible = false;
    let text = bar(&app, w, h);
    assert!(text.starts_with(" ✓ 2 C-b a  "), "{text:?}");
    let buffer = draw(&app, w, h);
    let tick = column_of(&buffer, w, h, "✓ 2").unwrap();
    assert_eq!(
        buffer[(tick, h - 1)].fg,
        role(Role::Done, app.palette()).fg.unwrap()
    );

    // No alert, no flag.
    let mut app = app_with(snapshot(1, vec![]), window(), w, h);
    app.sidebar_visible = false;
    let text = bar(&app, w, h);
    assert!(!text.contains("C-b a") && !text.contains('⚑'), "{text:?}");

    // Another prefix names itself; ASCII has its twin.
    let mut app = app_with(halted(), window(), w, h);
    app.sidebar_visible = false;
    app.settings.prefix_label = "C-a".into();
    assert!(
        bar(&app, w, h).starts_with(" ⚑ 3 C-a a  "),
        "{}",
        bar(&app, w, h)
    );
    app.settings.badges.ascii = true;
    let text = bar(&app, w, h);
    assert!(text.is_ascii() && text.contains(" 3 C-a a  "), "{text:?}");

    // It survives every hint drop: the run view at 40 columns keeps it and `esc`.
    let mut app = run_view_app(None, false, 40, h);
    app.sidebar_visible = false;
    let text = bar(&app, 40, h);
    assert!(text.starts_with(" RUN  ⚑ 1 C-b a  "), "{text:?}");
    assert!(text.ends_with("esc back"), "{text:?}");
}

/// Decision 33: every mode of Interfaces "Hint priorities" that the status bar draws
/// keeps its `esc` at 40 columns.
#[test]
fn every_mode_keeps_esc_at_40_columns() {
    let (w, h) = (40, 24);
    let mut modes: Vec<(&str, App)> = Vec::new();

    let mut tree = gate_app(w, h);
    prefix(&mut tree);
    press(&mut tree, KeyCode::Char('t'), NONE);
    modes.push(("sidebar tree", tree));

    let mut overview = gate_app(w, h);
    prefix(&mut overview);
    press(&mut overview, KeyCode::Char('T'), NONE);
    modes.push(("project overview", overview));

    modes.push(("run view at the gate", run_view_app(None, false, w, h)));
    let planning = run_view_app(Some(RunState::Planning), false, w, h);
    modes.push(("run view planning", planning));
    let held = run_view_app(Some(RunState::Running), true, w, h);
    modes.push(("run view with a hold", held));
    let (snap, windows) = three_task_fixture();
    let mut running = app_with(snap, windows, w, h);
    running.open_run_view(RUN_ID.into());
    modes.push(("run view", running));

    let mut review = gate_app(w, h);
    review.open_plan_review(RUN_ID.into(), ReviewTarget::Gate);
    modes.push(("plan review", review));
    let mut hold_review = gate_app(w, h);
    hold_review.open_plan_review(RUN_ID.into(), ReviewTarget::Hold("h".into()));
    modes.push(("hold review", hold_review));

    let mut alerts = gate_app(w, h);
    prefix(&mut alerts);
    press(&mut alerts, KeyCode::Char('a'), NONE);
    assert!(alerts.alerts_focus.is_some());
    modes.push(("alerts view", alerts));

    let mut menu = gate_app(w, h);
    open_menu_on_the_run(&mut menu);
    modes.push(("action menu", menu));

    let mut profile = gate_app(w, h);
    let _ = profile.open_profile_on("/r/demo".into(), false);
    modes.push(("profile", profile));
    let mut settings = gate_app(w, h);
    prefix(&mut settings);
    press(&mut settings, KeyCode::Char('S'), KeyModifiers::SHIFT);
    modes.push(("settings", settings));
    let mut stats = gate_app(w, h);
    let _ = stats.open_stats("/r/demo".into());
    modes.push(("stats", stats));

    for (name, mut app) in modes {
        for ascii in [false, true] {
            app.settings.badges.ascii = ascii;
            let text = bar(&app, w, h);
            assert!(
                text.ends_with("esc back"),
                "{name} (ascii {ascii}): {text:?}"
            );
            // Final fix wave (task 7's deferred minor 3): ASCII mode's bar is ASCII.
            assert!(!ascii || text.is_ascii(), "{name}: {text:?}");
        }
    }
}

/// Decision 33: the run view at its gate drops its hints by priority, `f` first, and
/// keeps approve, reject and review to the last.
#[test]
fn hints_drop_by_priority() {
    let full = bar(&run_view_app(None, false, 160, 24), 160, 24);
    assert_eq!(
        full,
        " RUN  a approve  x reject  e edit  d remove  p review  ⏎ open  . actions  \
         f filter: all  esc back"
    );
    // `f` (3) is the first to go, `⏎ open` (4) still shows.
    let text = bar(&run_view_app(None, false, 90, 24), 90, 24);
    assert!(!text.contains("f filter"), "{text:?}");
    assert!(text.contains("⏎ open  . actions  esc back"), "{text:?}");
    let text = bar(&run_view_app(None, false, 60, 24), 60, 24);
    assert_eq!(
        text,
        " RUN  a approve  x reject  p review  . actions  esc back"
    );
}

/// Pinning 9.0.6's `the_pending_prefix_lists_the_next_keys`: the prefix list's text and
/// priorities are unchanged by decision 33.
#[test]
fn the_prefix_list_is_unchanged() {
    let windows = vec![pty(1, "shell", "/tmp", Status::Idle)];
    let mut app = app_with(snapshot(1, vec![]), windows, 120, 24);
    prefix(&mut app);
    assert_eq!(
        bar(&app, 120, 24),
        " PREFIX  C-b › a alerts · g goal · T run · P profile · S settings · ? help"
    );
    // Narrower, the lowest priorities drop whole: settings (4), then profile (5).
    assert_eq!(
        bar(&app, 60, 24),
        " PREFIX  C-b › a alerts · g goal · T run · ? help"
    );
}

/// Final fix wave (task 7's deferred minor 1): a planning run with an awaiting hold
/// keeps the hold's keys; the hold is what waits on the user.
#[test]
fn a_planning_run_with_a_hold_shows_the_holds_keys() {
    let app = run_view_app(Some(RunState::Planning), true, 160, 24);
    let text = bar(&app, 160, 24);
    assert!(
        text.starts_with(" RUN  a approve hold  x reject hold  p review"),
        "{text:?}"
    );
    assert_eq!(column_of(&draw(&app, 160, 24), 160, 24, ""), None);
}
