//! Milestone 9.0.6 task 14, decision 36: the Settings screen's keys, its document, its
//! inline problems and warnings, and its save. Every rule is `config::settings`'s own.

use super::actions::tap;
use super::goal_form::{FixtureModel, app, doc, entry};
use super::orch::tagged;
use super::*;
use crate::app::screens::Screen;
use crate::app::settings_screen::{
    SettingsScreen, SettingsSection, hard_stop_calls, hard_stop_minutes,
};
use proto::settings::key;
use proto::{Origin, RunReply, RunRequest, SettingsDoc, SettingsReply, SettingsRequest};
use std::collections::BTreeMap;

fn model(runtime: Runtime, name: &str) -> FixtureModel {
    entry(runtime, name)
}

/// `claude-opus-5-5` (shipped) and a custom `claude-x` fast, plus Codex's default.
pub(super) fn sample() -> SettingsDoc {
    doc(vec![
        model(Runtime::Claude, "claude-opus-5-5"),
        model(Runtime::Claude, "claude-x"),
        model(Runtime::Codex, ""),
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

/// Opening the screen sends exactly one request: milestone 9.5 decision 48's tagged
/// read-only `Stats` for the app's project (`ui/settings_tuning_tests.rs` has the rest).
/// Milestone 9.8's `ListModels`, sent while the app holds no catalog, is left aside
/// (`app/models_table_tests.rs` checks it).
pub(super) fn only_the_tuning_ask(app: &App, effects: &[Effect]) {
    let dir = app.goal_project().expect("the app has a project");
    let effects: Vec<Effect> = (effects.iter())
        .filter(|e| !matches!(e, Effect::Send(ClientMsg::ListModels { .. })))
        .cloned()
        .collect();
    match effects.as_slice() {
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

/// M9.8.12 (carried from Task 11's review): the roster's strength warnings left with
/// the roster; a limits edit saves with nothing but the limits and the table.
#[test]
fn no_strength_warning_and_a_limit_saves() {
    let mut app = opened();
    for section in [SettingsSection::Models, SettingsSection::Limits] {
        to_section(&mut app, section);
        let text = crate::ui::settings::tests::screen_text(&app, 120, 40);
        assert!(!text.contains("cross-runtime reviewer"), "{text}");
    }
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
    let defaults = [key::ROLES, key::MAX_BOUNCES];
    let mut app = cached(sample(), origin(&defaults));
    open(&mut app);
    let s = screen(&app);
    for k in proto::SETTINGS_KEYS {
        assert_eq!(s.is_default(k), defaults.contains(&k), "{k}");
    }
    let text = |app: &App| crate::ui::settings::tests::screen_text(app, 80, 24);
    to_section(&mut app, SettingsSection::Limits);
    let t = text(&app);
    assert!(t.contains("max bounces       2  (default)"), "{t}");
    assert!(!t.contains("max readers       2  (default)"), "{t}");
    // A changed value is the user's, not the default any more.
    set_limit(&mut app, key::MAX_BOUNCES, "3");
    assert!(!screen(&app).is_default(key::MAX_BOUNCES));
    assert!(!text(&app).contains("(default)"));
}
