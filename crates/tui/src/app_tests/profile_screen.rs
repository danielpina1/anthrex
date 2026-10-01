//! Milestone 9.0.6 task 13, decisions 33-35: the Profile screen's keys, requests,
//! replies and polling. No test sleeps: the poll is driven by `screens_tick(now)` with
//! a moved clock (Global Constraint 11).

use super::*;
use crate::app::AlertKey;
use crate::app::profile_screen::{ProfilePage, ProfileScreen, ProfileTab, Side};
use crate::app::screens::Screen;
use proto::{
    DroppedCommand, ModuleNames, ProfileReply, ProfileRequest, ProfileSource, ProfileStatus,
    ProposalOrigin, ProposalRecord, ProposalState, RepoProfile, RunReply, RunRequest,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(super) const DIR: &str = "/p/shop";

pub(super) fn dir() -> PathBuf {
    DIR.into()
}

pub(super) fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

pub(super) fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        tap(app, KeyCode::Char(c));
    }
}

/// Every tagged request in `effects`, with its id.
pub(super) fn tagged(effects: &[Effect]) -> Vec<(u64, RunRequest)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(ClientMsg::RunTagged { id, request }) => Some((*id, request.clone())),
            _ => None,
        })
        .collect()
}

pub(super) fn profile_requests(effects: &[Effect]) -> Vec<ProfileRequest> {
    tagged(effects)
        .into_iter()
        .filter_map(|(_, r)| match r {
            RunRequest::Profile(p) => Some(p),
            _ => None,
        })
        .collect()
}

fn statuses(effects: &[Effect]) -> usize {
    profile_requests(effects)
        .iter()
        .filter(|r| matches!(r, ProfileRequest::Status { .. }))
        .count()
}

pub(super) fn screen(app: &App) -> &ProfileScreen {
    match &app.screen {
        Some(Screen::Profile(s)) => s,
        other => panic!("no profile screen: {:?}", other.is_some()),
    }
}

fn screen_mut(app: &mut App) -> &mut ProfileScreen {
    match &mut app.screen {
        Some(Screen::Profile(s)) => s,
        _ => panic!("no profile screen"),
    }
}

pub(super) fn reply(app: &mut App, id: u64, reply: ProfileReply) -> Vec<Effect> {
    app.on_daemon(DaemonMsg::Run(RunReply::Profile {
        reply: Box::new(reply),
        request_id: Some(id),
    }))
}

fn record(state: ProposalState) -> ProposalRecord {
    ProposalRecord {
        project: dir(),
        state,
        origin: ProposalOrigin::Detect,
        started_at: 0,
        updated_at: 0,
        base_sha: String::new(),
        scout_id: None,
        window_id: None,
        profile: None,
        verification: None,
        dropped: vec![],
        proposed: None,
        trusted_project: vec![],
        unconfined_checks: false,
        auto_confirm: false,
    }
}

pub(super) fn status(state: Option<ProposalState>) -> ProfileReply {
    ProfileReply::Status(ProfileStatus {
        project: dir(),
        repo_dir: "/data/repos/shop".into(),
        source: ProfileSource::Stored,
        confirmed_at: Some(0),
        stale: vec![],
        unparseable: None,
        proposal: state.map(record),
        scout: None,
        verify_confined: true,
    })
}

pub(super) fn stored_profile() -> RepoProfile {
    RepoProfile {
        check: Some("cargo test".into()),
        setup: Some("make".into()),
        source: vec!["src/**".into(), "lib/**".into()],
        full_shards: Some(2),
        module_names: Some(ModuleNames::Dir),
        env: BTreeMap::from([("RUST_LOG".into(), "debug".into())]),
        ..RepoProfile::default()
    }
}

/// `show_text`'s shape: the TOML, then a comment block.
pub(super) fn shown(profile: &RepoProfile, dropped: Vec<DroppedCommand>) -> ProfileReply {
    let toml = format!(
        "{}\n# protected: built-in .git/** + no extras\n",
        toml::to_string(profile).unwrap()
    );
    ProfileReply::Shown {
        source: ProfileSource::Stored,
        toml,
        meta: None,
        verification: None,
        dropped,
    }
}

pub(super) fn open_app() -> App {
    app_with(vec![project_win(1, DIR)])
}

/// `C-b P` on `DIR`; the three ids it sent: status, stored, proposal.
pub(super) fn open(app: &mut App) -> [u64; 3] {
    prefix(app);
    let effects = tap(app, KeyCode::Char('P'));
    let ids: Vec<u64> = tagged(&effects).into_iter().map(|(id, _)| id).collect();
    [ids[0], ids[1], ids[2]]
}

