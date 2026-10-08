//! Milestone 9.8 decision 39: the task edit form's model is chosen from the CLIs'
//! models (the picker, `role table` first) and its effort cycles the chosen model's
//! catalog efforts, over the fixture catalogs.

use super::tests::edit_fixture_task;
use super::{EditField, TaskEditForm};
use crate::app::model_picker::tests::{fixture_catalogs, mref, selected_model};
use crate::app::model_picker::{PickerEntry, PickerFor};
use crate::app::{App, Effect, Modal};
use crate::tree::run_fixtures::RUN_ID;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{ClientMsg, Effort, PlanEdit, RouteSpec, RunRequest, Runtime};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// The app with the fixture catalogs and the edit form open on `t1` (route spec
/// `{ claude, policy, policy, medium }`).
fn edit_app() -> App {
    let mut app = App::new(vec![], "/tmp".into(), Default::default());
    app.catalogs = fixture_catalogs();
    let form = TaskEditForm::new(RUN_ID, &edit_fixture_task());
    app.modal = Some(Modal::EditTask(Box::new(form)));
    app
}

fn form(app: &App) -> &TaskEditForm {
    match &app.modal {
        Some(Modal::EditTask(form)) => form,
        other => panic!("no edit form: {other:?}"),
    }
}

/// The route the form's `Edit` amends `t1` to.
fn sent_route(effects: Vec<Effect>) -> Option<RouteSpec> {
    let edits = effects
        .into_iter()
        .find_map(|e| match e {
            Effect::Send(ClientMsg::RunTagged {
                request: RunRequest::Edit { edits, .. },
                ..
            }) => Some(edits),
            _ => None,
        })
        .expect("an Edit");
    match &edits[..] {
        [PlanEdit::AmendTask { route, .. }] => route.clone(),
        other => panic!("{other:?}"),
    }
}

fn effort_shown(app: &App) -> String {
    form(app).value_parts(EditField::Effort).0
}

#[test]
fn the_task_edit_form_picks_a_model_and_its_efforts() {
    let mut app = edit_app();
    assert_eq!(form(&app).focus, EditField::Model);
    assert!(
        app.on_key(key(KeyCode::Enter)).is_empty(),
        "⏎ opens, sends nothing"
    );
    let picker = form(&app).picker.as_ref().expect("the picker");
    assert_eq!(picker.target, PickerFor::TaskEdit);
    assert_eq!(
        picker.entries[0],
        PickerEntry::RoleTable("role table".into())
    );
    assert_eq!(picker.selected, 0, "a policy model: on the role table");
    let luna = mref("codex:gpt-6-luna");
    for _ in 0..10 {
        if selected_model(form(&app).picker.as_ref().unwrap()) == Some(luna.clone()) {
            break;
        }
        app.on_key(key(KeyCode::Char('j')));
    }
    app.on_key(key(KeyCode::Enter));
    assert!(form(&app).picker.is_none());
    assert_eq!(
        form(&app).value_parts(EditField::Model).0,
        "‹ Codex · gpt-6 luna ›"
    );
    app.on_key(key(KeyCode::Tab));
    assert_eq!(form(&app).focus, EditField::Effort);
    assert_eq!(
        effort_shown(&app),
        "‹ role table ›",
        "a new model: its row's effort"
    );
    let mut seen = Vec::new();
    for _ in 0..5 {
        app.on_key(key(KeyCode::Char(' ')));
        seen.push(effort_shown(&app));
    }
    assert_eq!(
        seen,
        ["low", "medium", "high", "xhigh", "role table"].map(|e| format!("‹ {e} ›"))
    );
    app.on_key(key(KeyCode::Left));
    assert_eq!(
        sent_route(app.on_key(key(KeyCode::Enter))),
        Some(RouteSpec {
            runtime: Some(Runtime::Codex),
            model: Some("gpt-6-luna".into()),
            strength: None,
            effort: Some(Effort::new("xhigh")),
        })
    );

    // `role table`: the plan's route spec cleared, every field the table's.
    let mut app = edit_app();
    app.on_key(key(KeyCode::Enter));
    app.on_key(key(KeyCode::Enter));
    assert_eq!(form(&app).value_parts(EditField::Model).0, "‹ role table ›");
    // On the model row `⏎` is the picker's; from the next row it saves.
    app.on_key(key(KeyCode::Tab));
    assert_eq!(
        sent_route(app.on_key(key(KeyCode::Enter))),
        Some(RouteSpec::default())
    );
}

/// A route the plan names opens the picker on its model; `esc` closes the picker and
/// keeps the form; a paste while it is open goes nowhere but its custom name.
#[test]
fn the_edit_forms_picker_opens_on_the_plans_model_and_cancels() {
    let mut task = edit_fixture_task();
    task.route_spec = RouteSpec {
        runtime: Some(Runtime::Codex),
        model: Some("gpt-6-sol".into()),
        strength: None,
        effort: Some(Effort::new("max")),
    };
    let mut app = edit_app();
    app.modal = Some(Modal::EditTask(Box::new(TaskEditForm::new(RUN_ID, &task))));
    app.on_key(key(KeyCode::Enter));
    let picker = form(&app).picker.as_ref().unwrap();
    assert_eq!(selected_model(picker), Some(mref("codex:gpt-6-sol")));
    app.on_key(key(KeyCode::Esc));
    assert!(
        matches!(app.modal, Some(Modal::EditTask(_))),
        "esc closes the picker only"
    );
    assert!(form(&app).picker.is_none());
    // Its efforts are the catalog's.
    app.on_key(key(KeyCode::Tab));
    app.on_key(key(KeyCode::Char(' ')));
    assert_eq!(effort_shown(&app), "‹ role table ›", "max, then the row's");
    app.on_key(key(KeyCode::Char(' ')));
    assert_eq!(effort_shown(&app), "‹ low ›");
}
