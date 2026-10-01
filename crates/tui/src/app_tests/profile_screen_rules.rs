//! Milestone 9.0.6 task 13, fix round 1: the Profile screen across a lost link, the
//! keys it refuses, and the rules of its proposal view.

use super::profile_screen::{
    dir, open, open_app, profile_requests, ready_app, reply, screen, status, tagged, tap,
};
use super::*;
use crate::app::profile_screen::{ProfilePage, Side};
use proto::{ProfileReply, ProfileRequest, ProposalState, RunRequest};
use std::time::{Duration, Instant};

fn statuses(effects: &[Effect]) -> usize {
    profile_requests(effects)
        .iter()
        .filter(|r| matches!(r, ProfileRequest::Status { .. }))
        .count()
}

/// A lost link stops the poll quietly; the reconnect asks the three things once.
#[test]
fn the_poll_stops_with_the_link_and_the_reconnect_asks_again() {
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(Some(ProposalState::Scouting)));
    let t0 = Instant::now();
    app.on_link_lost("gone");
    let toast = app.toast_text().map(str::to_owned);
    for s in 1..=5 {
        let effects = app.screens_tick(t0 + Duration::from_secs(s));
        assert!(effects.is_empty(), "no send while disconnected ({s} s)");
    }
    assert_eq!(app.toast_text().map(str::to_owned), toast, "no new toast");
    let effects = app.on_reconnected(vec![project_win(1, DIR_OF_TEST)]);
    assert_eq!(
        profile_requests(&effects),
        vec![
            ProfileRequest::Status { dir: dir() },
            ProfileRequest::Show {
                dir: dir(),
                proposed: false
            },
            ProfileRequest::Show {
                dir: dir(),
                proposed: true
            },
        ]
    );
    // The new Status is in flight: the tick waits for it, then polls again.
    let status_id = tagged(&effects)
        .into_iter()
        .find(|(_, r)| matches!(r, RunRequest::Profile(ProfileRequest::Status { .. })))
        .unwrap()
        .0;
    assert_eq!(statuses(&app.screens_tick(t0 + Duration::from_secs(9))), 0);
    reply(&mut app, status_id, status(Some(ProposalState::Scouting)));
    assert_eq!(statuses(&app.screens_tick(t0 + Duration::from_secs(12))), 1);
}

const DIR_OF_TEST: &str = "/p/shop";

/// A view request the connection refused is not waited on, and says nothing.
#[test]
fn a_refused_view_send_is_quiet() {
    let mut app = open_app();
    let ids = open(&mut app);
    let msg = ClientMsg::RunTagged {
        id: ids[0],
        request: RunRequest::Profile(ProfileRequest::Status { dir: dir() }),
    };
    assert!(app.on_send_failed(&msg).is_empty());
    assert!(!app.replies.contains(ids[0]));
    assert_eq!(app.toast_text(), None);
}

/// A late refused `Show` (its entry expired) is dropped, not toasted.
#[test]
fn a_late_refused_show_is_not_toasted() {
    let mut app = open_app();
    let ids = open(&mut app);
    let past = Instant::now() - Duration::from_secs(120);
    app.set_reply_sent_at(ids[2], past);
    app.on_tick();
    assert_eq!(app.toast_text(), Some("no reply from daemon"));
    let refusal = ProfileReply::Refused {
        message: "no proposal for /p/shop".into(),
    };
    assert!(reply(&mut app, ids[2], refusal).is_empty());
    assert_eq!(app.toast_text(), Some("no reply from daemon"));
}

/// Progress ruling: `C-b a`, `C-b m` and `C-b t` are refused over the screen.
#[test]
fn prefix_keys_that_act_under_the_screen_are_refused() {
    for c in ['a', 'm', 't'] {
        let (mut app, _) = ready_app();
        let tree = app.tree_input;
        prefix(&mut app);
        assert!(tap(&mut app, KeyCode::Char(c)).is_empty(), "{c}");
        assert_eq!(
            app.toast_text(),
            Some("leave the profile first (esc)"),
            "{c}"
        );
        assert_eq!(app.alerts_focus, None, "{c}");
        assert!(!app.conversation.is_open(), "{c}");
        assert_eq!(app.tree_input, tree, "{c}");
        assert!(app.screen.is_some());
    }
}

/// Progress ruling: the Detect page starts a real agent, so only `y` sends it.
#[test]
fn the_detect_page_takes_only_y() {
    let (mut app, _) = ready_app();
    tap(&mut app, KeyCode::Char('d'));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("press y to detect"));
    assert!(matches!(
        screen(&app).page,
        Some(ProfilePage::Detect { .. })
    ));
}

/// `c` needs the proposal's state to be `Ready`; while a new detection runs, the
/// proposal view shows that detection, not the proposal it replaces.
#[test]
fn a_running_detection_hides_the_old_proposal() {
    let (mut app, _) = ready_app();
    tap(&mut app, KeyCode::Char('d'));
    let effects = tap(&mut app, KeyCode::Char('y'));
    let id = tagged(&effects)[0].0;
    let effects = reply(
        &mut app,
        id,
        ProfileReply::Done {
            message: "detection started".into(),
        },
    );
    let status_id = tagged(&effects)[0].0;
    reply(&mut app, status_id, status(Some(ProposalState::Scouting)));
    assert!(
        matches!(screen(&app).proposal, Side::Ready(_)),
        "the old one"
    );
    tap(&mut app, KeyCode::Char('c'));
    assert_eq!(screen(&app).page, None);
    assert_eq!(app.toast_text(), Some("no proposal is ready to confirm"));
    tap(&mut app, KeyCode::Tab);
    tap(&mut app, KeyCode::Char('p'));
    assert!(screen(&app).rows().is_empty());
    let text = render(&app);
    assert!(text.contains("scout running"), "{text}");
    assert!(!text.contains("cargo test --workspace"), "{text}");
}

fn render(app: &App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
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
                + "\n"
        })
        .collect()
}

/// The Confirm page's scroll stops at the TOML's last line.
#[test]
fn the_confirm_scroll_is_bounded() {
    let (mut app, _) = ready_app();
    tap(&mut app, KeyCode::Char('c'));
    for _ in 0..50 {
        tap(&mut app, KeyCode::PageDown);
    }
    let Some(ProfilePage::Confirm { toml, scroll }) = &screen(&app).page else {
        panic!("no confirm page");
    };
    let last = toml.lines().count() - 1;
    assert_eq!(*scroll, last);
    tap(&mut app, KeyCode::Char('k'));
    let Some(ProfilePage::Confirm { scroll: after, .. }) = &screen(&app).page else {
        panic!("no confirm page");
    };
    assert_eq!(*after, last - 1);
}

/// Decision 34: a selected stage's run gives the project.
#[test]
fn a_selected_stage_is_its_runs_project() {
    use crate::tree::run_fixtures::{RUN_ID, gate_fixture};
    let (snapshot, windows) = gate_fixture();
    let project = snapshot.runs[0].project.clone();
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    app.set_terminal_size(80, 24);
    app.on_daemon(DaemonMsg::Run(proto::RunReply::Snapshot(snapshot)));
    app.focused = None;
    app.tree.selected = Some(crate::tree::NodeKey::Stage {
        run: RUN_ID.into(),
        n: 1,
    });
    assert_eq!(app.goal_project(), Some(project));
}
