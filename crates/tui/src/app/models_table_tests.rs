//! Milestone 9.8 task 10 (MR §5.1): the Settings screen's models table as state, on
//! the spec's table and the fixture catalogs (`model_picker_tests.rs`).

use super::{ModelsTable, RowKey, Scope, TableRow};
use crate::app::model_picker::tests::{fixture_catalogs, mref};
use crate::app::model_picker::{Catalogs, PickerEntry, PickerFor};
use crate::app::screens::Screen;
use crate::app::settings_screen::SettingsScreen;
use crate::app::{App, Effect};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::models::{BrainstormChoice, CatalogSource, HelperKind, ModelTable, Role, RoleChoice};
use proto::{ClientMsg, DaemonMsg, RunReply, RunRequest, SettingsReply, SettingsRequest};

pub(crate) fn row(model: &str, effort: Option<&str>, fallback: Option<&str>) -> RoleChoice {
    RoleChoice {
        model: mref(model),
        effort: effort.map(str::to_string),
        fallback: fallback.map(mref),
    }
}

/// MR §5.1's table (preflight F17: brainstorm's effort is set, `high`).
pub(crate) fn spec_roles() -> ModelTable {
    let mut t = ModelTable::default();
    let opus = "claude:claude-opus-5-5";
    for (role, choice) in [
        (Role::Orchestrator, row(opus, Some("high"), None)),
        (Role::Planner, row(opus, Some("high"), None)),
        (
            Role::ImplementerSmall,
            row("codex:gpt-6-luna", Some("low"), Some("codex:gpt-6-sol")),
        ),
        (
            Role::ImplementerMedium,
            row("codex:gpt-6-sol", Some("medium"), Some(opus)),
        ),
        (
            Role::ImplementerHub,
            row(opus, Some("high"), Some("codex:gpt-6.1-sol")),
        ),
        (
            Role::TestWriter,
            row("codex:gpt-6-sol", Some("medium"), None),
        ),
        (
            Role::Reviewer,
            row("codex:gpt-6.1-sol", Some("high"), Some(opus)),
        ),
        (
            Role::Research,
            row("claude:claude-sonnet-5", Some("medium"), None),
        ),
        (Role::Helpers, row("claude:claude-haiku-4-5", None, None)),
    ] {
        t.rows.insert(role, choice);
    }
    t.brainstorm = Some(BrainstormChoice {
        first: mref(opus),
        second: mref("codex:gpt-6.1-sol"),
        effort: Some("high".into()),
    });
    t
}

/// The Settings screen opened on `roles`, with `catalogs` held.
pub(crate) fn opened_models(roles: ModelTable, catalogs: Catalogs) -> App {
    let mut doc = crate::ui::settings::tests::sample();
    doc.roles = roles;
    let mut app = crate::ui::settings::tests::opened(false, doc);
    app.catalogs = catalogs;
    app
}

pub(crate) fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
}

pub(crate) fn screen(app: &App) -> &SettingsScreen {
    match &app.screen {
        Some(Screen::Settings(s)) => s,
        other => panic!("no settings screen: {other:?}"),
    }
}

pub(crate) fn table(app: &App) -> &ModelsTable {
    &screen(app).models
}

pub(crate) fn rows(app: &App) -> Vec<TableRow> {
    table(app).rows(&app.catalogs)
}

/// Moves the selection to `key`'s row with `j`/`k`.
pub(crate) fn select(app: &mut App, key: RowKey) {
    for _ in 0..20 {
        tap(app, KeyCode::Char('k'));
    }
    for _ in 0..20 {
        if rows(app)[table(app).selected].key == key {
            return;
        }
        tap(app, KeyCode::Char('j'));
    }
    panic!("{key:?} is not a row");
}

pub(crate) fn select_role(app: &mut App, role: Role) {
    select(app, RowKey::Role(role));
}

fn cells(r: &TableRow) -> (String, String, String, String) {
    (
        r.role.clone(),
        r.model.clone(),
        r.effort.clone(),
        r.fallback.clone(),
    )
}

