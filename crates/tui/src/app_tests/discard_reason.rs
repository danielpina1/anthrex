//! Milestone 9.15 decision 40: a run's Discard asks for an optional reason on a form
//! before its confirmation page, and sends it as `Finish.reason` (decision 26).

use super::action_forms::{form, type_text};
use super::actions::{flow, sent_tagged_id, tap};
use super::actions_accept::app_with_complete_run;
use super::*;
use crate::actions_request::ActionTarget;
use crate::app::actions::ActionStep;
use crate::app::actions::forms::ActionForm;
use proto::{ActionKind, FinishAction, RunRequest};

const RUN: &str = "add-mul-0723";

/// The complete run's menu, opened on Discard, with Enter pressed on that entry.
fn discard_form() -> App {
    let mut app = app_with_complete_run(RUN);
    app.open_actions((RUN.into(), ActionTarget::Run), Some(ActionKind::Discard));
    tap(&mut app, KeyCode::Enter);
    app
}

fn page_rows(app: &App) -> Vec<(String, String)> {
    match &flow(app).step {
        ActionStep::Confirm(page) => page.details.clone(),
        other => panic!("not a page: {other:?}"),
    }
}

/// The one `Finish` the page's `y` sent, with its reason.
fn sent_reason(app: &mut App) -> Option<String> {
    let effects = tap(app, KeyCode::Char('y'));
    let reason = std::cell::RefCell::new(None);
    sent_tagged_id(&effects, |r| match r {
        RunRequest::Finish {
            run_id,
            action: FinishAction::Discard,
            confirm: Some(c),
            reason: sent,
        } if run_id == RUN && c == RUN => {
            *reason.borrow_mut() = sent.clone();
            true
        }
        _ => false,
    });
    reason.into_inner()
}

#[test]
fn discard_opens_a_reason_form_then_the_confirmation() {
    let mut app = discard_form();
    let ActionForm::DiscardReason(f) = form(&app) else {
        panic!("not the discard form: {:?}", form(&app));
    };
    assert_eq!(f.info.kind, ActionKind::Discard);
    type_text(&mut app, "too big");
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    let rows = page_rows(&app);
    assert_eq!(
        rows[0].0, "removes",
        "the discard's own rows first: {rows:?}"
    );
    assert_eq!(
        rows.last(),
        Some(&("reason".to_string(), "too big".to_string()))
    );
    assert_eq!(sent_reason(&mut app), Some("too big".into()));
}

#[test]
fn a_blank_reason_is_allowed_and_sent_as_none() {
    let mut app = discard_form();
    type_text(&mut app, "   ");
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(
        page_rows(&app).last(),
        Some(&("reason".to_string(), "(none)".to_string()))
    );
    assert_eq!(sent_reason(&mut app), None);
}

#[test]
fn esc_on_the_form_backs_out() {
    let mut app = discard_form();
    assert!(matches!(form(&app), ActionForm::DiscardReason(_)));
    type_text(&mut app, "x");
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(flow(&app).step, ActionStep::Menu);
    // Esc on the page goes back to the form, with what was typed.
    tap(&mut app, KeyCode::Enter);
    type_text(&mut app, "kept");
    tap(&mut app, KeyCode::Enter);
    assert!(matches!(flow(&app).step, ActionStep::Confirm(_)));
    tap(&mut app, KeyCode::Esc);
    let ActionForm::DiscardReason(f) = form(&app) else {
        panic!("back on the form");
    };
    assert_eq!(f.reason.text(), "kept");
}
