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
    // Review minor 8: an opened route with a strength, which a pick drops.
    let mut task = edit_fixture_task();
    task.route_spec.strength = Some("frontier".into());
    let mut app = edit_app();
    app.modal = Some(Modal::EditTask(Box::new(TaskEditForm::new(RUN_ID, &task))));
    assert_eq!(form(&app).focus, EditField::Model);
    assert!(
        app.on_key(key(KeyCode::Enter)).is_empty(),
        "⏎ opens, sends nothing"
    );
    let picker = form(&app).picker.as_ref().expect("the picker");
    assert_eq!(picker.target, PickerFor::TaskEdit);
    assert_eq!(
        picker.entries[0],
        PickerEntry::RoleTable("role table (Claude · Sonnet 5)".into())
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
    // Fix round 1 (controller ruling I2): a picked model with no effort runs at its
    // default, and says so.
    assert_eq!(
        effort_shown(&app),
        "‹ default ›",
        "a new model: its default"
    );
    let mut seen = Vec::new();
    for _ in 0..5 {
        app.on_key(key(KeyCode::Char(' ')));
        seen.push(effort_shown(&app));
    }
    assert_eq!(
        seen,
        ["low", "medium", "high", "xhigh", "default"].map(|e| format!("‹ {e} ›"))
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
    assert_eq!(
        form(&app).value_parts(EditField::Model).0,
        "‹ role table (Claude · Sonnet 5) ›"
    );
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
    assert_eq!(
        effort_shown(&app),
        "‹ default ›",
        "max, then the model's default"
    );
    app.on_key(key(KeyCode::Char(' ')));
    assert_eq!(effort_shown(&app), "‹ low ›");
}

fn user_route(model: &str, effort: &str) -> TaskEditForm {
    let mut task = edit_fixture_task();
    task.route_spec = RouteSpec {
        runtime: Some(Runtime::Codex),
        model: Some(model.into()),
        strength: None,
        effort: Some(Effort::new(effort)),
    };
    task.route = proto::Route {
        runtime: Runtime::Codex,
        model: model.into(),
        effort: Effort::new(effort),
    };
    TaskEditForm::new(RUN_ID, &task)
}

/// Review I1: `⏎⏎` on the model row (the picker opens on the route's model, `⏎`
/// picks it again) keeps the user's effort: nothing changed.
#[test]
fn re_picking_the_same_model_keeps_its_effort() {
    let mut app = edit_app();
    app.modal = Some(Modal::EditTask(Box::new(user_route("gpt-6-sol", "max"))));
    app.on_key(key(KeyCode::Enter));
    app.on_key(key(KeyCode::Enter));
    assert!(form(&app).picker.is_none());
    assert_eq!(form(&app).route.effort, Some(Effort::new("max")));
    assert_eq!(effort_shown(&app), "‹ max ›");
    app.on_key(key(KeyCode::Tab));
    assert!(app.on_key(key(KeyCode::Enter)).is_empty());
    assert_eq!(app.toast_text(), Some("nothing changed"));
}

/// Review I3: after `role table` clears a user's model, the effort row has no catalog
/// to cycle: the task's resolution is the old model's, not the row's.
#[test]
fn role_table_after_a_users_model_cycles_no_stale_efforts() {
    let mut app = edit_app();
    app.modal = Some(Modal::EditTask(Box::new(user_route("gpt-6-luna", "xhigh"))));
    app.on_key(key(KeyCode::Enter));
    for _ in 0..10 {
        app.on_key(key(KeyCode::Char('k')));
    }
    let picker = form(&app).picker.as_ref().unwrap();
    assert_eq!(picker.selected, 0);
    assert_eq!(
        picker.entries[0],
        PickerEntry::RoleTable("role table".into())
    );
    app.on_key(key(KeyCode::Enter));
    assert_eq!(form(&app).route, RouteSpec::default());
    assert_eq!(form(&app).value_parts(EditField::Model).0, "‹ role table ›");
    assert_eq!(form(&app).effort_model(), None);
    app.on_key(key(KeyCode::Tab));
    app.on_key(key(KeyCode::Char(' ')));
    assert_eq!(
        form(&app).route.effort,
        None,
        "luna's efforts are not offered"
    );
    assert_eq!(
        form(&app).value_parts(EditField::Effort),
        ("‹ role table ›".into(), None)
    );
}

/// Fix round 1 (controller ruling I2): with no route the model row names what the
/// table runs, the model and its effort; a picked model with no effort reads
/// `default`.
#[test]
fn the_form_draws_the_effective_choice() {
    let mut task = edit_fixture_task();
    task.route_spec = RouteSpec::default();
    let mut app = edit_app();
    let _ = app.set_terminal_size(120, 40);
    app.modal = Some(Modal::EditTask(Box::new(TaskEditForm::new(RUN_ID, &task))));
    // The app labels the table's model from the catalogs before each key.
    app.on_key(key(KeyCode::Tab));
    app.on_key(key(KeyCode::BackTab));
    let draw = |app: &App| crate::ui::audit::rows(&crate::ui::audit::draw(app, 120, 40)).join("\n");
    let rows = draw(&app);
    assert!(
        rows.contains("▌ model      ‹ role table (Claude · Sonnet 5 · medium) ›"),
        "{rows}"
    );
    assert!(
        rows.contains("  effort     ‹ role table ›  medium"),
        "{rows}"
    );
    app.on_key(key(KeyCode::Enter));
    app.on_key(key(KeyCode::Char('j')));
    app.on_key(key(KeyCode::Enter));
    let rows = draw(&app);
    assert!(
        rows.contains("▌ model      ‹ Claude · Haiku 4.5 ›"),
        "{rows}"
    );
    assert!(rows.contains("  effort     ‹ default ›"), "{rows}");
}

/// Review minor 9: a `Models` reply refreshes the edit form's open picker, the
/// selection kept; `r` there asks for a live probe.
#[test]
fn a_models_reply_refreshes_the_edit_forms_open_picker() {
    let mut app = edit_app();
    app.catalogs.list[1].models.pop();
    app.on_key(key(KeyCode::Enter));
    let luna = mref("codex:gpt-6-luna");
    for _ in 0..10 {
        if selected_model(form(&app).picker.as_ref().unwrap()) == Some(luna.clone()) {
            break;
        }
        app.on_key(key(KeyCode::Char('j')));
    }
    let has_sol = |app: &App| {
        (form(app).picker.as_ref().unwrap().entries.iter())
            .any(|e| matches!(e, PickerEntry::Model { label, .. } if label == "gpt-6.1 sol"))
    };
    assert!(!has_sol(&app));
    let catalogs = crate::app::model_picker::tests::fixture_catalogs().list;
    app.on_daemon(proto::DaemonMsg::Models { catalogs });
    assert!(has_sol(&app));
    assert_eq!(
        selected_model(form(&app).picker.as_ref().unwrap()),
        Some(luna)
    );
    assert_eq!(
        app.on_key(key(KeyCode::Char('r'))),
        [Effect::Send(ClientMsg::ListModels {
            runtime: None,
            refresh: true
        })]
    );
}

/// Fix round 1 (controller ruling): picking the task's row model with no effort runs
/// the row's effort, drawn `<effort> (role table)`; another model reads `default`.
#[test]
fn the_effort_row_shows_the_rows_effort_for_the_rows_model() {
    let mut task = edit_fixture_task();
    task.row = Some(proto::Route {
        runtime: Runtime::Claude,
        model: "claude-opus-5-5".into(),
        effort: Effort::new("max"),
    });
    let mut app = edit_app();
    app.modal = Some(Modal::EditTask(Box::new(TaskEditForm::new(RUN_ID, &task))));
    let pick = |app: &mut App, label: &str| {
        app.on_key(key(KeyCode::Enter));
        // The picker opens on the current model: from the top.
        for _ in 0..12 {
            app.on_key(key(KeyCode::Char('k')));
        }
        for _ in 0..12 {
            let picker = form(app).picker.as_ref().unwrap();
            if matches!(&picker.entries[picker.selected],
                PickerEntry::Model { label: l, .. } if l == label)
            {
                break;
            }
            app.on_key(key(KeyCode::Char('j')));
        }
        app.on_key(key(KeyCode::Enter));
    };
    pick(&mut app, "Opus 5.5");
    assert_eq!(effort_shown(&app), "‹ max (role table) ›");
    pick(&mut app, "Haiku 4.5");
    assert_eq!(effort_shown(&app), "‹ default ›");
}

/// The task's row, Opus 5.5 at `max` (`TaskInfo.row`), on `task`.
fn with_opus_row(mut task: proto::TaskInfo) -> proto::TaskInfo {
    task.row = Some(proto::Route {
        runtime: Runtime::Claude,
        model: "claude-opus-5-5".into(),
        effort: Effort::new("max"),
    });
    task
}

/// M9.8.15 (Task 11's parked minor 1): a pinned effort over the row's model runs at the
/// pinned effort, so the role table text never names the row's: it names the model, and
/// the effort row the pinned effort.
#[test]
fn a_pinned_effort_is_never_contradicted_by_the_rows() {
    let mut task = with_opus_row(edit_fixture_task());
    task.route_spec = RouteSpec {
        effort: Some(Effort::new("low")),
        ..RouteSpec::default()
    };
    let form = TaskEditForm::new(RUN_ID, &task);
    assert_eq!(
        form.role_table_text(),
        "role table (Claude · claude-opus-5-5)"
    );
    assert_eq!(form.value_parts(EditField::Effort).0, "‹ low ›");
    // With no pinned effort, the row's effort is what runs.
    let mut task = with_opus_row(edit_fixture_task());
    task.route_spec = RouteSpec::default();
    let form = TaskEditForm::new(RUN_ID, &task);
    assert_eq!(
        form.role_table_text(),
        "role table (Claude · claude-opus-5-5 · max)"
    );
}

/// M9.8.15 (Task 11's parked minor 2): a size change can move the task to another row,
/// which the form does not hold: it says the row is resolved on save, shows no stale
/// resolved effort and cycles no stale model's efforts; back at the opened size, the
/// row shows again. A hub task keeps its row at any size.
#[test]
fn a_size_change_says_the_row_is_resolved_on_save() {
    let mut task = with_opus_row(edit_fixture_task());
    task.route_spec = RouteSpec::default();
    let mut form = TaskEditForm::new(RUN_ID, &task);
    assert_eq!(form.size, proto::Size::M);
    form.size = proto::Size::S;
    assert_eq!(form.role_table_text(), "role table (resolved on save)");
    assert_eq!(form.table_model(), None);
    assert_eq!(form.effort_model(), None);
    assert_eq!(
        form.value_parts(EditField::Effort),
        ("‹ role table ›".to_string(), None)
    );
    form.size = proto::Size::M;
    assert_eq!(
        form.role_table_text(),
        "role table (Claude · claude-opus-5-5 · max)"
    );
    // A hub task's row does not depend on its size.
    task.hub = true;
    let mut form = TaskEditForm::new(RUN_ID, &task);
    form.size = proto::Size::S;
    assert_eq!(
        form.role_table_text(),
        "role table (Claude · claude-opus-5-5 · max)"
    );
}
