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
use crate::run_goal::{GoalField, GoalForm, GoalModel};
use crate::tree::NodeKey;
use proto::{
    ActionKind, BudgetLimit, ModelEntry, OrchestratorChoice, OrchestratorDefault, Origin, RunReply,
    RunRequest, SettingsDoc, SettingsLimits, SettingsReply, SettingsRequest, Strength,
};
use std::collections::BTreeMap;
use std::time::Instant;

pub(super) fn entry(runtime: Runtime, model: &str) -> ModelEntry {
    ModelEntry {
        runtime,
        model: model.into(),
        strength: Strength::Standard,
        note: String::new(),
    }
}

pub(super) fn doc(models: Vec<ModelEntry>) -> SettingsDoc {
    let budget = BudgetLimit {
        tool_calls: 10,
        minutes: 5,
    };
    SettingsDoc {
        models,
        orchestrator: OrchestratorDefault {
            runtime: None,
            model: String::new(),
        },
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
    }
}

pub(super) fn roster() -> Vec<ModelEntry> {
    vec![
        entry(Runtime::Claude, "claude-haiku-4-5"),
        entry(Runtime::Codex, "gpt-6-sol"),
        entry(Runtime::Claude, "claude-opus-5-5"),
        entry(Runtime::Codex, ""),
    ]
}

fn reply(reply: SettingsReply, id: u64) -> DaemonMsg {
    DaemonMsg::Run(RunReply::Settings {
        reply: Box::new(reply),
        request_id: Some(id),
    })
}

fn current(models: Vec<ModelEntry>) -> SettingsReply {
    SettingsReply::Current {
        doc: doc(models),
        origin: BTreeMap::from([("orchestrator.models".to_string(), Origin::File)]),
        path: "/cfg/config.toml".into(),
    }
}

fn saved(models: Vec<ModelEntry>) -> SettingsReply {
    SettingsReply::Saved {
        doc: doc(models),
        origin: BTreeMap::from([("orchestrator.models".to_string(), Origin::File)]),
    }
}

fn get() -> RunRequest {
    RunRequest::Settings(SettingsRequest::Get)
}

fn gets(effects: &[Effect]) -> Vec<u64> {
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
pub(super) fn app_with_cache(models: Vec<ModelEntry>) -> App {
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
    assert_eq!(cache.doc.models, roster());
    assert_eq!(cache.path, std::path::PathBuf::from("/cfg/config.toml"));
    assert_eq!(cache.origin["orchestrator.models"], Origin::File);
    // The entry is spent: a repeat of the same id changes nothing.
    app.on_daemon(reply(current(vec![]), ids[0]));
    assert_eq!(app.settings_cache.as_ref().unwrap().doc.models, roster());

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
    assert_eq!(cache.doc.models.len(), 1);
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
    assert_eq!(app.settings_cache.as_ref().unwrap().doc.models, roster());
    // The new Get's `Current` brings the file's truth in.
    let id = gets(&effects)[0];
    app.on_daemon(reply(current(vec![entry(Runtime::Codex, "gpt-6-sol")]), id));
    assert_eq!(app.settings_cache.as_ref().unwrap().doc.models.len(), 1);

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
    assert_eq!(app.settings_cache.as_ref().unwrap().doc.models, one);
    assert!(app.on_daemon(reply(current(roster()), older)).is_empty());
    assert_eq!(app.settings_cache.as_ref().unwrap().doc.models, one);
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

/// The picker's drawn entries and the one chosen.
fn picker(app: &App) -> (Vec<String>, String) {
    let f = form(app);
    let options = f.model_options();
    let at = options[f.model_at()].clone();
    (options, at)
}

#[test]
fn the_model_picker_lists_the_runtimes_enabled_models_then_custom() {
    let mut app = app_with_cache(roster());
    open_form(&mut app);
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Right);
    assert_eq!(form(&app).runtime, Some(Runtime::Claude));
    focus(&mut app, GoalField::Model);
    // Claude's models in roster order, the empty-model entry of Codex never.
    let (options, at) = picker(&app);
    assert_eq!(
        options,
        ["default", "claude-haiku-4-5", "claude-opus-5-5", "custom…"]
    );
    assert_eq!(at, "default");

    tap(&mut app, KeyCode::Right);
    assert_eq!(form(&app).model, GoalModel::Pick(0));
    tap(&mut app, KeyCode::Char(' '));
    assert_eq!(form(&app).model, GoalModel::Pick(1));
    assert_eq!(
        request(&mut app, "add a"),
        RunRequest::StartGoal {
            goal: "add a".into(),
            dir: "/p/a".into(),
            yes: false,
            trust_project: false,
            unconfined_checks: false,
            orchestrator: Some(OrchestratorChoice {
                runtime: Runtime::Claude,
                model: Some("claude-opus-5-5".into()),
            }),
            delivery: None,
            continue_from: None,
            design: None,
        }
    );
}

#[test]
fn custom_reveals_the_text_line_and_its_text_is_the_model_sent() {
    let mut app = app_with_cache(roster());
    open_form(&mut app);
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Char(' '));
    focus(&mut app, GoalField::Model);
    // `custom…` is last: one step back from `default` wraps to it.
    tap(&mut app, KeyCode::Left);
    assert!(matches!(form(&app).model, GoalModel::Custom));
    // Space and every character are text now; the arrows still move the picker.
    typed(&mut app, "my model");
    app.on_paste("-2\n".into());
    assert_eq!(form(&app).custom.text(), "my model-2");
    // The picker moving away and back keeps what was typed.
    tap(&mut app, KeyCode::Left);
    assert_eq!(form(&app).model, GoalModel::Pick(1));
    tap(&mut app, KeyCode::Right);
    assert_eq!(form(&app).model, GoalModel::Custom);
    assert_eq!(form(&app).custom.text(), "my model-2");
    assert_eq!(
        request(&mut app, "add a"),
        RunRequest::StartGoal {
            goal: "add a".into(),
            dir: "/p/a".into(),
            yes: false,
            trust_project: false,
            unconfined_checks: false,
            orchestrator: Some(OrchestratorChoice {
                runtime: Runtime::Claude,
                model: Some("my model-2".into()),
            }),
            delivery: None,
            continue_from: None,
            design: None,
        }
    );
}

