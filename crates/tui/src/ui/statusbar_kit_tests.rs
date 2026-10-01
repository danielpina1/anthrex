//! Milestone 9.0.6 task 4: the status bar's mode badges, the prefix list, hints through
//! `kit::hints`, toast severity and the kit-drawn confirm modals.

use crate::app::{App, Link, Modal, PendingAction, ReviewTarget, ToastLevel};
use crate::settings::UiSettings;
use crate::theme::{Role, role};
use crate::tree::run_fixtures::{RUN_ID, gate_fixture};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{ClientMsg, DaemonMsg, GitState, Head, RunReply, RunRequest, WindowInfo};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Vec<crate::app::Effect> {
    app.on_key(KeyEvent::new(code, mods))
}

fn prefix(app: &mut App) {
    press(app, KeyCode::Char('b'), KeyModifiers::CONTROL);
}

fn gate_app(width: u16, height: u16) -> App {
    let (snapshot, windows) = gate_fixture();
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    app.set_terminal_size(width, height);
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snapshot)));
    app
}

fn draw(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

/// The bottom row, trailing spaces trimmed.
fn bar(app: &App, width: u16, height: u16) -> String {
    let buffer = draw(app, width, height);
    (0..width)
        .map(|x| buffer[(x, height - 1)].symbol())
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn win(id: u32, worktree: Option<&str>) -> WindowInfo {
    WindowInfo {
        id,
        name: format!("w{id}"),
        runtime: proto::Runtime::Shell,
        cwd: "/tmp".into(),
        project: "/tmp".into(),
        worktree: worktree.map(Into::into),
        branch: None,
        status: proto::Status::Idle,
        tool: None,
        since_secs: 0,
        last_output_secs: 0,
        session_id: None,
        model: None,
        subagents: vec![],
        exit: None,
        kind: proto::WindowKind::Pty,
        run: None,
        signals_seen: false,
    }
}

fn clean_git() -> GitState {
    GitState {
        head: Head::Branch("main".into()),
        upstream: None,
        ahead: 0,
        behind: 0,
        dirty: 0,
        untracked: 0,
        conflicts: 0,
        operation: None,
        stale: false,
    }
}

#[test]
fn badges_follow_the_mode() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = gate_app(w, h);
        let plain = bar(&app, w, h);
        for badge in [
            " PREFIX ", " PLAN ", " ALERTS ", " CHAT ", " RUN ", " TREE ", " FILTER ",
        ] {
            assert!(!plain.contains(badge), "{w}: {plain:?}");
        }

        // Tree navigation without a run view.
        prefix(&mut app);
        assert!(bar(&app, w, h).starts_with(" PREFIX "), "{w}");
        press(&mut app, KeyCode::Char('t'), KeyModifiers::NONE);
        assert!(
            bar(&app, w, h).starts_with(" TREE "),
            "{w}: {}",
            bar(&app, w, h)
        );
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(bar(&app, w, h).starts_with(" FILTER "), "{w}");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.tree_input.is_none());

        // The alerts box, then the conversation, then the run view, then the plan review.
        prefix(&mut app);
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(app.alerts_focus.is_some());
        assert!(bar(&app, w, h).starts_with(" ALERTS "), "{w}");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.alerts_focus.is_none());

        app.open_conversation(1);
        assert!(app.conversation.is_open());
        assert!(
            bar(&app, w, h).starts_with(" CHAT "),
            "{w}: {}",
            bar(&app, w, h)
        );
        app.conversation.close();

        app.open_run_view(RUN_ID.into());
        assert!(
            bar(&app, w, h).starts_with(" RUN "),
            "{w}: {}",
            bar(&app, w, h)
        );
        // Precedence: the conversation sits above the run view, the plan review above both.
        app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate);
        assert!(
            bar(&app, w, h).starts_with(" PLAN "),
            "{w}: {}",
            bar(&app, w, h)
        );
        // The prefix pending wins over the review.
        prefix(&mut app);
        assert!(bar(&app, w, h).starts_with(" PREFIX "), "{w}");
    }
}

