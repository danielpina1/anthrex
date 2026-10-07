//! Milestone 9.0.6 task 14, decision 36: the Settings screen's keys, its document, its
//! inline problems and warnings, and its save. Every rule is `config::settings`'s own.

use super::actions::tap;
use super::goal_form::{app, doc, entry};
use super::orch::tagged;
use super::*;
use crate::app::screens::Screen;
use crate::app::settings_screen::{
    SettingsPage, SettingsScreen, SettingsSection, hard_stop_calls, hard_stop_minutes,
};
use proto::settings::key;
use proto::{
    ModelEntry, Origin, RunReply, RunRequest, SettingsDoc, SettingsReply, SettingsRequest, Strength,
};
use std::collections::BTreeMap;

fn model(runtime: Runtime, name: &str, strength: Strength) -> ModelEntry {
    ModelEntry {
        strength,
        ..entry(runtime, name)
    }
}

/// `claude-opus-5-5` (shipped) and a custom `claude-x` fast, plus Codex's default.
pub(super) fn sample() -> SettingsDoc {
    doc(vec![
        model(Runtime::Claude, "claude-opus-5-5", Strength::Frontier),
        model(Runtime::Claude, "claude-x", Strength::Fast),
        model(Runtime::Codex, "", Strength::Standard),
    ])
}

/// Every key from the file, except the ones named.
pub(super) fn origin(defaults: &[&str]) -> BTreeMap<String, Origin> {
    proto::SETTINGS_KEYS
        .iter()
        .map(|k| {
            let o = if defaults.contains(k) {
                Origin::Default
            } else {
                Origin::File
            };
            (k.to_string(), o)
        })
        .collect()
}

pub(super) fn reply(reply: SettingsReply, id: u64) -> DaemonMsg {
    DaemonMsg::Run(RunReply::Settings {
        reply: Box::new(reply),
        request_id: Some(id),
    })
}

pub(super) fn current(doc: SettingsDoc, origin: BTreeMap<String, Origin>) -> SettingsReply {
    SettingsReply::Current {
        doc,
        origin,
        path: "/cfg/config.toml".into(),
    }
}

/// The settings requests in `effects`, with their ids.
pub(super) fn settings_sent(effects: &[Effect]) -> Vec<(u64, SettingsRequest)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(ClientMsg::RunTagged {
                id,
                request: RunRequest::Settings(r),
            }) => Some((*id, r.clone())),
            _ => None,
        })
        .collect()
}

/// An app whose cache holds `doc` with `origin`.
pub(super) fn cached(doc: SettingsDoc, origin: BTreeMap<String, Origin>) -> App {
    let mut app = app();
    let (id, _) = tagged(&[app.settings_fetch()]);
    assert!(app.on_daemon(reply(current(doc, origin), id)).is_empty());
    app
}

pub(super) fn open(app: &mut App) -> Vec<Effect> {
    prefix(app);
    press(app, KeyCode::Char('S'), KeyModifiers::SHIFT)
}

pub(super) fn screen(app: &App) -> &SettingsScreen {
    match &app.screen {
        Some(Screen::Settings(s)) => s,
        other => panic!("no settings screen: {other:?}"),
    }
}

/// The sample, open on the Settings screen.
pub(super) fn opened() -> App {
    let mut app = cached(sample(), origin(&[]));
    let effects = open(&mut app);
    only_the_tuning_ask(&app, &effects);
    app
}

/// Opening the screen sends exactly one effect: milestone 9.5 decision 48's tagged
/// read-only `Stats` for the app's project (`ui/settings_tuning_tests.rs` has the rest).
pub(super) fn only_the_tuning_ask(app: &App, effects: &[Effect]) {
    let dir = app.goal_project().expect("the app has a project");
    match effects {
        [Effect::Send(ClientMsg::RunTagged { request, .. })] => assert_eq!(
            *request,
            RunRequest::Stats {
                dir,
                apply: Vec::new(),
                dismiss: Vec::new(),
                read_only: true,
            }
        ),
        other => panic!("one read-only Stats, and nothing else: {other:?}"),
    }
}

pub(super) fn to_section(app: &mut App, section: SettingsSection) {
    for _ in 0..4 {
        if screen(app).section == section {
            return;
        }
        tap(app, KeyCode::Tab);
    }
    panic!("{section:?} is not reachable by Tab");
}

/// Selects row `at` of the current section from the top.
pub(super) fn select(app: &mut App, at: usize) {
    for _ in 0..20 {
        tap(app, KeyCode::Char('k'));
    }
    for _ in 0..at {
        tap(app, KeyCode::Char('j'));
    }
    assert_eq!(screen(app).selected, at);
}

fn limit_at(app: &App, k: &str) -> usize {
    screen(app).limits.iter().position(|f| f.key == k).unwrap()
}

