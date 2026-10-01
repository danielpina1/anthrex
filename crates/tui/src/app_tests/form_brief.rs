//! Milestone 9.0.6 final review I2: the answer and message forms say why Enter did not
//! go on, and the answer form's brief never stays `loading…`: a brief that was not
//! sent, got no reply or lost its link says so, and a reconnect asks again while the
//! form is open.

use super::action_forms::{answer_form, detail, form, forms_app, open, task_t1, type_text};
use super::actions::{sent_tagged_id, tap};
use super::actions_replies::cell_fg;
use super::*;
use crate::app::actions::ActionStep;
use crate::app::actions::forms::{ActionForm, Brief};
use crate::theme::{Role, role};
use crate::tree::run_fixtures::RUN_ID;
use proto::{ActionKind, RunRequest};
use std::time::{Duration, Instant};

fn drawn(app: &App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..24)
        .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
        .collect()
}

fn brief_request() -> RunRequest {
    RunRequest::TaskDetail {
        run_id: RUN_ID.into(),
        task_id: "t1".into(),
    }
}

/// The answer form open on `t1`; the id of its `TaskDetail`.
fn answer_open(app: &mut App) -> u64 {
    open(app, task_t1(), ActionKind::Answer);
    sent_tagged_id(&tap(app, KeyCode::Enter), |r| *r == brief_request())
}

/// Enter on an empty answer or message stays on the form and says why, in `Failed`,
/// as the override's `type a reason first` does; typing takes the line away.
#[test]
fn an_empty_answer_or_message_says_why() {
    for (kind, why) in [
        (ActionKind::Answer, "type an answer first"),
        (ActionKind::Message, "type a message first"),
    ] {
        let mut app = forms_app(false);
        open(&mut app, task_t1(), kind.clone());
        tap(&mut app, KeyCode::Enter);
        assert!(!drawn(&app).contains(why), "{kind:?} before Enter");
        type_text(&mut app, "  ");
        assert!(tap(&mut app, KeyCode::Enter).is_empty());
        assert!(matches!(flow_step(&app), ActionStep::Form(_)));
        assert_eq!(form(&app).input().err(), Some(why));
        assert_eq!(
            cell_fg(&app, why),
            role(Role::Failed, app.palette()).fg,
            "{kind:?}"
        );
        type_text(&mut app, "x");
        assert!(!drawn(&app).contains(why), "{kind:?}: typing clears it");
    }
}

fn flow_step(app: &App) -> &ActionStep {
    &super::actions::flow(app).step
}

/// A refused send of the brief's `TaskDetail` fails the brief at once, with no toast:
/// the form shows it.
#[test]
fn a_brief_not_sent_says_so() {
    let mut app = forms_app(false);
    let id = answer_open(&mut app);
    app.on_send_failed(&ClientMsg::RunTagged {
        id,
        request: brief_request(),
    });
    assert!(!app.replies.contains(id));
    assert_eq!(
        answer_form(&app).brief,
        Brief::Failed("not sent: daemon is not responding".into())
    );
    assert_eq!(app.toast_text(), None);
}

/// A brief with no reply in time fails with `no reply from daemon` on the form, and the
/// form's own expiry is not toasted as well.
#[test]
fn an_expired_brief_says_so_on_the_form() {
    let mut app = forms_app(false);
    let id = answer_open(&mut app);
    app.set_reply_sent_at(id, Instant::now() - Duration::from_secs(31));
    app.on_tick();
    assert!(!app.replies.contains(id));
    assert_eq!(
        answer_form(&app).brief,
        Brief::Failed("no reply from daemon".into())
    );
    assert_eq!(app.toast_text(), None);
    // Its late reply still fills the brief (decision 16).
    app.on_run_reply(detail("late brief", id));
    assert_eq!(
        answer_form(&app).brief,
        Brief::Ready(vec!["late brief".into()])
    );
}

/// A lost link fails a loading brief with `not connected`; the reconnect asks for it
/// once more while the form is open, and its reply fills the brief.
#[test]
fn a_lost_link_fails_the_brief_and_a_reconnect_asks_again() {
    let mut app = forms_app(false);
    answer_open(&mut app);
    type_text(&mut app, "kept");
    app.on_link_lost("gone");
    assert_eq!(
        answer_form(&app).brief,
        Brief::Failed("not connected".into())
    );
    let windows = app.windows.clone();
    let effects = app.on_reconnected(windows);
    let briefs: Vec<u64> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(ClientMsg::RunTagged { id, request }) if *request == brief_request() => {
                Some(*id)
            }
            _ => None,
        })
        .collect();
    assert_eq!(briefs.len(), 1, "{effects:?}");
    let id = briefs[0];
    assert_eq!(answer_form(&app).brief, Brief::Loading);
    assert_eq!(answer_form(&app).text.text(), "kept");
    app.on_run_reply(detail("again", id));
    assert_eq!(answer_form(&app).brief, Brief::Ready(vec!["again".into()]));
    // A ready brief is not asked for again by the next reconnect.
    app.on_link_lost("gone");
    let windows = app.windows.clone();
    let effects = app.on_reconnected(windows);
    assert!(
        !effects.iter().any(|e| matches!(
            e,
            Effect::Send(ClientMsg::RunTagged { request, .. }) if *request == brief_request()
        )),
        "{effects:?}"
    );
    assert!(matches!(form(&app), ActionForm::Answer(_)));
}
