//! Milestone 9.0.6 task 12: the settings cache (decision 24) and the goal form's two
//! toggles and model picker (decision 39).

use super::actions::{action, running_snapshot, tap};
use super::orch::tagged;
use super::runs::{app_with_runs, snapshot};
use super::*;
use crate::actions_request::ActionTarget;
use crate::app::Modal;
use crate::app::actions::ActionStep;
use crate::app::actions::forms::ActionForm;
use crate::app::replies::PendingWhat;
use crate::run_goal::{GoalField, GoalForm};
use crate::tree::NodeKey;
use proto::models::{HelperKind, ModelRef, ModelTable, Role, RoleChoice};
use proto::{
    ActionKind, BudgetLimit, OrchestratorChoice, Origin, RunReply, RunRequest, SettingsDoc,
    SettingsLimits, SettingsReply, SettingsRequest,
};
use std::collections::BTreeMap;
use std::time::Instant;

/// A fixture's model: a runtime and a model id (`proto::ModelEntry`, the roster's
/// entry, went with strength in M9.8.14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FixtureModel {
    pub(super) runtime: Runtime,
    pub(super) model: String,
}

pub(super) fn entry(runtime: Runtime, model: &str) -> FixtureModel {
    FixtureModel {
        runtime,
        model: model.into(),
    }
}

/// M9.8.12: the roster left the settings document. A fixture's entries become helper
/// kind rows (one per kind, in order), which tell documents apart without changing
/// what the orchestrator row or the goal form's `role table` entry reads.
pub(super) fn table(models: Vec<FixtureModel>) -> ModelTable {
    let mut out = ModelTable::default();
    for (m, kind) in models.into_iter().zip(HelperKind::ALL) {
        let id = if m.model.is_empty() {
            "default"
        } else {
            &m.model
        };
        let model = ModelRef::parse(&format!("{}:{id}", m.runtime.label())).unwrap();
        let row = RoleChoice {
            model,
            effort: None,
            fallback: None,
        };
        out.rows.insert(Role::Helper(kind), row);
    }
    out
}

pub(super) fn doc(models: Vec<FixtureModel>) -> SettingsDoc {
    let budget = BudgetLimit {
        tool_calls: 10,
        minutes: 5,
    };
    SettingsDoc {
        limits: SettingsLimits {
            budget_s: budget,
            budget_m: budget,
            budget_l: budget,
            stall_after_secs: 600,
            max_writers: 2,
            max_readers: 2,
            max_bounces: 2,
        },
        design_default: None,
        roles: table(models),
    }
}

pub(super) fn roster() -> Vec<FixtureModel> {
    vec![
        entry(Runtime::Claude, "claude-haiku-4-5"),
        entry(Runtime::Codex, "gpt-6-sol"),
        entry(Runtime::Claude, "claude-opus-5-5"),
        entry(Runtime::Codex, ""),
    ]
}

pub(super) fn reply(reply: SettingsReply, id: u64) -> DaemonMsg {
    DaemonMsg::Run(RunReply::Settings {
        reply: Box::new(reply),
        request_id: Some(id),
    })
}

fn current(models: Vec<FixtureModel>) -> SettingsReply {
    SettingsReply::Current {
        doc: doc(models),
        origin: BTreeMap::from([("models".to_string(), Origin::File)]),
        path: "/cfg/config.toml".into(),
    }
}

fn saved(models: Vec<FixtureModel>) -> SettingsReply {
    SettingsReply::Saved {
        doc: doc(models),
        origin: BTreeMap::from([("models".to_string(), Origin::File)]),
    }
}

fn get() -> RunRequest {
    RunRequest::Settings(SettingsRequest::Get)
}

pub(super) fn gets(effects: &[Effect]) -> Vec<u64> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(ClientMsg::RunTagged { id, request }) if *request == get() => Some(*id),
            _ => None,
        })
        .collect()
}

pub(super) fn app() -> App {
    app_with_runs(project_windows(), snapshot(1, 100, vec![]))
}

/// An app whose settings fetch was answered with `models`.
pub(super) fn app_with_cache(models: Vec<FixtureModel>) -> App {
    let mut app = app();
    let id = gets(&[app.settings_fetch()])[0];
    assert!(app.on_daemon(reply(current(models), id)).is_empty());
    app
}

