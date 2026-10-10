//! Milestone 9.0.6 task 13, decisions 33-35: the Profile screen's keys, requests,
//! replies and polling. No test sleeps: the poll is driven by `screens_tick(now)` with
//! a moved clock (Global Constraint 11).

use super::*;
use crate::app::profile_screen::{ADVANCED_ROW, ProfilePage, ProfileScreen, Side};
use crate::app::screens::Screen;
use proto::{
    DroppedCommand, ModuleNames, ProfileReply, ProfileRequest, ProfileSource, ProfileStatus,
    ProposalOrigin, ProposalRecord, ProposalState, RepoProfile, RunReply, RunRequest,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

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

pub(super) fn statuses(effects: &[Effect]) -> usize {
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

pub(super) fn screen_mut(app: &mut App) -> &mut ProfileScreen {
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
        edit: None,
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
        queued: Vec::new(),
        checking: None,
        verified_at: None,
        unreadable_text: None,
        dropped_goals: Vec::new(),
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

/// The screen open with a stored profile and no proposal: the profile itself.
pub(super) fn stored_app() -> (App, [u64; 3]) {
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(None));
    reply(&mut app, ids[1], shown(&stored_profile(), vec![]));
    let none = ProfileReply::Refused {
        message: "no proposal for /p/shop".into(),
    };
    reply(&mut app, ids[2], none);
    (app, ids)
}

/// `status(state)` changed by `change`.
pub(super) fn status_with(
    state: Option<ProposalState>,
    change: impl FnOnce(&mut ProfileStatus),
) -> ProfileReply {
    let ProfileReply::Status(mut st) = status(state) else {
        unreachable!()
    };
    change(&mut st);
    ProfileReply::Status(st)
}

/// `key`'s row selected (Advanced opened when the key sits inside it).
pub(super) fn select(app: &mut App, key: &str) {
    for advanced in [false, true] {
        if advanced && !screen(app).advanced {
            tap(app, KeyCode::Char('a'));
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
    }
    panic!("no row {key}");
}

/// The screen's status set to `reply`'s, as a poll would bring it.
pub(super) fn set_status(app: &mut App, reply: ProfileReply) {
    let ProfileReply::Status(st) = reply else {
        unreachable!()
    };
    screen_mut(app).status = Some(st);
}

pub(super) fn keys_of(app: &App) -> Vec<String> {
    screen(app).rows().into_iter().map(|r| r.key).collect()
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
    assert_eq!(s.dir, dir());
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
    // 9.0.7 decision 37: an empty session opens on the start directory, so the toast
    // is for an empty one.
    let mut app = app_with(vec![]);
    app.default_dir = std::path::PathBuf::new();
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

/// Decision 35: `d` opens the toggles, both off; the page's `y` sends `Detect`.
#[test]
fn detect_asks_its_toggles_then_sends() {
    let (mut app, _) = stored_app();
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

/// Decision 29: `o` expands a verified command's output tail; a reply for a closed
/// screen changes nothing.
#[test]
fn o_expands_a_checks_tail_and_a_late_reply_is_harmless() {
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(None));
    let mut reply_shown = shown(&stored_profile(), vec![]);
    if let ProfileReply::Shown { verification, .. } = &mut reply_shown {
        *verification = Some(verification_of("cargo test"));
    }
    reply(&mut app, ids[1], reply_shown);
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('o'));
    assert_eq!(screen(&app).expanded.as_deref(), Some("check"));
    tap(&mut app, KeyCode::Char('o'));
    assert_eq!(screen(&app).expanded, None);
    // A row without a check has no output.
    select(&mut app, "source");
    tap(&mut app, KeyCode::Char('o'));
    assert_eq!(screen(&app).expanded, None);
    tap(&mut app, KeyCode::Esc);
    assert!(reply(&mut app, ids[2], shown(&stored_profile(), vec![])).is_empty());
    assert!(app.screen.is_none());
}

/// Decision 27: Tab, `s`, `c` and `p` are gone: on the profile and on the card they
/// change nothing and send nothing.
#[test]
fn no_tab_no_s_no_c_no_p() {
    for (mut app, _) in [stored_app(), ready_app()] {
        let before = screen(&app).clone();
        for code in [
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Char('s'),
            KeyCode::Char('c'),
            KeyCode::Char('p'),
        ] {
            assert!(tap(&mut app, code).is_empty(), "{code:?}");
            assert_eq!(*screen(&app), before, "{code:?}");
        }
        assert_eq!(app.toast_text(), None);
    }
}

/// Decision 29: `a` opens and folds Advanced; the selection stays on its row, or on the
/// Advanced line when its row folds away.
#[test]
fn a_toggles_advanced_and_keeps_the_selection_on_its_key() {
    let (mut app, _) = stored_app();
    assert!(!screen(&app).advanced);
    assert!(screen(&app).rows().iter().all(|r| !r.advanced), "folded");
    assert!(keys_of(&app).contains(&ADVANCED_ROW.to_string()));
    select(&mut app, "check");
    assert!(tap(&mut app, KeyCode::Char('a')).is_empty());
    assert!(screen(&app).advanced);
    let s = screen(&app);
    assert_eq!(s.rows()[s.selected].key, "check");
    for key in ["full_shards", "module_names", "env.RUST_LOG", "hub"] {
        assert!(keys_of(&app).contains(&key.to_string()), "{key}");
    }
    select(&mut app, "full_shards");
    tap(&mut app, KeyCode::Char('a'));
    assert!(!screen(&app).advanced);
    let s = screen(&app);
    assert_eq!(
        s.rows()[s.selected].key,
        ADVANCED_ROW,
        "its row folded away"
    );
    // The main sections come first, then the Advanced line, then what it holds.
    tap(&mut app, KeyCode::Char('a'));
    let keys = keys_of(&app);
    let at = |k: &str| keys.iter().position(|x| x == k).unwrap();
    assert!(at("setup") < at("check") && at("check") < at("source"));
    assert!(at("delivery.mode") < at(ADVANCED_ROW));
    assert!(at(ADVANCED_ROW) < at("build_check"));
    assert!(
        at("env.RUST_LOG") < at(crate::profile_view::ENV_ADD),
        "the add row last"
    );
}

/// Decision 29: Enter on the Advanced line toggles it.
#[test]
fn enter_on_advanced_toggles_it() {
    let (mut app, _) = stored_app();
    select(&mut app, ADVANCED_ROW);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(screen(&app).advanced);
    assert_eq!(screen(&app).page, None);
    let s = screen(&app);
    assert_eq!(s.rows()[s.selected].key, ADVANCED_ROW);
    tap(&mut app, KeyCode::Enter);
    assert!(!screen(&app).advanced);
}

/// Decision 30: Enter on a row opens its page; Esc closes it.
#[test]
fn enter_on_a_row_opens_its_page() {
    let (mut app, _) = stored_app();
    select(&mut app, "source");
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(
        screen(&app).page,
        Some(ProfilePage::Row {
            key: "source".into(),
            scroll: 0
        })
    );
    tap(&mut app, KeyCode::Esc);
    assert_eq!(screen(&app).page, None);
    assert!(app.screen.is_some());
}

/// Decisions 30 and 32: Enter on an unreadable profile shows the file's text exactly.
#[test]
fn enter_on_an_unreadable_profile_shows_its_text() {
    let text = "check = \"cargo test\n[[broken\n  trailing  \n";
    let mut app = open_app();
    let ids = open(&mut app);
    reply(
        &mut app,
        ids[0],
        status_with(None, |st| {
            st.unparseable = Some("expected `]`".into());
            st.unreadable_text = Some(text.into());
        }),
    );
    let refused = |m: &str| ProfileReply::Refused { message: m.into() };
    reply(&mut app, ids[1], refused("the profile does not parse"));
    reply(&mut app, ids[2], refused("no proposal for /p/shop"));
    assert!(screen(&app).rows().is_empty());
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(
        screen(&app).page,
        Some(ProfilePage::RawText {
            text: text.into(),
            scroll: 0
        })
    );
}

/// Decision 29: `x` only while a review proposal exists; an `Edit`-origin proposal (a
/// row edit of the stored profile) is not one.
#[test]
fn x_is_offered_only_with_a_review_proposal() {
    let (mut app, _) = stored_app();
    assert!(!screen(&app).review_proposal());
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert_eq!(screen(&app).page, None);
    let edit_origin = status_with(Some(ProposalState::Verifying), |st| {
        if let Some(p) = st.proposal.as_mut() {
            p.origin = proto::ProposalOrigin::Edit {
                keys: vec!["check".into()],
            };
        }
    });
    set_status(&mut app, edit_origin);
    assert!(!screen(&app).review_proposal());
    tap(&mut app, KeyCode::Char('x'));
    assert_eq!(screen(&app).page, None);
    // A detection under way is one.
    set_status(&mut app, status(Some(ProposalState::Scouting)));
    assert!(screen(&app).review_proposal());
    tap(&mut app, KeyCode::Char('x'));
    assert_eq!(screen(&app).page, Some(ProfilePage::Discard));
}

/// Decision 30: `d` with no stored profile is `set up`; with one, `detect again`.
#[test]
fn d_without_a_stored_profile_is_set_up() {
    let mut app = open_app();
    let ids = open(&mut app);
    reply(
        &mut app,
        ids[0],
        status_with(None, |st| {
            st.source = ProfileSource::None;
            st.confirmed_at = None;
        }),
    );
    let refused = |m: &str| ProfileReply::Refused { message: m.into() };
    reply(&mut app, ids[1], refused("no stored profile for /p/shop"));
    reply(&mut app, ids[2], refused("no proposal for /p/shop"));
    assert!(screen(&app).rows().is_empty(), "the status line alone");
    tap(&mut app, KeyCode::Char('d'));
    assert!(matches!(
        screen(&app).page,
        Some(ProfilePage::Detect { .. })
    ));
    assert_eq!(screen(&app).detect_title(), "set up");
    let (mut app, _) = stored_app();
    tap(&mut app, KeyCode::Char('d'));
    assert_eq!(screen(&app).detect_title(), "detect again");
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
