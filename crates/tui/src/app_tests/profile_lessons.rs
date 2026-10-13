//! Milestone 9.15 decisions 35, 36 and 38 (task M9.15.12): the Profile screen's Lessons
//! page. `l` opens it and lists; `a` approves at once; `r` and `d` ask `y/n` on the
//! page's bottom line and send on `y`; `t` lists rejected and retired lessons too; a
//! refusal shows on the page in the daemon's words; after any reply the page lists
//! again. No test sleeps: replies are delivered by id.

use super::*;
use crate::app::profile_screen::{LessonsPage, ProfilePage};
use proto::{
    LessonInfo, LessonLayer, LessonOrigin, LessonStatus, LessonsRequest, RunReply, RunRequest,
};

/// A lesson with `id`, `status` and `title`.
pub(super) fn lesson(id: &str, status: LessonStatus, title: &str) -> LessonInfo {
    LessonInfo {
        id: id.into(),
        layer: LessonLayer::Semantic,
        status,
        origin: LessonOrigin::Distilled,
        title: title.into(),
        body: format!("the body of {id}"),
        files: vec!["crates/a/**".into()],
        created_at: 0,
        last_seen: 0,
        runs: vec![],
        green: 0,
        outcomes: 0,
        reconfirmed: 0,
        headlines: vec![],
        retired_reason: None,
    }
}

fn three() -> Vec<LessonInfo> {
    vec![
        lesson("l-aaaaaa01", LessonStatus::Proposed, "first"),
        lesson("l-bbbbbb02", LessonStatus::OnTrial, "second"),
        lesson("kf-cccccc03", LessonStatus::Known, "third"),
    ]
}

/// Every lessons request in `effects`, with its id.
fn lessons_requests(effects: &[Effect]) -> Vec<(u64, LessonsRequest)> {
    tagged(effects)
        .into_iter()
        .filter_map(|(id, r)| match r {
            RunRequest::Lessons(l) => Some((id, l)),
            _ => None,
        })
        .collect()
}

fn list(all: bool) -> LessonsRequest {
    LessonsRequest::List { dir: dir(), all }
}

fn page(app: &App) -> &LessonsPage {
    match &screen(app).page {
        Some(ProfilePage::Lessons(page)) => page,
        other => panic!("no lessons page: {other:?}"),
    }
}

fn lessons_reply(app: &mut App, id: u64, lessons: Vec<LessonInfo>) -> Vec<Effect> {
    app.on_daemon(DaemonMsg::Run(RunReply::Lessons {
        lessons,
        message: None,
        request_id: Some(id),
    }))
}

fn done(app: &mut App, id: u64, message: &str) -> Vec<Effect> {
    app.on_daemon(DaemonMsg::Run(RunReply::Done {
        request: proto::run_wire::request::LESSONS.into(),
        message: message.into(),
        request_id: Some(id),
    }))
}

fn refused(app: &mut App, id: u64, message: &str) -> Vec<Effect> {
    app.on_daemon(DaemonMsg::Run(RunReply::Refused {
        request: proto::run_wire::request::LESSONS.into(),
        message: message.into(),
        request_id: Some(id),
    }))
}

/// The screen open on its stored profile, `l` pressed and the list answered with
/// [`three`].
fn page_app() -> App {
    let (mut app, _) = stored_app();
    let sent = lessons_requests(&tap(&mut app, KeyCode::Char('l')));
    let (id, _) = sent[0];
    lessons_reply(&mut app, id, three());
    app
}

/// The only lessons request in `effects`.
fn one(effects: &[Effect]) -> (u64, LessonsRequest) {
    let sent = lessons_requests(effects);
    assert_eq!(sent.len(), 1, "{sent:?}");
    sent[0].clone()
}