pub(super) fn form(app: &App) -> &GoalForm {
    match &app.modal {
        Some(Modal::StartGoal(form)) => form,
        other => panic!("no goal form: {other:?}"),
    }
}

pub(super) fn open_form(app: &mut App) {
    let rows = crate::tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, NodeKey::Project("/p/a".into()));
    prefix(app);
    assert!(press(app, KeyCode::Char('g'), KeyModifiers::NONE).is_empty());
    assert!(matches!(app.modal, Some(Modal::StartGoal(_))));
}

pub(super) fn focus(app: &mut App, field: GoalField) {
    // Milestone 9.6 task 18: nine fields.
    for _ in 0..9 {
        if form(app).focus == field {
            return;
        }
        tap(app, KeyCode::Tab);
    }
    panic!("{field:?} is not reachable by Tab");
}

pub(super) fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        assert!(tap(app, KeyCode::Char(c)).is_empty());
    }
}

/// The form's request, from a blank goal given text `goal`: Ctrl-S starts it
/// (milestone 9.3 decision 7; Enter in the text is a newline).
fn request(app: &mut App, goal: &str) -> RunRequest {
    focus(app, GoalField::Goal);
    typed(app, goal);
    tagged(&press(app, KeyCode::Char('s'), KeyModifiers::CONTROL)).1
}

#[test]
fn a_new_connection_fetches_the_settings_once() {
    let mut app = app();
    // The start: one tagged `Settings(Get)`, remembered by its id.
    let first = app.settings_fetch();
    let ids = gets(&[first]);
    assert_eq!(ids.len(), 1);
    assert_eq!(app.settings_cache, None);

    // Another reply id is ignored; so is a reply nobody asked for.
    assert!(
        app.on_daemon(reply(current(roster()), ids[0] + 50))
            .is_empty()
    );
    assert_eq!(app.settings_cache, None);
    assert_eq!(app.toast_text(), None, "an unknown id is not shown either");

    // The `Current` reply to the fetch fills the cache.
    assert!(app.on_daemon(reply(current(roster()), ids[0])).is_empty());
    let cache = app.settings_cache.as_ref().expect("cache");
    assert_eq!(cache.doc.roles, table(roster()));
    assert_eq!(cache.path, std::path::PathBuf::from("/cfg/config.toml"));
    assert_eq!(cache.origin["models"], Origin::File);
    // The entry is spent: a repeat of the same id changes nothing.
    app.on_daemon(reply(current(vec![]), ids[0]));
    assert_eq!(
        app.settings_cache.as_ref().unwrap().doc.roles,
        table(roster())
    );

    // Every new connection: exactly one more, under a new id.
    assert!(app.on_link_lost("x").is_empty());
    let effects = app.on_reconnected(project_windows());
    let again = gets(&effects);
    assert_eq!(again.len(), 1, "{effects:?}");
    assert_ne!(again[0], ids[0]);
    assert!(app.replies.contains(again[0]));
}

#[test]
fn a_refused_settings_send_stops_waiting() {
    let mut app = app();
    let id = gets(&[app.settings_fetch()])[0];
    let msg = ClientMsg::RunTagged { id, request: get() };
    assert!(app.on_send_failed(&msg).is_empty());
    assert!(!app.replies.contains(id));
    assert_eq!(app.toast_text(), None, "quiet, as the run subscription is");
}

#[test]
fn a_saved_reply_replaces_the_cache_and_keeps_its_path() {
    let mut app = app_with_cache(roster());
    let put = app.settings_put(doc(vec![entry(Runtime::Claude, "claude-opus-5-5")]));
    let (id, request) = tagged(&[put]);
    assert!(matches!(
        request,
        RunRequest::Settings(SettingsRequest::Put { .. })
    ));
    let effects = app.on_daemon(reply(
        saved(vec![entry(Runtime::Claude, "claude-opus-5-5")]),
        id,
    ));
    assert!(effects.is_empty(), "{effects:?}");
    let cache = app.settings_cache.as_ref().unwrap();
    assert_eq!(cache.doc.roles.rows.len(), 1);
    assert_eq!(cache.path, std::path::PathBuf::from("/cfg/config.toml"));
}