/// The screen open with a stored profile and a ready proposal.
pub(super) fn ready_app() -> (App, [u64; 3]) {
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(Some(ProposalState::Ready)));
    reply(&mut app, ids[1], shown(&stored_profile(), vec![]));
    let mut proposal = stored_profile();
    proposal.check = Some("cargo test --workspace".into());
    reply(&mut app, ids[2], shown(&proposal, vec![]));
    (app, ids)
}

/// The Profile tab with `key` selected.
pub(super) fn select(app: &mut App, key: &str) {
    if screen(app).tab != ProfileTab::Profile {
        tap(app, KeyCode::Tab);
    }
    for _ in 0..6 {
        tap(app, KeyCode::PageUp);
    }
    for _ in 0..60 {
        let s = screen(app);
        if s.rows().get(s.selected).is_some_and(|r| r.key == key) {
            return;
        }
        tap(app, KeyCode::Char('j'));
    }
    panic!("no row {key}");
}

#[test]
fn c_b_p_opens_on_the_selected_project_and_asks_three_things() {
    let mut app = open_app();
    prefix(&mut app);
    let effects = tap(&mut app, KeyCode::Char('P'));
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
    assert_eq!(tagged(&effects).len(), 3, "every request is tagged");
    let s = screen(&app);
    assert_eq!((s.dir.clone(), s.tab), (dir(), ProfileTab::Status));
    assert!(app.keymap.screen_mode());
    // Bare keys go to the screen, never to the PTY.
    assert!(tap(&mut app, KeyCode::Char('q')).is_empty());
    // A refused show is that side's absence, shown, not toasted.
    let ids: Vec<u64> = tagged(&effects).into_iter().map(|(id, _)| id).collect();
    let no = |m: &str| ProfileReply::Refused { message: m.into() };
    reply(&mut app, ids[0], status(None));
    reply(&mut app, ids[1], no("no stored profile for /p/shop"));
    reply(&mut app, ids[2], no("no proposal for /p/shop"));
    assert!(matches!(screen(&app).stored, Side::Absent(_)));
    assert!(matches!(screen(&app).proposal, Side::Absent(_)));
    assert_eq!(app.toast_text(), None);
    assert!(app.replies.is_empty());
    // Esc leaves, and the keymap with it.
    tap(&mut app, KeyCode::Esc);
    assert!(app.screen.is_none());
    assert!(!app.keymap.screen_mode());
}

/// The selected project wins over the focused window's (decision 34).
#[test]
fn a_selected_project_is_the_screens_project() {
    let mut app = app_with(vec![project_win(1, "/p/other"), project_win(2, DIR)]);
    app.tree.selected = Some(crate::tree::NodeKey::Project(dir()));
    open(&mut app);
    assert_eq!(screen(&app).dir, dir());
}

#[test]
fn no_project_toasts() {
    let mut app = app_with(vec![]);
    prefix(&mut app);
    assert!(tap(&mut app, KeyCode::Char('P')).is_empty());
    assert!(app.screen.is_none());
    assert_eq!(
        app.toast_text(),
        Some("select a Git project to see its profile")
    );
    // Over the plan review it asks to leave the review first.
    let mut app = open_app();
    app.open_plan_review("r1".into(), crate::app::ReviewTarget::Gate);
    prefix(&mut app);
    assert!(tap(&mut app, KeyCode::Char('P')).is_empty());
    assert!(app.screen.is_none());
    assert_eq!(app.toast_text(), Some("leave the plan review first (esc)"));
}

