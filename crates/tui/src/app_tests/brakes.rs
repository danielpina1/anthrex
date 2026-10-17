//! Milestone 9.16 decisions 31 and 32: `C-b H`'s confirm and its request, the ` HELD `
//! badge, and the brakes' alerts (the user's, always), with Enter on them. Pure App
//! tests: snapshots in, effects out; no clock and no daemon.

use crate::app::alerts::DispatchHeldKind;
use crate::app::{
    AlertKey, AlertWho, App, Effect, Modal, PendingAction, Route, alert_route, alerts,
    orchestrator_lives, route_kind,
};
use crate::settings::UiSettings;
use crate::tree::orch_fixtures::orch_fixture;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::brakes::{
    BrakeTripInfo, BrakesInfo, DispatchHold, Permille, QuotaInfo, QuotaState, QuotaWindow,
};
use proto::{ClientMsg, DaemonMsg, RunReply, RunRequest, RunState, RunsSnapshot, Runtime};
use ratatui::{Terminal, backend::TestBackend};

/// 1970-01-02 14:30 UTC.
const UNTIL: u64 = 86_400 + 14 * 3600 + 30 * 60;

fn deliver(app: &mut App, snapshot: RunsSnapshot) {
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snapshot)));
}

/// The orchestrated fixture (its orchestrator lives) with `brakes`.
fn snapshot_with(brakes: BrakesInfo) -> (RunsSnapshot, Vec<proto::WindowInfo>) {
    let (mut snapshot, windows) = orch_fixture(RunState::Running);
    snapshot.brakes = brakes;
    (snapshot, windows)
}

fn app_with(brakes: BrakesInfo) -> App {
    let (snapshot, windows) = snapshot_with(brakes);
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    let _ = app.set_terminal_size(100, 30);
    deliver(&mut app, snapshot);
    app
}

fn held() -> BrakesInfo {
    BrakesInfo {
        hold: Some(DispatchHold {
            since: 1_000,
            by: "user".into(),
        }),
        quota_brake: true,
        quota: Vec::new(),
    }
}

/// Codex braked on its 7-day window; Claude measured and fine.
fn codex_braked() -> BrakesInfo {
    let trip = BrakeTripInfo {
        window: QuotaWindow::SevenDay,
        used: Permille(968),
        threshold_percent: 95,
        until: UNTIL,
    };
    let quota = |runtime, state, trip| QuotaInfo {
        runtime,
        state,
        windows: Vec::new(),
        read_at: Some(0),
        trip,
    };
    BrakesInfo {
        hold: None,
        quota_brake: true,
        quota: vec![
            quota(Runtime::Claude, QuotaState::Fresh, None),
            quota(Runtime::Codex, QuotaState::Braked, Some(trip)),
        ],
    }
}

fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Vec<Effect> {
    app.on_key(KeyEvent::new(code, mods))
}

fn c_b_h(app: &mut App) -> Vec<Effect> {
    press(app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    press(app, KeyCode::Char('H'), KeyModifiers::SHIFT)
}

fn confirm_action(app: &App) -> PendingAction {
    match &app.modal {
        Some(Modal::Confirm { action, .. }) => action.clone(),
        other => panic!("no confirm: {other:?}"),
    }
}

fn hold_sent(on: bool) -> Vec<Effect> {
    vec![Effect::Send(ClientMsg::Run(RunRequest::Hold { on }))]
}

#[test]
#[allow(non_snake_case)]
fn c_b_H_confirms_then_sends_hold() {
    let mut app = app_with(BrakesInfo::default());
    assert!(c_b_h(&mut app).is_empty());
    let action = confirm_action(&app);
    assert_eq!(action, PendingAction::DispatchHold(true));
    assert_eq!(action.verb(), "hold dispatch");
    assert!(!action.destructive());
    assert!(press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE).is_empty());
    assert!(app.modal.is_none());

    c_b_h(&mut app);
    assert_eq!(
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE),
        hold_sent(true)
    );
    assert!(app.modal.is_none());

    // With a hold, the confirm releases it.
    let (snapshot, _) = snapshot_with(held());
    deliver(&mut app, snapshot);
    c_b_h(&mut app);
    let action = confirm_action(&app);
    assert_eq!(action.verb(), "release dispatch");
    assert_eq!(
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE),
        hold_sent(false)
    );
}

/// The daemon's answer is toasted.
#[test]
fn the_held_reply_is_toasted() {
    let mut app = app_with(BrakesInfo::default());
    let message = "dispatch released";
    app.on_daemon(DaemonMsg::Run(RunReply::Held {
        hold: None,
        message: message.into(),
        request_id: None,
    }));
    assert_eq!(app.toast_text(), Some(message));
}

/// The bottom row of a 100×30 frame.
fn bar(app: &App) -> String {
    let (w, h) = (100, 30);
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..w).map(|x| buffer[(x, h - 1)].symbol()).collect()
}

#[test]
fn held_badge_shows_while_held() {
    let mut app = app_with(BrakesInfo::default());
    assert!(!bar(&app).contains(" HELD "), "{}", bar(&app));
    let (snapshot, _) = snapshot_with(held());
    deliver(&mut app, snapshot);
    assert!(bar(&app).contains(" HELD "), "{}", bar(&app));
    // A quota brake alone is no hold.
    let (snapshot, _) = snapshot_with(codex_braked());
    deliver(&mut app, snapshot);
    assert!(!bar(&app).contains(" HELD "), "{}", bar(&app));
}

fn held_alerts(app: &App) -> Vec<crate::app::Alert> {
    alerts(app)
        .into_iter()
        .filter(|a| matches!(a.key, AlertKey::DispatchHeld(_)))
        .collect()
}