#[test]
fn a_put_that_was_not_saved_asks_for_the_settings_again() {
    let mut app = app_with_cache(roster());
    let (id, _) = tagged(&[app.settings_put(doc(vec![]))]);
    let effects = app.on_daemon(reply(
        SettingsReply::Refused {
            problems: vec![
                "config.toml was still being written after 5 s; if the write completes, new runs use the new settings"
                    .into(),
            ],
        },
        id,
    ));
    // The cache may be stale (the write's outcome is unknown): one new Get.
    assert_eq!(gets(&effects).len(), 1, "{effects:?}");
    assert_eq!(app.toast_level(), Some(ToastLevel::Error));
    assert!(app.toast_text().unwrap().contains("still being written"));
    assert_eq!(
        app.settings_cache.as_ref().unwrap().doc.roles,
        table(roster())
    );
    // The new Get's `Current` brings the file's truth in.
    let id = gets(&effects)[0];
    app.on_daemon(reply(current(vec![entry(Runtime::Codex, "gpt-6-sol")]), id));
    assert_eq!(app.settings_cache.as_ref().unwrap().doc.roles.rows.len(), 1);

    // A refused `Get` does not loop: it toasts and sends nothing.
    let id = gets(&[app.settings_fetch()])[0];
    let effects = app.on_daemon(reply(SettingsReply::Refused { problems: vec![] }, id));
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn an_older_reply_never_replaces_a_newer_cache() {
    let mut app = app();
    let get = gets(&[app.settings_fetch()])[0];
    app.on_daemon(reply(current(roster()), get));
    // A newer Get, then a Put after it: the Put's Saved lands first, the Get's
    // (older) Current arrives late and is ignored.
    let older = gets(&[app.settings_fetch()])[0];
    let (put, _) = tagged(&[app.settings_put(doc(vec![]))]);
    let one = vec![entry(Runtime::Claude, "claude-opus-5-5")];
    app.on_daemon(reply(saved(one.clone()), put));
    assert_eq!(
        app.settings_cache.as_ref().unwrap().doc.roles,
        table(one.clone())
    );
    assert!(app.on_daemon(reply(current(roster()), older)).is_empty());
    assert_eq!(
        app.settings_cache.as_ref().unwrap().doc.roles,
        table(one.clone())
    );
    assert!(
        !app.replies.contains(older),
        "the stale reply was still spent"
    );
}

#[test]
fn saved_with_no_cache_asks_for_the_settings() {
    let mut app = app();
    let (put, _) = tagged(&[app.settings_put(doc(vec![]))]);
    let effects = app.on_daemon(reply(saved(roster()), put));
    assert_eq!(gets(&effects).len(), 1, "{effects:?}");
    assert_eq!(app.settings_cache, None);
}

#[test]
fn a_put_that_expires_asks_for_the_settings_again() {
    let mut app = app_with_cache(roster());
    let (put, _) = tagged(&[app.settings_put(doc(vec![]))]);
    app.set_reply_sent_at(put, Instant::now() - std::time::Duration::from_secs(60));
    let effects = app.on_tick();
    assert_eq!(gets(&effects).len(), 1, "{effects:?}");
    assert_eq!(app.toast_text(), Some("no reply from daemon"));
    assert!(!app.replies.contains(put));
}

#[test]
fn a_settings_reply_with_another_kinds_id_leaves_that_entry() {
    let mut app = app();
    app.replies.insert(
        7,
        PendingWhat::FormBrief {
            run_id: "r".into(),
            task_id: "t".into(),
        },
        std::time::Duration::from_secs(30),
    );
    app.on_daemon(reply(current(roster()), 7));
    assert!(app.replies.contains(7));
    assert_eq!(app.settings_cache, None);
}

#[test]
fn a_refused_put_send_stops_waiting_and_says_so() {
    let mut app = app_with_cache(roster());
    let (id, request) = tagged(&[app.settings_put(doc(vec![]))]);
    let msg = ClientMsg::RunTagged { id, request };
    app.on_send_failed(&msg);
    assert!(!app.replies.contains(id));
    assert!(app.toast_text().is_some());
}

#[test]
fn a_put_waits_longer_than_two_write_timeouts() {
    let mut app = app_with_cache(roster());
    let (id, _) = tagged(&[app.settings_put(doc(vec![]))]);
    // The daemon may wait 5 s for the write lock and 5 s for the save.
    let later = Instant::now() + std::time::Duration::from_secs(11);
    assert!(app.replies.expire(later).is_empty());
    assert!(app.replies.contains(id));
    assert_eq!(app.replies.peek(id), Some(&PendingWhat::SettingsPut));
}

#[test]
fn the_toggles_reach_the_request() {
    let mut app = app();
    open_form(&mut app);
    assert!(!form(&app).yes && !form(&app).unconfined_checks && !form(&app).trust_project);
    focus(&mut app, GoalField::Yes);
    tap(&mut app, KeyCode::Char(' '));
    focus(&mut app, GoalField::UnconfinedChecks);
    tap(&mut app, KeyCode::Right);
    assert!(form(&app).yes && form(&app).unconfined_checks);
    assert_eq!(
        request(&mut app, "add a"),
        RunRequest::StartGoal {
            goal: "add a".into(),
            dir: "/p/a".into(),
            yes: true,
            trust_project: false,
            unconfined_checks: true,
            orchestrator: None,
            delivery: None,
            continue_from: None,
            design: None,
        }
    );

    // Both default off: the M9 request is unchanged.
    let mut app = self::app();
    open_form(&mut app);
    assert_eq!(
        request(&mut app, "add a"),
        RunRequest::StartGoal {
            goal: "add a".into(),
            dir: "/p/a".into(),
            yes: false,
            trust_project: false,
            unconfined_checks: false,
            orchestrator: None,
            delivery: None,
            continue_from: None,
            design: None,
        }
    );
}

/// Milestone 9.8 decision 39 (replacing 9.0.6's roster picker tests,
/// `the_model_picker_lists_the_runtimes_enabled_models_then_custom`,
/// `custom_reveals_the_text_line_…`, `custom_with_no_text_is_the_default_model`,
/// `with_runtime_configured_the_model_is_default_only`,
/// `without_a_cache_the_picker_offers_default_and_custom` and
/// `a_cache_that_arrives_while_the_form_is_open_fills_the_picker`): the picker's first
/// entry names the role table's orchestrator row, and follows a cache that arrives
/// while the form is open.
#[test]
fn the_role_table_entry_names_the_orchestrator_row_and_follows_the_cache() {
    let mut app = app();
    let id = gets(&[app.settings_fetch()])[0];
    open_form(&mut app);
    // No catalog yet: the row's model by its id.
    assert_eq!(
        form(&app).role_table,
        "role table (Claude · claude-opus-5-5)"
    );
    let mut roles = proto::models::ModelTable::default();
    roles.rows.insert(
        proto::models::Role::Orchestrator,
        proto::models::RoleChoice {
            model: proto::models::ModelRef::parse("codex:gpt-6-sol").unwrap(),
            effort: None,
            fallback: None,
        },
    );
    let mut with_roles = doc(roster());
    with_roles.roles = roles;
    app.on_daemon(reply(
        SettingsReply::Current {
            doc: with_roles,
            origin: BTreeMap::new(),
            path: "/cfg/config.toml".into(),
        },
        id,
    ));
    assert_eq!(form(&app).role_table, "role table (Codex · gpt-6-sol)");
    focus(&mut app, GoalField::Model);
    tap(&mut app, KeyCode::Enter);
    let picker = form(&app).picker.as_ref().expect("the picker");
    assert_eq!(
        picker.entries[0],
        crate::app::model_picker::PickerEntry::RoleTable("role table (Codex · gpt-6-sol)".into())
    );
}

/// `custom…` in the goal form's picker: the runtime, then any name `ModelRef` accepts;
/// a paste reaches the name.
#[test]
fn custom_in_the_picker_names_any_model() {
    let mut app = app();
    open_form(&mut app);
    focus(&mut app, GoalField::Model);
    tap(&mut app, KeyCode::Enter);
    // No catalogs: the role table, two headers, then `custom…`.
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Enter);
    tap(&mut app, KeyCode::Right);
    tap(&mut app, KeyCode::Enter);
    typed(&mut app, "my-model");
    app.on_paste("-2\n".into());
    tap(&mut app, KeyCode::Enter);
    assert!(form(&app).picker.is_none());
    assert_eq!(form(&app).model_label, "Codex · my-model-2");
    let RunRequest::StartGoal { orchestrator, .. } = request(&mut app, "add a") else {
        panic!()
    };
    assert_eq!(
        orchestrator,
        Some(OrchestratorChoice {
            runtime: Runtime::Codex,
            model: Some("my-model-2".into()),
            effort: None
        })
    );
}