pub(crate) fn spec() -> App {
    opened_models(spec_roles(), fixture_catalogs())
}

#[test]
fn rows_follow_the_spec_order_with_catalog_labels() {
    let app = spec();
    let got: Vec<_> = rows(&app).iter().map(cells).collect();
    let want = [
        ("orchestrator", "Claude · Opus 5.5", "high", "—"),
        ("planner", "Claude · Opus 5.5", "high", "—"),
        (
            "implementer · small",
            "Codex  · gpt-6 luna",
            "low",
            "Codex · gpt-6 sol",
        ),
        (
            "implementer · medium",
            "Codex  · gpt-6 sol",
            "medium",
            "Claude · Opus 5.5",
        ),
        (
            "implementer · hub",
            "Claude · Opus 5.5",
            "high",
            "Codex · gpt-6.1 sol",
        ),
        ("test writer", "Codex  · gpt-6 sol", "medium", "—"),
        (
            "reviewer",
            "Codex  · gpt-6.1 sol",
            "high",
            "Claude · Opus 5.5",
        ),
        ("research", "Claude · Sonnet 5", "medium", "—"),
        (
            "brainstorm",
            "Claude · Opus 5.5  +  Codex · gpt-6.1 sol",
            "high",
            "",
        ),
        ("helpers ▸", "Claude · Haiku 4.5", "—", "—"),
    ];
    let want: Vec<_> = want
        .iter()
        .map(|(a, b, c, d)| (a.to_string(), b.to_string(), c.to_string(), d.to_string()))
        .collect();
    assert_eq!(got, want);
    let r = rows(&app);
    assert_eq!(r[8].key, RowKey::Brainstorm);
    assert!(
        r.iter()
            .all(|r| !r.overridden && !r.inherited && r.warnings.is_empty())
    );
    assert_eq!(table(&app).scope, Scope::Everywhere);
}

#[test]
fn opening_asks_for_the_catalogs_only_while_there_are_none() {
    let mut app = crate::app::App::new(vec![], "/tmp".into(), Default::default());
    let id = match app.settings_fetch() {
        Effect::Send(ClientMsg::RunTagged { id, .. }) => id,
        other => panic!("{other:?}"),
    };
    app.on_daemon(DaemonMsg::Run(RunReply::Settings {
        reply: Box::new(SettingsReply::Current {
            doc: crate::ui::settings::tests::sample(),
            origin: Default::default(),
            path: "/cfg/config.toml".into(),
        }),
        request_id: Some(id),
    }));
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    let effects = app.on_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT));
    let lists: Vec<&Effect> = effects
        .iter()
        .filter(|e| matches!(e, Effect::Send(ClientMsg::ListModels { .. })))
        .collect();
    assert_eq!(
        lists,
        [&Effect::Send(ClientMsg::ListModels {
            runtime: None,
            refresh: false
        })]
    );
    tap(&mut app, KeyCode::Esc);
    app.catalogs = fixture_catalogs();
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    let effects = app.on_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Send(ClientMsg::ListModels { .. }))),
        "{effects:?}"
    );
}

#[test]
fn helpers_expand_to_six_kinds_same_as_helpers_until_set() {
    let mut app = spec();
    select_role(&mut app, Role::Helpers);
    tap(&mut app, KeyCode::Char(' '));
    let r = rows(&app);
    assert_eq!(r.len(), 16);
    assert_eq!(r[9].role, "helpers ▾");
    let kinds: Vec<&str> = r[10..].iter().map(|r| r.role.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "  run name",
            "  triage",
            "  size check",
            "  check summary",
            "  blocked reason",
            "  ci summary"
        ]
    );
    assert!(
        r[10..].iter().all(|r| r.model == "same as helpers"),
        "{r:?}"
    );

    select_role(&mut app, Role::Helper(HelperKind::RunName));
    tap(&mut app, KeyCode::Enter);
    let picker = table(&app).picker.clone().expect("the picker");
    assert_eq!(
        picker.target,
        PickerFor::Row(Role::Helper(HelperKind::RunName))
    );
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Enter);
    assert_eq!(table(&app).picker, None);
    let r = rows(&app);
    assert_eq!(r[10].model, "Claude · Sonnet 5");
    assert_eq!(r[11].model, "same as helpers");
    let doc = screen(&app).doc().expect("the doc saves");
    assert_eq!(
        doc.roles.rows[&Role::Helper(HelperKind::RunName)].model,
        mref("claude:claude-sonnet-5")
    );
    assert!(screen(&app).dirty());

    // `space` closes them again; the selection stays on `helpers`.
    tap(&mut app, KeyCode::Char(' '));
    assert_eq!(rows(&app).len(), 10);
    assert_eq!(rows(&app)[table(&app).selected].role, "helpers ▸");
}