#[test]
fn the_pending_prefix_lists_the_next_keys() {
    let mut app = App::new(vec![win(1, None)], "/tmp".into(), UiSettings::default());
    app.set_terminal_size(120, 24);
    prefix(&mut app);
    assert_eq!(
        bar(&app, 120, 24),
        " PREFIX  C-b › a alerts · g goal · T run · P profile · S settings · ? help"
    );

    // At 80 columns with a git segment the lowest priorities drop whole; help stays.
    let mut app = App::new(
        vec![win(1, Some("/repo"))],
        "/tmp".into(),
        UiSettings::default(),
    );
    app.set_terminal_size(80, 24);
    app.on_daemon(DaemonMsg::Git {
        root: "/repo".into(),
        state: Some(clean_git()),
    });
    prefix(&mut app);
    let text = bar(&app, 80, 24);
    assert!(text.contains("? help"), "{text:?}");
    assert!(text.contains("main"), "{text:?}");
    assert!(!text.contains("S settings"), "{text:?}");
    assert!(text.starts_with(" PREFIX  C-b › a alerts"), "{text:?}");
    // Narrower still: whole hints only, never a cut word.
    let text = bar(&app, 40, 24);
    assert!(text.contains("? help"), "{text:?}");
    assert!(!text.contains("setti"), "{text:?}");
}

#[test]
fn hints_never_cut_esc() {
    let w = 40;
    let h = 24;
    let mut modes: Vec<(&str, App)> = Vec::new();

    let mut tree = gate_app(w, h);
    prefix(&mut tree);
    press(&mut tree, KeyCode::Char('t'), KeyModifiers::NONE);
    modes.push(("tree", tree));

    let mut run = gate_app(w, h);
    run.open_run_view(RUN_ID.into());
    modes.push(("run view", run));

    let mut review = gate_app(w, h);
    review.open_plan_review(RUN_ID.into(), ReviewTarget::Gate);
    modes.push(("plan review", review));

    let mut hold = gate_app(w, h);
    hold.open_plan_review(RUN_ID.into(), ReviewTarget::Hold("h".into()));
    modes.push(("hold review", hold));

    let mut alerts = gate_app(w, h);
    prefix(&mut alerts);
    press(&mut alerts, KeyCode::Char('a'), KeyModifiers::NONE);
    assert!(alerts.alerts_focus.is_some());
    modes.push(("alerts", alerts));

    for (name, app) in modes {
        for width in [40, 24] {
            let text = bar(&app, width, h);
            assert!(text.ends_with("esc back"), "{name} at {width}: {text:?}");
        }
    }
}

fn toast_style(app: &App) -> ratatui::style::Style {
    let buffer = draw(app, 80, 24);
    let text = app.toast_text().unwrap().to_string();
    let row: String = (0..80).map(|x| buffer[(x, 23)].symbol()).collect();
    let at = row.find(&text).expect("the toast is drawn") as u16;
    let cell = &buffer[(at, 23)];
    ratatui::style::Style::default().fg(cell.fg)
}

#[test]
fn an_error_toast_is_drawn_in_failed() {
    let mut app = gate_app(80, 24);
    app.toast_at(ToastLevel::Error, "no such run");
    assert_eq!(app.toast_level(), Some(ToastLevel::Error));
    let failed = role(Role::Failed, app.palette());
    assert_eq!(toast_style(&app).fg, failed.fg);
}

#[test]
fn a_warning_in_attention() {
    let mut app = gate_app(80, 24);
    app.toast_at(ToastLevel::Warn, "careful");
    let attention = role(Role::Attention, app.palette());
    assert_eq!(toast_style(&app).fg, attention.fg);
    // An info toast keeps the accent, and `toast()` means Info.
    app.toast("fine");
    assert_eq!(app.toast_level(), Some(ToastLevel::Info));
    assert_eq!(toast_style(&app).fg, Some(app.settings.accent));
}

#[test]
fn a_refusal_toasts_as_an_error() {
    let mut app = gate_app(80, 24);
    app.on_daemon(DaemonMsg::Run(RunReply::Refused {
        request: "approve".into(),
        message: "the plan gate is closed".into(),
        request_id: None,
    }));
    assert_eq!(app.toast_text(), Some("the plan gate is closed"));
    assert_eq!(app.toast_level(), Some(ToastLevel::Error));
}