#[test]
fn custom_with_no_text_is_the_default_model() {
    let mut app = app_with_cache(roster());
    open_form(&mut app);
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Right);
    tap(&mut app, KeyCode::Right); // codex
    focus(&mut app, GoalField::Model);
    assert_eq!(picker(&app).0, ["default", "gpt-6-sol", "custom…"]);
    tap(&mut app, KeyCode::Left);
    assert!(matches!(form(&app).model, GoalModel::Custom));
    let RunRequest::StartGoal { orchestrator, .. } = request(&mut app, "add a") else {
        panic!()
    };
    assert_eq!(
        orchestrator,
        Some(OrchestratorChoice {
            runtime: Runtime::Codex,
            model: None
        })
    );
}

#[test]
fn with_runtime_configured_the_model_is_default_only() {
    let mut app = app_with_cache(roster());
    open_form(&mut app);
    assert_eq!(form(&app).runtime, None);
    focus(&mut app, GoalField::Model);
    assert_eq!(picker(&app).0, ["default"]);
    // Nothing to step to: the arrows and Space leave it on default.
    for code in [KeyCode::Right, KeyCode::Left, KeyCode::Char(' ')] {
        tap(&mut app, code);
        assert_eq!(form(&app).model, GoalModel::Default);
    }
    // Choosing a runtime resets the model, which named another runtime's.
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Right);
    focus(&mut app, GoalField::Model);
    tap(&mut app, KeyCode::Right);
    assert_eq!(form(&app).model, GoalModel::Pick(0));
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Right);
    assert_eq!(form(&app).model, GoalModel::Default);
}

#[test]
fn without_a_cache_the_picker_offers_default_and_custom() {
    let mut app = app();
    assert_eq!(app.settings_cache, None);
    open_form(&mut app);
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Right);
    focus(&mut app, GoalField::Model);
    assert_eq!(picker(&app).0, ["default", "custom…"]);
}

#[test]
fn a_cache_that_arrives_while_the_form_is_open_fills_the_picker() {
    let mut app = app();
    let id = gets(&[app.settings_fetch()])[0];
    open_form(&mut app);
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Right);
    focus(&mut app, GoalField::Model);
    assert_eq!(picker(&app).0.len(), 2);
    app.on_daemon(reply(current(roster()), id));
    assert_eq!(picker(&app).0.len(), 4);
    tap(&mut app, KeyCode::Right);
    tap(&mut app, KeyCode::Right);
    assert_eq!(form(&app).model, GoalModel::Pick(1));

    // A save that drops the picked model leaves the choice on default; one that keeps
    // it picked, keeps it by name.
    let (id, _) = tagged(&[app.settings_put(doc(vec![]))]);
    app.on_daemon(reply(
        saved(vec![
            entry(Runtime::Claude, "claude-opus-5-5"),
            entry(Runtime::Claude, "claude-haiku-4-5"),
        ]),
        id,
    ));
    assert_eq!(form(&app).model, GoalModel::Pick(0));
    let (id, _) = tagged(&[app.settings_put(doc(vec![]))]);
    app.on_daemon(reply(saved(vec![]), id));
    assert_eq!(form(&app).model, GoalModel::Default);
}

#[test]
fn promote_picks_from_the_roster_cache() {
    let (mut snap, windows) = running_snapshot();
    snap.runs[0]
        .actions
        .push(action(ActionKind::Promote, "promote", None));
    let mut app = app_with_runs(windows, snap);
    let id = gets(&[app.settings_fetch()])[0];
    app.on_daemon(reply(current(roster()), id));
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
            "cl claude-haiku-4-5",
            "cx gpt-6-sol",
            "cl claude-opus-5-5",
            "cx default"
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
                model: Some("claude-opus-5-5".into()),
            }),
        }
    );
}