pub(crate) fn repo_requests(effects: &[Effect]) -> Vec<(u64, SettingsRequest)> {
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

#[test]
fn this_repo_scope_marks_overrides_and_dims_inherited() {
    let mut app = spec();
    let project = app.goal_project().expect("the app has a project");
    let sent = repo_requests(&tap(&mut app, KeyCode::Right));
    assert_eq!(
        sent.iter().map(|s| &s.1).collect::<Vec<_>>(),
        [&SettingsRequest::RepoModels {
            project: project.clone()
        }]
    );
    assert_eq!(table(&app).scope, Scope::Repo);
    let mut repo = ModelTable::default();
    repo.rows.insert(
        Role::Reviewer,
        row("claude:claude-sonnet-5", Some("high"), None),
    );
    app.on_daemon(DaemonMsg::Run(RunReply::Settings {
        reply: Box::new(SettingsReply::RepoModels {
            project: project.clone(),
            table: repo,
            path: "/data/repos/tmp-1234/models.toml".into(),
        }),
        request_id: Some(sent[0].0),
    }));
    let r = rows(&app);
    let overridden: Vec<&str> = (r.iter().filter(|r| r.overridden))
        .map(|r| r.role.as_str())
        .collect();
    assert_eq!(overridden, ["reviewer"]);
    assert!(
        r.iter()
            .filter(|r| r.role != "reviewer")
            .all(|r| r.inherited)
    );
    assert!(!r[6].inherited);
    assert_eq!(r[6].model, "Claude · Sonnet 5");
    assert_eq!(
        r[5].model, "Codex  · gpt-6 sol",
        "inherited from everywhere"
    );
    assert!(!screen(&app).dirty());

    // Leaving and coming back asks nothing more.
    tap(&mut app, KeyCode::Left);
    assert_eq!(table(&app).scope, Scope::Everywhere);
    assert!(repo_requests(&tap(&mut app, KeyCode::Right)).is_empty());

    select_role(&mut app, Role::Reviewer);
    tap(&mut app, KeyCode::Char('x'));
    let r = rows(&app);
    assert!(r.iter().all(|r| r.inherited && !r.overridden));
    assert_eq!(r[6].model, "Codex  · gpt-6.1 sol");
    assert!(screen(&app).dirty());

    tap(&mut app, KeyCode::Left);
    select_role(&mut app, Role::ImplementerSmall);
    tap(&mut app, KeyCode::Char('x'));
    let small = &rows(&app)[2];
    assert_eq!(
        (
            small.model.as_str(),
            small.effort.as_str(),
            small.fallback.as_str()
        ),
        ("Claude · Sonnet 5", "low", "—"),
        "the built-in"
    );
    assert!(
        !screen(&app)
            .models
            .global
            .rows
            .contains_key(&Role::ImplementerSmall)
    );
}

#[test]
fn e_cycles_the_models_efforts_and_default() {
    let mut app = spec();
    select_role(&mut app, Role::ImplementerSmall);
    let mut seen = Vec::new();
    for _ in 0..5 {
        tap(&mut app, KeyCode::Char('e'));
        let small = table(&app).global.rows[&Role::ImplementerSmall].clone();
        seen.push((rows(&app)[2].effort.clone(), small.effort));
    }
    let shown: Vec<&str> = seen.iter().map(|s| s.0.as_str()).collect();
    assert_eq!(shown, ["medium", "high", "xhigh", "—", "low"]);
    assert_eq!(seen[3].1, None, "default is no effort");

    select_role(&mut app, Role::Helpers);
    let before = table(&app).clone();
    tap(&mut app, KeyCode::Char('e'));
    assert_eq!(*table(&app), before, "Haiku 4.5 offers no effort");

    // Brainstorm cycles its first model's efforts.
    select(&mut app, RowKey::Brainstorm);
    tap(&mut app, KeyCode::Char('e'));
    assert_eq!(rows(&app)[8].effort, "max");
}

#[test]
fn an_effort_not_offered_and_an_unreported_model_warn() {
    let mut roles = spec_roles();
    roles.rows.get_mut(&Role::ImplementerSmall).unwrap().effort = Some("max".into());
    roles
        .rows
        .insert(Role::Research, row("claude:claude-x", Some("medium"), None));
    let app = opened_models(roles.clone(), fixture_catalogs());
    let r = rows(&app);
    assert_eq!(r[2].warnings, ["⚠ effort 'max' not offered"]);
    assert_eq!(r[7].warnings, ["⚠ not reported by claude 2.1.290"]);
    assert_eq!(r[7].model, "Claude · claude-x");
    let warned: Vec<usize> = (0..r.len())
        .filter(|i| !r[*i].warnings.is_empty())
        .collect();
    assert_eq!(warned, [2, 7]);

    // Decision 21: a built-in list is a guess and warns about nothing.
    let mut catalogs = fixture_catalogs();
    for c in &mut catalogs.list {
        c.source = CatalogSource::Builtin;
    }
    let app = opened_models(roles.clone(), catalogs);
    assert!(rows(&app).iter().all(|r| r.warnings.is_empty()));
    // Claude's alone built-in: research is no longer warned, the Codex row still is.
    let mut catalogs = fixture_catalogs();
    catalogs.list[0].source = CatalogSource::Builtin;
    let app = opened_models(roles, catalogs);
    let r = rows(&app);
    assert!(r[7].warnings.is_empty());
    assert_eq!(r[2].warnings, ["⚠ effort 'max' not offered"]);
}

#[test]
fn enter_opens_the_picker_on_the_current_model() {
    let mut app = spec();
    select_role(&mut app, Role::ImplementerMedium);
    tap(&mut app, KeyCode::Enter);
    let p = table(&app).picker.clone().expect("the picker");
    assert_eq!(p.target, PickerFor::Row(Role::ImplementerMedium));
    assert_eq!(
        crate::app::model_picker::tests::selected_model(&p),
        Some(mref("codex:gpt-6-sol"))
    );
    tap(&mut app, KeyCode::Esc);
    assert_eq!(table(&app).picker, None, "esc cancels the picker only");
    assert!(app.screen.is_some());

    tap(&mut app, KeyCode::Char('f'));
    let p = table(&app).picker.clone().expect("the fallback picker");
    assert_eq!(p.target, PickerFor::Fallback(Role::ImplementerMedium));
    assert_eq!(p.entries[0], PickerEntry::NoFallback);
    assert_eq!(
        crate::app::model_picker::tests::selected_model(&p),
        Some(mref("claude:claude-opus-5-5"))
    );
    // `none` clears the fallback.
    for _ in 0..10 {
        tap(&mut app, KeyCode::Char('k'));
    }
    tap(&mut app, KeyCode::Enter);
    assert_eq!(rows(&app)[3].fallback, "—");
    assert_eq!(
        table(&app).global.rows[&Role::ImplementerMedium].fallback,
        None
    );
}

#[test]
fn enter_on_brainstorm_picks_its_first_then_its_second() {
    let mut app = spec();
    select(&mut app, RowKey::Brainstorm);
    tap(&mut app, KeyCode::Enter);
    assert_eq!(
        table(&app).picker.as_ref().map(|p| p.target),
        Some(PickerFor::Brainstorm(0))
    );
    tap(&mut app, KeyCode::Char('k'));
    tap(&mut app, KeyCode::Enter);
    let p = table(&app).picker.clone().expect("the second pick");
    assert_eq!(p.target, PickerFor::Brainstorm(1));
    assert_eq!(
        crate::app::model_picker::tests::selected_model(&p),
        Some(mref("codex:gpt-6.1-sol"))
    );
    tap(&mut app, KeyCode::Char('k'));
    tap(&mut app, KeyCode::Enter);
    assert_eq!(table(&app).picker, None);
    assert_eq!(
        rows(&app)[8].model,
        "Claude · Sonnet 5  +  Codex · gpt-6 sol"
    );
    // A model change keeps an effort the new model offers.
    assert_eq!(rows(&app)[8].effort, "high");
}

#[test]
fn a_new_model_keeps_an_offered_effort_and_drops_one_it_lacks() {
    let mut app = spec();
    select_role(&mut app, Role::ImplementerSmall);
    tap(&mut app, KeyCode::Char('e'));
    tap(&mut app, KeyCode::Char('e'));
    tap(&mut app, KeyCode::Char('e'));
    assert_eq!(rows(&app)[2].effort, "xhigh");
    // gpt-6 luna → gpt-6 sol (no `xhigh`): its default effort, `medium`.
    tap(&mut app, KeyCode::Enter);
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Enter);
    assert_eq!(rows(&app)[2].model, "Codex  · gpt-6 sol");
    assert_eq!(rows(&app)[2].effort, "medium");
}