/// Types `value` over the limit `k`.
pub(super) fn set_limit(app: &mut App, k: &str, value: &str) {
    to_section(app, SettingsSection::Limits);
    let at = limit_at(app, k);
    select(app, at);
    for _ in 0..12 {
        tap(app, KeyCode::Backspace);
    }
    for c in value.chars() {
        tap(app, KeyCode::Char(c));
    }
}

pub(super) fn names(app: &App, runtime: Runtime) -> Vec<(String, bool, bool)> {
    screen(app)
        .rows(runtime)
        .iter()
        .map(|r| (r.entry.model.clone(), r.enabled, r.custom))
        .collect()
}

pub(super) fn w(app: &mut App) -> Vec<Effect> {
    tap(app, KeyCode::Char('w'))
}

pub(super) fn puts(effects: &[Effect]) -> Vec<(u64, SettingsDoc)> {
    settings_sent(effects)
        .into_iter()
        .filter_map(|(id, r)| match r {
            SettingsRequest::Put { settings } => Some((id, settings)),
            SettingsRequest::Get
            | SettingsRequest::RepoModels { .. }
            | SettingsRequest::PutRepoModels { .. } => None,
        })
        .collect()
}

#[test]
fn c_b_s_opens_from_the_cache() {
    let mut app = cached(sample(), origin(&[]));
    // The cache is read, not asked for: the one effect is milestone 9.5 decision 48's
    // read-only `Stats` for the project's tuning.
    let effects = open(&mut app);
    only_the_tuning_ask(&app, &effects);
    assert!(screen(&app).loaded);
    assert_eq!(screen(&app).doc(), Ok(sample()));
    assert_eq!(screen(&app).path, "/cfg/config.toml");
    assert!(!screen(&app).dirty());

    // No cache: one Get, a loading screen, filled by the reply.
    let mut app = self::app();
    let effects = open(&mut app);
    let sent = settings_sent(&effects);
    assert_eq!(sent.len(), 1, "{effects:?}");
    assert_eq!(sent[0].1, SettingsRequest::Get);
    assert!(!screen(&app).loaded);
    assert!(w(&mut app).is_empty());
    app.on_daemon(reply(current(sample(), origin(&[])), sent[0].0));
    assert!(screen(&app).loaded);
    assert_eq!(screen(&app).doc(), Ok(sample()));

    // A Get already on its way is not doubled.
    let mut app = self::app();
    let _ = app.settings_fetch();
    assert!(settings_sent(&open(&mut app)).is_empty());
}

#[test]
fn rows_are_the_shipped_lists_then_custom_models() {
    let app = opened();
    let s = |n: &str, on: bool, custom: bool| (n.to_string(), on, custom);
    assert_eq!(
        names(&app, Runtime::Claude),
        vec![
            s("claude-haiku-4-5", false, false),
            s("claude-sonnet-5", false, false),
            s("claude-opus-5-5", true, false),
            s("claude-x", true, true),
        ]
    );
    assert_eq!(
        screen(&app).rows(Runtime::Claude)[3].entry.strength,
        Strength::Fast
    );
    // The shipped Codex list, `""` (Codex default) last and on; no custom rows.
    let codex = names(&app, Runtime::Codex);
    let shipped: Vec<&str> = config::settings::SHIPPED_CODEX
        .iter()
        .map(|m| m.model)
        .collect();
    assert_eq!(
        codex.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        shipped
    );
    assert_eq!(codex.iter().filter(|r| r.1).count(), 1);
    assert!(codex[7].1);
    // `custom…` is the row after the last model.
    assert_eq!(screen(&app).last_row(), 4);
}

#[test]
fn space_toggles_and_the_doc_follows() {
    let mut app = opened();
    select(&mut app, 0);
    tap(&mut app, KeyCode::Char(' '));
    let doc = screen(&app).doc().unwrap();
    // The roster keeps its order; the new model follows with its shipped strength.
    assert_eq!(doc.models[..3], sample().models[..]);
    assert_eq!(
        doc.models[3],
        model(Runtime::Claude, "claude-haiku-4-5", Strength::Fast)
    );
    assert!(screen(&app).dirty());
    tap(&mut app, KeyCode::Char(' '));
    assert_eq!(screen(&app).doc(), Ok(sample()));
    assert!(!screen(&app).dirty());
    // Off, the custom row stays a row and leaves the doc.
    select(&mut app, 3);
    tap(&mut app, KeyCode::Char(' '));
    assert_eq!(
        names(&app, Runtime::Claude)[3],
        ("claude-x".into(), false, true)
    );
    assert!(
        !screen(&app)
            .doc()
            .unwrap()
            .models
            .iter()
            .any(|m| m.model == "claude-x")
    );
    // `←`/`→` change a custom row's strength, not a shipped one's.
    tap(&mut app, KeyCode::Right);
    assert_eq!(
        screen(&app).rows(Runtime::Claude)[3].entry.strength,
        Strength::Standard
    );
    select(&mut app, 2);
    tap(&mut app, KeyCode::Right);
    assert_eq!(
        screen(&app).rows(Runtime::Claude)[2].entry.strength,
        Strength::Frontier
    );
}