/// Decision 34: one `Status` a second while the detection runs, never two in flight;
/// none after `Esc`; none once `Ready`, which fetches the proposal once.
#[test]
fn a_running_detection_polls_once_a_second() {
    let mut app = open_app();
    let ids = open(&mut app);
    let t0 = Instant::now();
    let at = |tenths: u64| t0 + Duration::from_millis(100 * tenths);
    // Nothing runs yet: no poll.
    assert_eq!(statuses(&app.screens_tick(at(30))), 0);
    reply(&mut app, ids[0], status(Some(ProposalState::Scouting)));
    let mut sent = Vec::new();
    for tick in 1..=10 {
        let effects = app.screens_tick(at(tick));
        if statuses(&effects) > 0 {
            sent.push((tick, tagged(&effects)[0].0));
        }
    }
    assert_eq!(sent.len(), 1, "one per second: {sent:?}");
    assert_eq!(sent[0].0, 10);
    // Unanswered: three more seconds send nothing.
    for tick in 11..=40 {
        assert_eq!(
            statuses(&app.screens_tick(at(tick))),
            0,
            "in flight at {tick}"
        );
    }
    // Answered, still running: the next one goes.
    reply(&mut app, sent[0].1, status(Some(ProposalState::Verifying)));
    let effects = app.screens_tick(at(41));
    assert_eq!(statuses(&effects), 1);
    let id = tagged(&effects)[0].0;
    // Ready: the proposal is fetched once, and the polling stops.
    let effects = reply(&mut app, id, status(Some(ProposalState::Ready)));
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Show {
            dir: dir(),
            proposed: true
        }]
    );
    for tick in 42..=80 {
        assert_eq!(statuses(&app.screens_tick(at(tick))), 0, "ready at {tick}");
    }
    // A proposal already ready at open was asked for by the open: nothing more.
    let mut app = open_app();
    let ids = open(&mut app);
    assert!(reply(&mut app, ids[0], status(Some(ProposalState::Ready))).is_empty());
    // Esc stops it too.
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(Some(ProposalState::Preparing)));
    tap(&mut app, KeyCode::Esc);
    for tick in 1..=30 {
        assert!(app.screens_tick(at(tick)).is_empty());
    }
    // `on_tick` drives it: a Status once a second has passed since the last.
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(Some(ProposalState::Scouting)));
    assert_eq!(statuses(&app.on_tick()), 0);
    screen_mut(&mut app).status_sent_at = Some(Instant::now() - Duration::from_secs(2));
    assert_eq!(statuses(&app.on_tick()), 1);
}

/// Decision 35: `d` opens the toggles, both off; the page's `y` sends `Detect`.
#[test]
fn detect_asks_its_toggles_then_sends() {
    let (mut app, _) = ready_app();
    assert!(tap(&mut app, KeyCode::Char('d')).is_empty());
    assert_eq!(
        screen(&app).page,
        Some(ProfilePage::Detect {
            trust_project: false,
            unconfined_checks: false,
            focus: 0
        })
    );
    let effects = tap(&mut app, KeyCode::Char('y'));
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Detect {
            dir: dir(),
            trust_project: false,
            unconfined_checks: false
        }]
    );
    assert_eq!(screen(&app).page, None);
    // Space toggles the focused one; Tab moves; Esc sends nothing.
    tap(&mut app, KeyCode::Char('d'));
    tap(&mut app, KeyCode::Char(' '));
    tap(&mut app, KeyCode::Tab);
    tap(&mut app, KeyCode::Char(' '));
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(screen(&app).page, None);
    assert!(app.screen.is_some(), "esc closes the page, not the screen");
    tap(&mut app, KeyCode::Char('d'));
    tap(&mut app, KeyCode::Char(' '));
    tap(&mut app, KeyCode::Down);
    tap(&mut app, KeyCode::Char(' '));
    let effects = tap(&mut app, KeyCode::Char('y'));
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Detect {
            dir: dir(),
            trust_project: true,
            unconfined_checks: true
        }]
    );
    // Its Done starts the polling with a Status at once.
    let id = tagged(&effects)[0].0;
    let effects = reply(
        &mut app,
        id,
        ProfileReply::Done {
            message: "detection started".into(),
        },
    );
    assert_eq!(statuses(&effects), 1);
    assert_eq!(screen(&app).message.as_deref(), Some("detection started"));
}

/// Decision 35: `c` shows `Shown.toml` exactly, and only `y` sends it back.
#[test]
fn c_on_a_ready_proposal_shows_the_exact_toml_and_y_confirms_it() {
    let mut app = open_app();
    let ids = open(&mut app);
    // Not ready: nothing to confirm.
    tap(&mut app, KeyCode::Char('c'));
    assert_eq!(screen(&app).page, None);
    reply(&mut app, ids[0], status(Some(ProposalState::Ready)));
    let toml = "check = \"cargo test\"  \n\n# protected: built-in .git/** + no extras\n# \
                verification 2026-10-01 10:00 (confined)\n";
    reply(
        &mut app,
        ids[2],
        ProfileReply::Shown {
            source: ProfileSource::None,
            toml: toml.into(),
            meta: None,
            verification: None,
            dropped: vec![],
        },
    );
    tap(&mut app, KeyCode::Char('c'));
    assert!(matches!(
        &screen(&app).page,
        Some(ProfilePage::Confirm { toml: t, .. }) if t == toml
    ));
    assert!(tap(&mut app, KeyCode::Enter).is_empty(), "y only");
    assert_eq!(app.toast_text(), Some("press y to confirm"));
    let effects = tap(&mut app, KeyCode::Char('y'));
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Confirm {
            dir: dir(),
            shown: Some(toml.into())
        }]
    );
}

