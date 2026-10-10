//! Milestone 9.10 decisions 33-35: one review alert per ready review proposal with its
//! waiting goals, the set-up alert (progress, failure and Retry) for a repository whose
//! goals wait for its profile, and the goal form's `Queued` reply.

use super::alerts::listed;
use super::goal_editor::{ctrl, drawn, send_and_close};
use super::goal_form::{app as form_app, open_form, typed};
use super::orch::tagged;
use super::profile_screen::screen;
use super::runs::app_with_runs;
use super::*;
use crate::app::alerts_route::{Route, RouteKind, alert_route, route_kind};
use crate::app::alerts_view::enter_label;
use crate::app::{AlertKey, AlertWho, alerts};
use crate::theme::{Glyph, Palette, Role, alert_glyph, alert_style, glyph, role};
use crate::tree::alert_fixtures::{at, orch_window, with_orch};
use crate::tree::run_fixtures::snapshot;
use proto::{
    CheckProgress, ProfileRequest, ProposalAlertInfo, QueuedGoalInfo, RunReply, RunRequest,
    RunState, RunsSnapshot, SetupState,
};
use std::path::PathBuf;

const SHOP: &str = "/r/shop";

fn queued(id: &str, project: &str, goal: &str, setup: SetupState) -> QueuedGoalInfo {
    QueuedGoalInfo {
        id: id.into(),
        project: project.into(),
        goal: goal.into(),
        queued_at: 1,
        yes: false,
        trust_project: false,
        unconfined_checks: false,
        setup,
    }
}

fn checking(done: u32, total: u32) -> SetupState {
    SetupState::Checking {
        progress: Some(CheckProgress { done, total }),
    }
}

fn with_queue(goals: Vec<QueuedGoalInfo>) -> RunsSnapshot {
    let mut snap = snapshot(1, vec![]);
    snap.queued_goals = goals;
    snap
}

/// `C-b a`, then `j` until `key` is selected.
fn view_on(app: &mut App, key: &AlertKey) {
    prefix(app);
    press(app, KeyCode::Char('a'), KeyModifiers::NONE);
    for _ in 0..20 {
        let on = app.alerts_focus.as_ref().and_then(|f| f.selected.clone());
        if on.as_ref() == Some(key) {
            return;
        }
        press(app, KeyCode::Char('j'), KeyModifiers::NONE);
    }
    panic!("{key:?} is not listed");
}

/// Decision 33: a ready review proposal is one alert, whatever waits on it; its detail
/// lists each waiting goal.
#[test]
fn a_ready_proposal_is_one_review_alert_with_its_waiting_goals() {
    let mut snap = with_queue(vec![
        queued("q-1", SHOP, "add a", SetupState::NeedsReview),
        queued("q-2", SHOP, "fix b", SetupState::NeedsReview),
    ]);
    snap.proposals = vec![ProposalAlertInfo {
        project: SHOP.into(),
        updated_at: 1,
    }];
    let app = app_with_runs(vec![], snap);
    let list = alerts(&app);
    assert_eq!(list.len(), 1, "{list:?}");
    let alert = &list[0];
    assert_eq!(alert.key, AlertKey::Proposal(SHOP.into()));
    assert_eq!(alert.priority, 4);
    assert_eq!(alert.who, AlertWho::Project("shop".into()));
    assert_eq!(alert.text, "review how anthrex will work here");
    assert_eq!(
        alert.detail,
        "anthrex learned how to work in this repo; ⏎ opens the review\n\
         waiting: add a (profile needs review)\n\
         waiting: fix b (profile needs review)"
    );
    assert!(!alert.you);
}

/// Decision 34: one set-up alert per repository with queued goals and no ready review
/// proposal, at priority 5, with the set-up's progress.
#[test]
fn setting_up_is_one_alert_per_repository() {
    let snap = with_queue(vec![
        queued("q-1", SHOP, "add a", checking(2, 4)),
        queued("q-2", SHOP, "fix b", checking(2, 4)),
        queued("q-3", "/r/cart", "pay c", SetupState::Reading),
    ]);
    let app = app_with_runs(vec![], snap);
    assert_eq!(
        listed(&app),
        vec![
            (
                5,
                "shop".into(),
                "setting up anthrex · checking commands (2/4)".into()
            ),
            (
                5,
                "cart".into(),
                "setting up anthrex · reading the repo".into()
            ),
        ]
    );
    let list = alerts(&app);
    assert_eq!(list[0].key, AlertKey::Setup(SHOP.into()));
    assert_eq!(list[0].detail, "waiting: add a\nwaiting: fix b");
    assert_eq!(list[1].key, AlertKey::Setup("/r/cart".into()));
    assert!(list.iter().all(|alert| !alert.you));
    assert_eq!(enter_label(&app, &list[0].key), "open profile");
}