#[test]
fn saving_with_no_model_is_refused_inline() {
    let mut app = opened();
    for at in [2, 3] {
        select(&mut app, at);
        tap(&mut app, KeyCode::Char(' '));
    }
    to_section(&mut app, SettingsSection::Codex);
    select(&mut app, 7);
    tap(&mut app, KeyCode::Char(' '));
    assert_eq!(screen(&app).problems(), vec!["enable at least one model"]);
    assert!(w(&mut app).is_empty(), "w sends nothing");
    assert_eq!(app.toast_text(), None);
}

#[test]
fn a_strength_on_one_runtime_warns_but_saves() {
    let mut app = opened();
    let warnings = screen(&app).warnings();
    assert_eq!(
        warnings,
        vec![
            "no codex model is fast: the cross-runtime reviewer cannot be chosen for fast tasks",
            "no claude model is standard: the cross-runtime reviewer cannot be chosen for standard tasks",
            "no codex model is frontier: the cross-runtime reviewer cannot be chosen for frontier tasks",
        ]
    );
    assert!(screen(&app).problems().is_empty());
    set_limit(&mut app, key::MAX_READERS, "4");
    let sent = puts(&w(&mut app));
    assert_eq!(sent.len(), 1);
    let mut want = sample();
    want.limits.max_readers = 4;
    assert_eq!(sent[0].1, want);
}

#[test]
fn an_out_of_range_limit_is_refused_inline() {
    let mut app = opened();
    set_limit(&mut app, key::MAX_WRITERS, "9");
    assert_eq!(
        screen(&app).problems(),
        vec!["orchestrator.max_writers: must be between 1 and 8"]
    );
    assert!(w(&mut app).is_empty());
    // A value its field cannot hold is refused with the same range.
    set_limit(&mut app, key::MAX_WRITERS, "300");
    assert_eq!(
        screen(&app).problems(),
        vec!["orchestrator.max_writers: must be between 1 and 8"]
    );
    // Blank: refused, never sent as a guess.
    set_limit(&mut app, key::MAX_WRITERS, "");
    set_limit(&mut app, key::BUDGET_S_CALLS, "");
    assert_eq!(
        screen(&app).problems(),
        vec![
            "orchestrator.budget.s.tool_calls: must be at least 1",
            "orchestrator.max_writers: must be between 1 and 8",
        ]
    );
    set_limit(&mut app, key::MAX_WRITERS, "8");
    set_limit(&mut app, key::BUDGET_S_CALLS, "12");
    let doc = screen(&app).doc().unwrap();
    assert_eq!(
        (doc.limits.max_writers, doc.limits.budget_s.tool_calls),
        (8, 12)
    );
    // Digits only, at most nine.
    tap(&mut app, KeyCode::Char('x'));
    for _ in 0..12 {
        tap(&mut app, KeyCode::Char('1'));
    }
    let at = limit_at(&app, key::BUDGET_S_CALLS);
    assert_eq!(screen(&app).limits[at].text, "121111111");
}

#[test]
fn budgets_show_their_hard_stop() {
    assert_eq!(hard_stop_calls(40), "hard stop at 60 calls");
    assert_eq!(hard_stop_calls(5), "hard stop at 8 calls");
    assert_eq!(hard_stop_minutes(15), "hard stop at 22m30s");
    assert_eq!(hard_stop_minutes(60), "hard stop at 90m");
    let mut app = opened();
    set_limit(&mut app, key::BUDGET_S_CALLS, "40");
    set_limit(&mut app, key::BUDGET_M_CALLS, "5");
    set_limit(&mut app, key::BUDGET_S_MINUTES, "15");
    set_limit(&mut app, key::BUDGET_L_MINUTES, "60");
    let text = crate::ui::settings::tests::screen_text(&app, 120, 40);
    for want in [
        "s tool calls      40    at least 1   hard stop at 60 calls",
        "m tool calls      5     at least 1   hard stop at 8 calls",
        "s minutes         15    at least 1   hard stop at 22m30s",
        "l minutes         60    at least 1   hard stop at 90m",
    ] {
        assert!(text.contains(want), "{want}\n{text}");
    }
}