#[test]
fn promote_picks_from_the_catalogs() {
    // M9.8.12: the roster left the settings; the picker offers what the CLIs report.
    let (mut snap, windows) = running_snapshot();
    snap.runs[0]
        .actions
        .push(action(ActionKind::Promote, "promote", None));
    let mut app = app_with_runs(windows, snap);
    let id = gets(&[app.settings_fetch()])[0];
    app.on_daemon(reply(current(roster()), id));
    app.catalogs = crate::app::model_picker::tests::fixture_catalogs();
    app.open_actions(
        (crate::tree::run_fixtures::RUN_ID.into(), ActionTarget::Run),
        Some(ActionKind::Promote),
    );
    tap(&mut app, KeyCode::Enter);
    let Some(Modal::Action(flow)) = &app.modal else {
        panic!()
    };
    let ActionStep::Form(f) = &flow.step else {
        panic!()
    };
    let ActionForm::Promote(f) = &**f else {
        panic!()
    };
    let labels: Vec<_> = f.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "configured",
            "cl default",
            "cl opus[1m]",
            "cl claude-fable-5-1[1m]",
            "cl sonnet",
            "cl haiku",
            "cx gpt-6-luna",
            "cx gpt-6-sol",
            "cx gpt-6.1-sol"
        ]
    );
    // Picking the third entry sends its runtime and model.
    for _ in 0..3 {
        tap(&mut app, KeyCode::Tab);
    }
    tap(&mut app, KeyCode::Enter);
    let (_, request) = tagged(&tap(&mut app, KeyCode::Enter));
    assert_eq!(
        request,
        RunRequest::Promote {
            run_id: crate::tree::run_fixtures::RUN_ID.into(),
            orchestrator: Some(OrchestratorChoice {
                runtime: Runtime::Claude,
                model: Some("claude-fable-5-1[1m]".into()),
                effort: None,
            }),
        }
    );
}