#[test]
fn w_saves_the_scope() {
    let mut app = spec();
    let project = app.goal_project().unwrap();
    let ask = repo_requests(&tap(&mut app, KeyCode::Right))[0].0;
    app.on_daemon(DaemonMsg::Run(RunReply::Settings {
        reply: Box::new(SettingsReply::RepoModels {
            project: project.clone(),
            table: ModelTable::default(),
            path: "/data/repos/tmp-1234/models.toml".into(),
        }),
        request_id: Some(ask),
    }));
    assert!(repo_requests(&tap(&mut app, KeyCode::Char('w'))).is_empty());
    assert_eq!(
        app.toast_text(),
        Some(crate::app::settings_screen::NO_CHANGES)
    );
    select_role(&mut app, Role::Research);
    tap(&mut app, KeyCode::Char('e'));
    let mut want = ModelTable::default();
    want.rows.insert(
        Role::Research,
        row("claude:claude-sonnet-5", Some("high"), None),
    );
    let sent = repo_requests(&tap(&mut app, KeyCode::Char('w')));
    assert_eq!(
        sent.iter().map(|s| &s.1).collect::<Vec<_>>(),
        [&SettingsRequest::PutRepoModels {
            project: project.clone(),
            table: want.clone()
        }]
    );
    assert!(screen(&app).dirty(), "until the daemon says it saved");
    app.on_daemon(DaemonMsg::Run(RunReply::Settings {
        reply: Box::new(SettingsReply::RepoSaved {
            project,
            table: want,
        }),
        request_id: Some(sent[0].0),
    }));
    assert!(!screen(&app).dirty());
    assert_eq!(
        screen(&app).outcome,
        Some(crate::app::settings_screen::SaveOutcome::Saved)
    );

    // `everywhere`: the global table rides in the screen's `Settings(Put)`.
    tap(&mut app, KeyCode::Left);
    select_role(&mut app, Role::Research);
    tap(&mut app, KeyCode::Char('x'));
    let sent = repo_requests(&tap(&mut app, KeyCode::Char('w')));
    match &sent[..] {
        [(_, SettingsRequest::Put { settings })] => {
            assert!(!settings.roles.rows.contains_key(&Role::Research));
            assert_eq!(settings.roles.rows.len(), spec_roles().rows.len() - 1);
            assert_eq!(settings.limits, crate::ui::settings::tests::sample().limits);
        }
        other => panic!("{other:?}"),
    }
}