#[test]
fn defaults_are_marked() {
    let defaults = [key::MODELS, key::AGENT_MODEL, key::MAX_BOUNCES];
    let mut app = cached(sample(), origin(&defaults));
    open(&mut app);
    let s = screen(&app);
    for k in proto::SETTINGS_KEYS {
        assert_eq!(s.is_default(k), defaults.contains(&k), "{k}");
    }
    let text = |app: &App| crate::ui::settings::tests::screen_text(app, 80, 24);
    assert!(
        text(&app).contains("enabled models  (default)"),
        "{}",
        text(&app)
    );
    to_section(&mut app, SettingsSection::Orchestrator);
    let t = text(&app);
    assert!(t.contains("model    ‹ default ›  (default)"), "{t}");
    assert!(!t.contains("‹ configured ›  (default)"), "{t}");
    to_section(&mut app, SettingsSection::Limits);
    let t = text(&app);
    assert!(t.contains("max bounces       2  (default)"), "{t}");
    assert!(!t.contains("max readers       2  (default)"), "{t}");
    // A changed value is the user's, not the default any more.
    set_limit(&mut app, key::MAX_BOUNCES, "3");
    assert!(!screen(&app).is_default(key::MAX_BOUNCES));
    assert!(!text(&app).contains("(default)"));
}

#[test]
fn the_orchestrator_picker_offers_only_enabled_models() {
    let mut app = opened();
    to_section(&mut app, SettingsSection::Orchestrator);
    assert_eq!(screen(&app).model_options(), vec![String::new()]);
    // runtime ‹ configured › → ‹ claude ›; the model resets to default.
    tap(&mut app, KeyCode::Right);
    assert_eq!(screen(&app).runtime, Some(Runtime::Claude));
    assert_eq!(
        screen(&app).model_options(),
        vec!["".to_string(), "claude-opus-5-5".into(), "claude-x".into()]
    );
    select(&mut app, 1);
    let mut seen = vec![];
    for _ in 0..3 {
        tap(&mut app, KeyCode::Right);
        seen.push(screen(&app).model.clone());
    }
    assert_eq!(seen, vec!["claude-opus-5-5", "claude-x", ""]);
    tap(&mut app, KeyCode::Left);
    assert_eq!(screen(&app).model, "claude-x");
    assert_eq!(screen(&app).doc().unwrap().orchestrator.model, "claude-x");
    // Disabled, a model leaves the picker, and the stale choice is the daemon's problem.
    to_section(&mut app, SettingsSection::Claude);
    select(&mut app, 3);
    tap(&mut app, KeyCode::Char(' '));
    assert_eq!(
        screen(&app).model_options(),
        vec!["".to_string(), "claude-opus-5-5".into()]
    );
    assert_eq!(
        screen(&app).problems(),
        vec!["orchestrator.agent.model: claude-x is not an enabled claude model"]
    );
    // Codex's `""` is its default, not a name to pick.
    to_section(&mut app, SettingsSection::Orchestrator);
    select(&mut app, 0);
    tap(&mut app, KeyCode::Right);
    assert_eq!(screen(&app).runtime, Some(Runtime::Codex));
    assert_eq!(screen(&app).model, "");
    assert_eq!(screen(&app).model_options(), vec![String::new()]);
}

#[test]
fn custom_adds_a_model_with_its_strength() {
    let mut app = opened();
    to_section(&mut app, SettingsSection::Codex);
    select(&mut app, 8);
    tap(&mut app, KeyCode::Enter);
    assert!(matches!(screen(&app).page, Some(SettingsPage::Custom(_))));
    // An empty name is refused in the dialog.
    tap(&mut app, KeyCode::Enter);
    let Some(SettingsPage::Custom(c)) = &screen(&app).page else {
        panic!("the dialog closed");
    };
    assert_eq!(c.error.as_deref(), Some("type a model name first"));
    for ch in "gpt-x".chars() {
        tap(&mut app, KeyCode::Char(ch));
    }
    tap(&mut app, KeyCode::Tab);
    tap(&mut app, KeyCode::Right);
    tap(&mut app, KeyCode::Enter);
    assert_eq!(screen(&app).page, None);
    assert_eq!(names(&app, Runtime::Codex)[8], ("gpt-x".into(), true, true));
    assert_eq!(screen(&app).selected, 8);
    let doc = screen(&app).doc().unwrap();
    assert_eq!(
        doc.models.last(),
        Some(&model(Runtime::Codex, "gpt-x", Strength::Frontier))
    );
    // A shipped name enables its row instead of adding a second.
    select(&mut app, 9);
    tap(&mut app, KeyCode::Enter);
    app.on_paste("gpt-6-luna\n".into());
    tap(&mut app, KeyCode::Enter);
    assert_eq!(names(&app, Runtime::Codex).len(), 9);
    assert_eq!(
        names(&app, Runtime::Codex)[3],
        ("gpt-6-luna".into(), true, false)
    );
    assert_eq!(screen(&app).selected, 3);
}