/// M9.8.12 fix round 1 (M5): with no catalog yet, opening Promote asks for the lists
/// as the Settings screen does, and the open picker takes them when they come.
#[test]
fn promote_asks_for_the_catalogs_and_follows_them() {
    let (mut snap, windows) = running_snapshot();
    snap.runs[0]
        .actions
        .push(action(ActionKind::Promote, "promote", None));
    let mut app = app_with_runs(windows, snap);
    app.open_actions(
        (crate::tree::run_fixtures::RUN_ID.into(), ActionTarget::Run),
        Some(ActionKind::Promote),
    );
    let effects = tap(&mut app, KeyCode::Enter);
    let asks: Vec<_> = effects
        .iter()
        .filter(|e| {
            matches!(
                e,
                Effect::Send(ClientMsg::ListModels {
                    runtime: None,
                    refresh: false
                })
            )
        })
        .collect();
    assert_eq!(asks.len(), 1, "{effects:?}");
    let options = |app: &App| {
        let Some(Modal::Action(flow)) = &app.modal else {
            panic!()
        };
        let ActionStep::Form(f) = &flow.step else {
            panic!()
        };
        let ActionForm::Promote(f) = &**f else {
            panic!()
        };
        f.options.len()
    };
    assert_eq!(options(&app), 1, "only configured");
    let catalogs = crate::app::model_picker::tests::fixture_catalogs().list;
    app.on_daemon(proto::DaemonMsg::Models { catalogs });
    assert_eq!(options(&app), 9);
}