#[test]
fn a_destructive_confirm_ignores_enter() {
    let mut app = gate_app(80, 24);
    app.open_run_view(RUN_ID.into());
    press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(matches!(
        app.modal,
        Some(Modal::Confirm {
            action: PendingAction::RejectRun(_),
            ..
        })
    ));
    // Drawn on the kit: the destructive title and the y-only hint.
    let screen: String = {
        let buffer = draw(&app, 80, 24);
        (0..24)
            .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
            .collect()
    };
    assert!(screen.contains("y reject · esc back"), "{screen}");
    assert!(!screen.contains("Enter"), "{screen}");

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(effects.is_empty(), "{effects:?}");
    assert!(app.modal.is_some());
    assert_eq!(app.toast_text(), Some("press y to reject"));

    let effects = press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    assert!(app.modal.is_none());
    assert!(
        effects.iter().any(|e| matches!(
            e,
            crate::app::Effect::Send(ClientMsg::Run(RunRequest::Reject { .. }))
        )),
        "{effects:?}"
    );
}

#[test]
fn a_non_destructive_confirm_still_takes_enter() {
    let mut app = gate_app(80, 24);
    app.open_run_view(RUN_ID.into());
    press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
    assert!(matches!(
        app.modal,
        Some(Modal::Confirm {
            action: PendingAction::ApproveRun(_),
            ..
        })
    ));
    let buffer = draw(&app, 80, 24);
    let screen: String = (0..24)
        .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
        .collect();
    assert!(screen.contains("⏎ approve · esc back"), "{screen}");
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.modal.is_none());
}

#[test]
fn a_disconnected_bar_keeps_its_badge() {
    let mut app = gate_app(80, 24);
    app.link = Link::Lost { reason: "x".into() };
    assert!(bar(&app, 80, 24).contains("DISCONNECTED"));
}

fn confirm_app(action: PendingAction) -> App {
    let mut app = gate_app(80, 24);
    app.modal = Some(Modal::Confirm {
        message: "sure?".into(),
        action,
    });
    app
}

/// Decision 5, every destructive confirm: Enter keeps the modal and toasts
/// `press y to <verb>`; the non-destructive confirms take Enter.
#[test]
fn enter_is_ignored_by_every_destructive_confirm_and_taken_by_the_rest() {
    let run = || "r".to_string();
    let destructive = [
        (PendingAction::RejectRun(run()), "reject"),
        (
            PendingAction::RemoveTask {
                run_id: run(),
                task_id: "t1".into(),
            },
            "remove",
        ),
        (
            PendingAction::RejectHold {
                run_id: run(),
                hold: "h".into(),
            },
            "reject hold",
        ),
        (PendingAction::StopDaemon, "stop the daemon"),
        (PendingAction::Kill(1), "kill"),
    ];
    for (action, verb) in destructive {
        let mut app = confirm_app(action);
        let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(effects.is_empty(), "{verb}: {effects:?}");
        assert!(app.modal.is_some(), "{verb}");
        assert_eq!(
            app.toast_text(),
            Some(format!("press y to {verb}").as_str())
        );
        assert_eq!(app.toast_level(), Some(ToastLevel::Warn));
    }
    for action in [
        PendingAction::Restart(1),
        PendingAction::SubmitPlan("r".into()),
    ] {
        let mut app = confirm_app(action);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.modal.is_none());
    }

    // The remove-agent dialog is destructive too.
    let mut app = gate_app(80, 24);
    app.modal = Some(Modal::Remove(crate::dialog::RemoveConfirm {
        window_id: 1,
        name: "w".into(),
        branch: None,
        remove_worktree: false,
    }));
    assert!(press(&mut app, KeyCode::Enter, KeyModifiers::NONE).is_empty());
    assert!(matches!(app.modal, Some(Modal::Remove(_))));
    assert_eq!(app.toast_text(), Some("press y to remove"));
    assert_eq!(
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE).len(),
        1
    );
}

#[test]
fn the_enter_hint_has_an_ascii_twin() {
    let mut app = confirm_app(PendingAction::SubmitPlan("r".into()));
    app.settings.badges.ascii = true;
    let buffer = draw(&app, 80, 24);
    let screen: String = (0..24)
        .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
        .collect();
    assert!(screen.contains("enter submit"), "{screen}");
    assert!(!screen.contains('⏎'), "{screen}");
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