#[test]
fn l_opens_the_page_and_lists() {
    let (mut app, _) = stored_app();
    let effects = tap(&mut app, KeyCode::Char('l'));
    let (id, request) = one(&effects);
    assert_eq!(request, list(false));
    assert_eq!(page(&app).lessons, None, "loading until the list comes");
    lessons_reply(&mut app, id, three());
    let p = page(&app);
    assert_eq!(p.lessons.as_ref().map(Vec::len), Some(3));
    assert_eq!(p.selected, 0);
    assert!(!app.replies.contains(id));
    // `j` and `k` move within the list, and stop at its ends.
    for _ in 0..5 {
        tap(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(page(&app).selected, 2);
    tap(&mut app, KeyCode::Up);
    assert_eq!(page(&app).selected, 1);
    // Esc goes back to the profile, which keeps the screen.
    tap(&mut app, KeyCode::Esc);
    assert!(screen(&app).page.is_none());
    assert_eq!(app.toast_text(), None);
}

/// Decision 36: the screen sends one `List` when it opens, with its other asks.
#[test]
fn the_screen_lists_once_when_it_opens() {
    let mut app = open_app();
    prefix(&mut app);
    let effects = tap(&mut app, KeyCode::Char('P'));
    let (id, request) = one(&effects);
    assert_eq!(request, list(false));
    lessons_reply(&mut app, id, three());
    assert_eq!(screen(&app).to_review, 1, "one proposed lesson");
    assert!(screen(&app).page.is_none(), "the page stays closed");
}

#[test]
fn a_sends_decide_approve_at_once() {
    let mut app = page_app();
    let effects = tap(&mut app, KeyCode::Char('a'));
    let (_, request) = one(&effects);
    assert_eq!(
        request,
        LessonsRequest::Decide {
            dir: dir(),
            ids: vec!["l-aaaaaa01".into()],
            approve: true,
        }
    );
    assert_eq!(page(&app).question, None, "no question for approve");
}

#[test]
fn r_asks_then_sends_reject_on_y() {
    let mut app = page_app();
    tap(&mut app, KeyCode::Char('j'));
    assert!(tagged(&tap(&mut app, KeyCode::Char('r'))).is_empty());
    let question = page(&app).question.as_ref().expect("a question");
    assert_eq!(question.text(), "reject l-bbbbbb02? y/n");
    // Other keys do nothing while it asks.
    assert!(tagged(&tap(&mut app, KeyCode::Char('a'))).is_empty());
    assert!(page(&app).question.is_some());
    let (_, request) = one(&tap(&mut app, KeyCode::Char('y')));
    assert_eq!(
        request,
        LessonsRequest::Decide {
            dir: dir(),
            ids: vec!["l-bbbbbb02".into()],
            approve: false,
        }
    );
    assert_eq!(page(&app).question, None);
}

#[test]
fn d_asks_then_sends_forget_on_y() {
    let mut app = page_app();
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Char('j'));
    assert!(tagged(&tap(&mut app, KeyCode::Char('d'))).is_empty());
    let question = page(&app).question.as_ref().expect("a question");
    assert_eq!(question.text(), "forget kf-cccccc03? y/n");
    let (_, request) = one(&tap(&mut app, KeyCode::Char('y')));
    assert_eq!(
        request,
        LessonsRequest::Forget {
            dir: dir(),
            ids: vec!["kf-cccccc03".into()],
        }
    );
    assert_eq!(page(&app).question, None);
}

#[test]
fn n_or_esc_cancels_the_question() {
    for cancel in [KeyCode::Char('n'), KeyCode::Esc] {
        for ask in ['r', 'd'] {
            let mut app = page_app();
            tap(&mut app, KeyCode::Char(ask));
            assert!(page(&app).question.is_some());
            assert!(
                tagged(&tap(&mut app, cancel)).is_empty(),
                "{cancel:?} sends nothing"
            );
            assert_eq!(page(&app).question, None, "{cancel:?} cancels");
            // Esc cancels the question only; the page stays open.
            assert!(matches!(screen(&app).page, Some(ProfilePage::Lessons(_))));
            assert!(tagged(&tap(&mut app, KeyCode::Char('y'))).is_empty());
        }
    }
}

#[test]
fn t_toggles_all() {
    let mut app = page_app();
    let (id, request) = one(&tap(&mut app, KeyCode::Char('t')));
    assert_eq!(request, list(true));
    assert!(page(&app).all);
    let mut with_gone = three();
    with_gone.push(lesson("l-dddddd04", LessonStatus::Rejected, "gone"));
    lessons_reply(&mut app, id, with_gone);
    assert_eq!(page(&app).lessons.as_ref().map(Vec::len), Some(4));
    let (_, request) = one(&tap(&mut app, KeyCode::Char('t')));
    assert_eq!(request, list(false));
    assert!(!page(&app).all);
}

#[test]
fn a_refusal_is_shown_on_the_page() {
    let mut app = page_app();
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Char('j'));
    let (id, _) = one(&tap(&mut app, KeyCode::Char('a')));
    let words =
        "kf-cccccc03 is known; only a proposed or on-trial lesson can be approved or rejected";
    refused(&mut app, id, words);
    assert_eq!(page(&app).error.as_deref(), Some(words));
    assert_eq!(app.toast_text(), None, "on the page, not toasted");
    assert!(!app.replies.contains(id));
    // A refused list is shown the same way, and not asked again.
    let (id, _) = one(&tap(&mut app, KeyCode::Char('t')));
    let effects = refused(&mut app, id, "not a git repository: /p/shop");
    assert!(lessons_requests(&effects).is_empty(), "no retry loop");
    assert_eq!(
        page(&app).error.as_deref(),
        Some("not a git repository: /p/shop")
    );
}