/// Decisions 7 and 34: a failed set-up is a priority 3 alert whose Enter retries the
/// detection with the first queued goal's flags.
#[test]
fn a_failed_set_up_alert_retries() {
    let failed = SetupState::Failed {
        reason: "the scout ended\nmore".into(),
    };
    let mut first = queued("q-1", SHOP, "add a", failed.clone());
    first.trust_project = true;
    let mut second = queued("q-2", SHOP, "fix b", failed);
    second.unconfined_checks = true;
    let mut app = app_with_runs(vec![], with_queue(vec![first, second]));
    let key = AlertKey::Setup(SHOP.into());
    assert_eq!(
        listed(&app),
        vec![(
            3,
            "shop".into(),
            "setting up failed: the scout ended".into()
        )]
    );
    assert_eq!(enter_label(&app, &key), "retry");
    view_on(&mut app, &key);
    let (id, request) = tagged(&press(&mut app, KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        request,
        RunRequest::Profile(ProfileRequest::Detect {
            dir: SHOP.into(),
            trust_project: true,
            unconfined_checks: false,
        })
    );
    assert!(app.replies.contains(id), "the retry's reply is awaited");
    assert!(app.alerts_focus.is_none(), "Enter leaves the view");
    assert!(app.screen.is_none(), "Retry opens no screen");
}

/// Decision 34: `o` on a set-up alert opens the Profile screen on its repository,
/// whether it is setting up or failed; Enter does too while it sets up.
#[test]
fn o_on_a_set_up_alert_opens_the_profile() {
    let failed = SetupState::Failed {
        reason: "boom".into(),
    };
    for (setup, code) in [
        (failed, KeyCode::Char('o')),
        (SetupState::Reading, KeyCode::Char('o')),
        (checking(1, 3), KeyCode::Enter),
    ] {
        let snap = with_queue(vec![queued("q-1", SHOP, "add a", setup.clone())]);
        let mut app = app_with_runs(vec![], snap);
        let key = AlertKey::Setup(SHOP.into());
        view_on(&mut app, &key);
        let effects = press(&mut app, code, KeyModifiers::NONE);
        assert!(
            !effects.is_empty(),
            "{setup:?}: the screen asks for its views"
        );
        assert_eq!(screen(&app).dir, PathBuf::from(SHOP), "{setup:?}");
        assert!(app.alerts_focus.is_none(), "{setup:?}");
    }
}

/// Decision 34: set-up alerts are the user's: routed as a proposal, and listed with
/// `you: false` even while an orchestrator lives in the same repository.
#[test]
fn setup_alerts_are_the_users() {
    let p = PathBuf::from(SHOP);
    assert_eq!(
        route_kind(&AlertKey::Setup(p.clone()), None),
        RouteKind::Proposal
    );
    assert_eq!(alert_route(RouteKind::Proposal, true), Route::User);
    let mut run = with_orch(at("r-1", RunState::Running, 1), 7);
    run.project = p.clone();
    if let Some(orch) = run.orchestrator.as_mut() {
        orch.live = true;
        orch.stuck = None;
    }
    let mut snap = snapshot(1, vec![run]);
    snap.queued_goals = vec![queued("q-1", SHOP, "add a", SetupState::Reading)];
    let windows = vec![orch_window(7, "r-1", proto::Status::Working, true)];
    let app = app_with_runs(windows, snap);
    let setup: Vec<_> = alerts(&app)
        .into_iter()
        .filter(|alert| alert.key == AlertKey::Setup(p.clone()))
        .collect();
    assert_eq!(setup.len(), 1, "{setup:?}");
    assert!(!setup[0].you);
}

fn queued_reply(id: u64, message: &str) -> DaemonMsg {
    DaemonMsg::Run(RunReply::Queued {
        goal_id: "q-1".into(),
        project: "/p/a".into(),
        message: message.into(),
        request_id: Some(id),
    })
}

/// Decision 35: `Queued` is a success with no run yet: the form closes, its draft goes,
/// the message is toasted and no run view is awaited. A closed dialog's draft goes too.
#[test]
fn a_queued_reply_closes_the_goal_form_and_clears_its_draft() {
    let message = "queued: setting up anthrex for a; the goal starts once you review it";
    let mut app = form_app();
    drawn(&mut app);
    open_form(&mut app);
    typed(&mut app, "add a");
    let (id, _) = tagged(&ctrl(&mut app, 's'));
    app.on_daemon(queued_reply(id, message));
    assert_eq!(app.modal, None);
    assert!(
        !app.goal_drafts.contains_key(&PathBuf::from("/p/a")),
        "{:?}",
        app.goal_drafts
    );
    assert_eq!(app.toast_text(), Some(message));
    assert_eq!(app.pending_open, None);
    assert!(app.run_view.is_none());

    // The dialog closed while it waited: its draft goes on the reply all the same.
    open_form(&mut app);
    typed(&mut app, "fix b");
    let id = send_and_close(&mut app);
    assert_eq!(app.goal_drafts[&PathBuf::from("/p/a")], "fix b");
    app.on_daemon(queued_reply(id, message));
    assert!(app.goal_drafts.is_empty(), "{:?}", app.goal_drafts);
    assert_eq!(app.toast_text(), Some(message));
    assert_eq!(app.pending_open, None);
}

/// Decision 34: priority 5 wears `Glyph::Checking` in `Role::Working`.
#[test]
fn priority_five_draws_as_working() {
    let p = Palette {
        accent: crate::theme::DEFAULT_ACCENT,
        truecolor: false,
        ascii: false,
    };
    assert_eq!(alert_glyph(5, false), glyph(Glyph::Checking, false));
    assert_eq!(alert_glyph(5, false), "◇");
    assert_eq!(alert_glyph(5, true), "~");
    assert_eq!(alert_style(5, p), role(Role::Working, p));
    // The others are unchanged.
    assert_eq!(
        [1, 2, 3, 4].map(|n| alert_glyph(n, false)),
        ["⚑", "⚑", "⚑", "✓"]
    );
}