#[test]
fn x_rejects_the_proposal_after_y() {
    let (mut app, _) = ready_app();
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert_eq!(screen(&app).page, Some(ProfilePage::Reject));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("press y to reject proposal"));
    let effects = tap(&mut app, KeyCode::Char('y'));
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Reject { dir: dir() }]
    );
}

#[test]
fn p_switches_stored_and_proposal() {
    let (mut app, _) = ready_app();
    tap(&mut app, KeyCode::Tab);
    assert_eq!(screen(&app).tab, ProfileTab::Profile);
    assert!(!screen(&app).proposed);
    let check_mark = |app: &App| {
        let rows = screen(app).rows();
        rows.iter().find(|r| r.key == "check").unwrap().mark
    };
    assert_eq!(check_mark(&app), None);
    tap(&mut app, KeyCode::Char('p'));
    assert!(screen(&app).proposed);
    assert_eq!(check_mark(&app), Some(crate::profile_view::Mark::Changed));
    tap(&mut app, KeyCode::Char('p'));
    assert!(!screen(&app).proposed);
    tap(&mut app, KeyCode::BackTab);
    assert_eq!(screen(&app).tab, ProfileTab::Status);
}

/// Preflight F26: Enter on a proposal alert opens this screen on the proposal.
#[test]
fn enter_on_a_proposal_alert_opens_the_profile_screen() {
    let mut app = super::alerts::every_app();
    let key = AlertKey::Proposal("/r/shop".into());
    prefix(&mut app);
    tap(&mut app, KeyCode::Char('a'));
    let mut effects = vec![];
    for _ in 0..20 {
        let on = app.alerts_focus.as_ref().and_then(|f| f.selected.clone());
        if on.as_ref() == Some(&key) {
            effects = tap(&mut app, KeyCode::Enter);
            break;
        }
        tap(&mut app, KeyCode::Char('j'));
    }
    let shop = PathBuf::from("/r/shop");
    assert_eq!(
        profile_requests(&effects),
        vec![
            ProfileRequest::Status { dir: shop.clone() },
            ProfileRequest::Show {
                dir: shop.clone(),
                proposed: false
            },
            ProfileRequest::Show {
                dir: shop.clone(),
                proposed: true
            },
        ]
    );
    let s = screen(&app);
    assert_eq!(s.dir, shop);
    assert_eq!(s.tab, ProfileTab::Profile);
    assert!(s.proposed);
    assert_eq!(app.alerts_focus, None);
    assert_eq!(app.toast_text(), None);
}

/// Enter on a verified command expands its output tail; a reply for a closed screen
/// changes nothing.
#[test]
fn enter_expands_a_checks_tail_and_a_late_reply_is_harmless() {
    let mut app = open_app();
    let ids = open(&mut app);
    let mut reply_shown = shown(&stored_profile(), vec![]);
    if let ProfileReply::Shown { verification, .. } = &mut reply_shown {
        *verification = Some(verification_of("cargo test"));
    }
    reply(&mut app, ids[1], reply_shown);
    select(&mut app, "check");
    tap(&mut app, KeyCode::Enter);
    assert_eq!(screen(&app).expanded.as_deref(), Some("check"));
    tap(&mut app, KeyCode::Enter);
    assert_eq!(screen(&app).expanded, None);
    tap(&mut app, KeyCode::Esc);
    assert!(reply(&mut app, ids[2], shown(&stored_profile(), vec![])).is_empty());
    assert!(app.screen.is_none());
}

/// A check record for `command`, passed in 4 s, with a two-line tail.
pub(super) fn verification_of(command: &str) -> proto::ProfileVerification {
    proto::ProfileVerification {
        at: 0,
        confined: true,
        setup: None,
        check: Some(proto::CommandCheck {
            command: command.into(),
            ok: true,
            code: Some(0),
            timed_out: false,
            secs: 4,
            tail: "test a ... ok\ntest result: ok".into(),
        }),
        single_test: None,
        build_check: None,
        module_graph: None,
        module_test: None,
        module_tests: None,
        toolchain_id: None,
    }
}