#[test]
fn the_page_lists_again_after_a_reply() {
    let mut app = page_app();
    let (id, _) = one(&tap(&mut app, KeyCode::Char('a')));
    let (relist, request) = one(&done(&mut app, id, "approved 1"));
    assert_eq!(request, list(false));
    assert_eq!(page(&app).message.as_deref(), Some("approved 1"));
    let mut after = three();
    after[0].status = LessonStatus::Approved;
    lessons_reply(&mut app, relist, after);
    assert_eq!(
        page(&app).lessons.as_ref().unwrap()[0].status,
        LessonStatus::Approved
    );
    assert_eq!(screen(&app).to_review, 0, "the count follows the list");
    // A refusal lists again too.
    tap(&mut app, KeyCode::Char('r'));
    let (id, _) = one(&tap(&mut app, KeyCode::Char('y')));
    let (_, request) = one(&refused(&mut app, id, "l-aaaaaa01 is approved; ..."));
    assert_eq!(request, list(false));
    // A list older than the latest does not fill the page.
    let (stale, _) = one(&tap(&mut app, KeyCode::Char('t')));
    let (_, _) = one(&tap(&mut app, KeyCode::Char('t')));
    lessons_reply(&mut app, stale, vec![]);
    assert_eq!(page(&app).lessons.as_ref().map(Vec::len), Some(3));
}

/// Until the graph screen exists (task M9.15.15), `g` says so; M9.15.15 rewrites this
/// test to assert the `Graph` request.
#[test]
fn g_opens_the_graph_screen() {
    let mut app = page_app();
    assert!(tagged(&tap(&mut app, KeyCode::Char('g'))).is_empty());
    assert_eq!(app.toast_text(), Some("not available yet"));
    assert!(matches!(screen(&app).page, Some(ProfilePage::Lessons(_))));
}

/// A decision whose page closed before its reply is toasted, as the screen's other
/// outcomes are; a reply for a closed screen too.
#[test]
fn an_outcome_after_the_page_closed_is_toasted() {
    let mut app = page_app();
    let (id, _) = one(&tap(&mut app, KeyCode::Char('a')));
    tap(&mut app, KeyCode::Esc);
    done(&mut app, id, "approved 1");
    assert_eq!(app.toast_text(), Some("approved 1"));
    let mut app = page_app();
    let (id, _) = one(&tap(&mut app, KeyCode::Char('a')));
    tap(&mut app, KeyCode::Esc);
    tap(&mut app, KeyCode::Esc);
    assert!(app.screen.is_none());
    refused(&mut app, id, "unknown lesson l-aaaaaa01");
    assert_eq!(app.toast_text(), Some("unknown lesson l-aaaaaa01"));
}

/// The open-time list is a view: its refusal (no repository) is quiet.
#[test]
fn a_refused_open_time_list_is_quiet() {
    let mut app = open_app();
    prefix(&mut app);
    let (id, _) = one(&tap(&mut app, KeyCode::Char('P')));
    let effects = refused(&mut app, id, "not a git repository: /p/shop");
    assert!(lessons_requests(&effects).is_empty());
    assert_eq!(app.toast_text(), None);
    assert_eq!(screen(&app).to_review, 0);
    assert!(!app.replies.contains(id));
}

/// No reply in time: the open page says so instead of loading forever, with no toast.
#[test]
fn an_unanswered_list_shows_on_the_page() {
    let (mut app, _) = stored_app();
    let (id, _) = one(&tap(&mut app, KeyCode::Char('l')));
    let past = std::time::Instant::now() - std::time::Duration::from_secs(31);
    app.set_reply_sent_at(id, past);
    app.expire_replies();
    assert_eq!(page(&app).error.as_deref(), Some("no reply from daemon"));
    assert_eq!(app.toast_text(), None);
}

/// L2 (pinning): proposals surface on the Profile screen and in the CLI only. The
/// alerts of a snapshot are the same whatever the lessons are; `app/alerts.rs` never
/// sees them.
#[test]
fn no_alerts_for_proposals() {
    let mut app = super::super::alerts::every_app();
    let effects = app.open_profile_on("/r/shop".into());
    let (id, _) = one(&effects);
    let before = crate::app::alerts(&app);
    let proposed: Vec<LessonInfo> = (0..5)
        .map(|n| lesson(&format!("l-eeeeee0{n}"), LessonStatus::Proposed, "p"))
        .collect();
    lessons_reply(&mut app, id, proposed);
    assert_eq!(screen(&app).to_review, 5, "the reply was taken");
    assert_eq!(crate::app::alerts(&app), before);
}