#[test]
fn dispatch_held_alerts_are_the_users() {
    let app = app_with(held());
    assert!(orchestrator_lives(&app.runs.runs[0]));
    let list = held_alerts(&app);
    assert_eq!(list.len(), 1, "{list:?}");
    let alert = &list[0];
    assert_eq!(alert.key, AlertKey::DispatchHeld(DispatchHeldKind::Hold));
    assert_eq!(alert.priority, 2);
    assert_eq!(alert.who, AlertWho::Daemon);
    assert_eq!(alert.text, "dispatch held daemon-wide · C-b H releases it");
    let kind = route_kind(&alert.key, Some(&app.runs.runs[0]));
    assert_eq!(alert_route(kind, true), Route::User);

    let app = app_with(codex_braked());
    let list = held_alerts(&app);
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(
        list[0].key,
        AlertKey::DispatchHeld(DispatchHeldKind::Quota(Runtime::Codex))
    );
    assert_eq!(
        list[0].text,
        "codex dispatch held: 7d at 96% (≥95%) until 14:30 UTC"
    );
    assert_eq!((list[0].priority, &list[0].who), (2, &AlertWho::Daemon));

    let app = app_with(BrakesInfo::default());
    assert!(held_alerts(&app).is_empty());
}

#[test]
fn enter_on_the_hold_alert_opens_the_confirm() {
    let mut app = app_with(held());
    press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
    let key = AlertKey::DispatchHeld(DispatchHeldKind::Hold);
    // Down to the hold's alert.
    for _ in 0..20 {
        let selected = app.alerts_focus.as_ref().and_then(|f| f.selected.clone());
        if selected.as_ref() == Some(&key) {
            break;
        }
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    }
    assert!(press(&mut app, KeyCode::Enter, KeyModifiers::NONE).is_empty());
    assert_eq!(confirm_action(&app), PendingAction::DispatchHold(false));
    assert_eq!(
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE),
        hold_sent(false)
    );

    // A quota alert's Enter only says what releases it.
    let mut app = app_with(codex_braked());
    let effects = app.enter_dispatch_held(DispatchHeldKind::Quota(Runtime::Codex));
    assert!(effects.is_empty() && app.modal.is_none());
    assert_eq!(app.toast_text(), Some(super::QUOTA_WAITS));
}

fn queued(n: usize) -> Vec<proto::QueuedGoalInfo> {
    (0..n)
        .map(|i| proto::QueuedGoalInfo {
            id: format!("q{i}"),
            project: "/tmp/p".into(),
            goal: format!("goal {i}"),
            queued_at: 0,
            yes: false,
            trust_project: false,
            unconfined_checks: false,
            setup: proto::SetupState::Reading,
        })
        .collect()
}

fn confirm_message(app: &App) -> String {
    match &app.modal {
        Some(Modal::Confirm { message, .. }) => message.clone(),
        other => panic!("no confirm: {other:?}"),
    }
}

/// Task 10's review I1: the release confirm names how many queued goals start, and a
/// snapshot that changes that count (or the hold) closes it, so `y` never confirms a
/// stale count.
#[test]
fn the_release_confirm_names_the_queued_goals_and_rechecks_them() {
    let (mut snapshot, windows) = snapshot_with(held());
    snapshot.queued_goals = queued(3);
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    let _ = app.set_terminal_size(100, 30);
    deliver(&mut app, snapshot.clone());
    c_b_h(&mut app);
    assert_eq!(
        confirm_message(&app),
        "Release dispatch? New sessions may start again; 3 queued goals will start."
    );

    // The same count: the confirm stays open.
    deliver(&mut app, snapshot.clone());
    assert_eq!(confirm_action(&app), PendingAction::DispatchHold(false));

    // One more goal queued: it closes and says why; `y` sends nothing.
    snapshot.queued_goals = queued(4);
    deliver(&mut app, snapshot.clone());
    assert!(app.modal.is_none());
    assert_eq!(
        app.toast_text(),
        Some("the queued goals changed; press C-b H again")
    );
    let effects = press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    assert!(!effects.contains(&hold_sent(false)[0]), "{effects:?}");

    // One goal, the singular; none, the plain question.
    snapshot.queued_goals = queued(1);
    deliver(&mut app, snapshot.clone());
    c_b_h(&mut app);
    assert_eq!(
        confirm_message(&app),
        "Release dispatch? New sessions may start again; 1 queued goal will start."
    );
    snapshot.queued_goals = queued(0);
    deliver(&mut app, snapshot.clone());
    assert!(app.modal.is_none());
    c_b_h(&mut app);
    assert_eq!(confirm_message(&app), super::RELEASE_ASK);

    // Another client released the hold: the release confirm closes too.
    let (released, _) = snapshot_with(BrakesInfo::default());
    deliver(&mut app, released);
    assert!(app.modal.is_none());
    assert_eq!(
        app.toast_text(),
        Some("the dispatch hold changed; press C-b H again")
    );
}

/// Milestone 9.19 (M9.19.4 review, Minor 2): the ASCII fold's `≥` → `>=` reaches a
/// quota trip's alert detail, the one user text with `≥`; the Unicode form keeps `≥`.
#[test]
fn a_quota_alerts_threshold_folds_to_ascii() {
    let screen = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|f| {
                crate::ui::draw(f, app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..40)
            .map(|y| {
                (0..120)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    for (ascii, want) in [(true, "(>=95%)"), (false, "(≥95%)")] {
        let mut app = app_with(codex_braked());
        app.settings.badges.ascii = ascii;
        press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        let mut found = false;
        for _ in 0..8 {
            let text = screen(&app);
            if ascii {
                assert!(!text.contains('≥'), "{text}");
            }
            if text.contains(want) {
                found = true;
                break;
            }
            press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        }
        assert!(found, "no {want} in the alerts view (ascii {ascii})");
    }
}
