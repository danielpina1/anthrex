//! Milestone 9.8 task 10 (MR §5.2): the model picker as state, over the fixture
//! catalogs `fake-agent` serves (`crates/fake-agent/fixtures/`).

use super::{Catalogs, CustomModel, ModelPicker, Picked, PickerEntry, PickerFor};
use crate::app::Effect;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::models::{CatalogModel, CatalogSource, ModelCatalog, ModelRef, Role};
use proto::{ClientMsg, DaemonMsg, Runtime};

fn model(id: &str, label: &str, description: &str, efforts: &[&str]) -> CatalogModel {
    CatalogModel {
        id: id.into(),
        label: label.into(),
        description: description.into(),
        efforts: efforts.iter().map(|e| e.to_string()).collect(),
        default_effort: None,
        is_default: false,
    }
}

/// `fake-agent/fixtures/claude-initialize.json`, as the daemon's probe reads it.
pub(crate) fn claude_catalog() -> ModelCatalog {
    let four = ["low", "medium", "high", "max"];
    ModelCatalog {
        runtime: Runtime::Claude,
        cli_version: "2.1.290".into(),
        fetched_at: 1,
        source: CatalogSource::Live,
        models: vec![
            model(
                "claude-haiku-4-5",
                "Haiku 4.5",
                "Fastest for quick tasks",
                &[],
            ),
            model(
                "claude-sonnet-5",
                "Sonnet 5",
                "Best for everyday tasks",
                &four,
            ),
            model(
                "claude-opus-5-5",
                "Opus 5.5",
                "Most capable for complex work",
                &four,
            ),
        ],
        problem: None,
    }
}

/// `fake-agent/fixtures/codex-model-list.json` (both pages), as the daemon reads it.
pub(crate) fn codex_catalog() -> ModelCatalog {
    let with = |m: CatalogModel, default: &str, is_default: bool| CatalogModel {
        default_effort: Some(default.into()),
        is_default,
        ..m
    };
    let max = ["low", "medium", "high", "max"];
    ModelCatalog {
        runtime: Runtime::Codex,
        cli_version: "0.160.1".into(),
        fetched_at: 1,
        source: CatalogSource::Live,
        models: vec![
            with(
                model(
                    "gpt-6-luna",
                    "gpt-6 luna",
                    "Fast, low cost",
                    &["low", "medium", "high", "xhigh"],
                ),
                "low",
                false,
            ),
            with(
                model("gpt-6-sol", "gpt-6 sol", "Balanced", &max),
                "medium",
                true,
            ),
            with(
                model("gpt-6.1-sol", "gpt-6.1 sol", "Frontier", &max),
                "high",
                false,
            ),
        ],
        problem: None,
    }
}

/// Both fixture catalogs, `Live`.
pub(crate) fn fixture_catalogs() -> Catalogs {
    Catalogs {
        list: vec![claude_catalog(), codex_catalog()],
        ..Default::default()
    }
}

pub(crate) fn mref(text: &str) -> ModelRef {
    ModelRef::parse(text).unwrap()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn typed(p: &mut ModelPicker, text: &str) {
    for c in text.chars() {
        assert_eq!(p.on_key(key(KeyCode::Char(c))), None);
    }
}

fn header(runtime: Runtime, text: &str, missing: bool) -> PickerEntry {
    PickerEntry::Header {
        runtime,
        text: text.into(),
        missing,
    }
}

/// The selected entry's model, if it is one.
pub(crate) fn selected_model(p: &ModelPicker) -> Option<ModelRef> {
    match &p.entries[p.selected] {
        PickerEntry::Model { model, .. } => Some(model.clone()),
        _ => None,
    }
}

fn medium(catalogs: &Catalogs) -> ModelPicker {
    ModelPicker::new(
        PickerFor::Row(Role::ImplementerMedium),
        catalogs,
        Some(&mref("codex:gpt-6-sol")),
        None,
    )
}

#[test]
fn the_picker_lists_both_clis_then_custom() {
    let p = medium(&fixture_catalogs());
    let shape: Vec<String> = p
        .entries
        .iter()
        .map(|e| match e {
            PickerEntry::Header { text, .. } => format!("# {text}"),
            PickerEntry::Model { label, .. } => label.clone(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        [
            "# CLAUDE  (claude 2.1.290)",
            "Haiku 4.5",
            "Sonnet 5",
            "Opus 5.5",
            "# CODEX  (codex 0.160.1)",
            "gpt-6 luna",
            "gpt-6 sol",
            "gpt-6.1 sol",
            "Custom",
        ]
    );
    assert_eq!(
        p.entries[0],
        header(Runtime::Claude, "CLAUDE  (claude 2.1.290)", false)
    );
    let models: Vec<(String, String, bool)> = p
        .entries
        .iter()
        .filter_map(|e| match e {
            PickerEntry::Model {
                description,
                efforts,
                current,
                ..
            } => Some((description.clone(), efforts.clone(), *current)),
            _ => None,
        })
        .collect();
    let efforts: Vec<&str> = models.iter().map(|m| m.1.as_str()).collect();
    assert_eq!(
        efforts,
        [
            "—",
            "low … max",
            "low … max",
            "low … xhigh",
            "low … max",
            "low … max"
        ]
    );
    assert!(models[4].0.ends_with("(Codex default)"), "{:?}", models[4]);
    assert_eq!(models[4].0, "Balanced (Codex default)");
    assert!(!models[3].0.contains("default"), "{:?}", models[3]);
    let current: Vec<bool> = models.iter().map(|m| m.2).collect();
    assert_eq!(current, [false, false, false, false, true, false]);
    assert_eq!(selected_model(&p), Some(mref("codex:gpt-6-sol")));
}

#[test]
fn a_cached_or_builtin_catalog_says_so_and_a_missing_cli_is_greyed() {
    let mut catalogs = fixture_catalogs();
    catalogs.list[0].source = CatalogSource::Cached;
    catalogs.list[1].source = CatalogSource::Builtin;
    let p = medium(&catalogs);
    assert_eq!(
        p.entries[0],
        header(Runtime::Claude, "CLAUDE  (cached)", false)
    );
    assert_eq!(
        p.entries[4],
        header(Runtime::Codex, "CODEX  (built-in list)", false)
    );

    catalogs.list[1].problem = Some("codex not found".into());
    let mut p = ModelPicker::new(
        PickerFor::Row(Role::ImplementerHub),
        &catalogs,
        Some(&mref("claude:claude-opus-5-5")),
        None,
    );
    assert_eq!(
        p.entries[4],
        header(Runtime::Codex, "CODEX  codex not found", true)
    );
    assert_eq!(selected_model(&p), Some(mref("claude:claude-opus-5-5")));
    assert_eq!(p.on_key(key(KeyCode::Char('j'))), None);
    assert_eq!(
        p.entries[p.selected],
        PickerEntry::Custom,
        "codex is skipped"
    );
    assert_eq!(p.on_key(key(KeyCode::Char('k'))), None);
    assert_eq!(selected_model(&p), Some(mref("claude:claude-opus-5-5")));
    // A missing runtime's current model is not where the picker opens.
    let p = medium(&catalogs);
    assert_eq!(selected_model(&p), Some(mref("claude:claude-haiku-4-5")));
}

#[test]
fn r_asks_for_a_live_probe() {
    let mut p = medium(&fixture_catalogs());
    assert_eq!(p.on_key(key(KeyCode::Char('r'))), Some(Picked::Refresh));
    assert_eq!(p.on_key(key(KeyCode::Esc)), Some(Picked::Cancel));

    // Through the app: the Settings screen's picker on `implementer · medium`.
    let mut app = super::super::models_table::tests::opened_models(
        super::super::models_table::tests::spec_roles(),
        fixture_catalogs(),
    );
    super::super::models_table::tests::select_role(&mut app, Role::ImplementerMedium);
    super::super::models_table::tests::tap(&mut app, KeyCode::Enter);
    let effects = super::super::models_table::tests::tap(&mut app, KeyCode::Char('r'));
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::ListModels {
            runtime: None,
            refresh: true,
        })]
    );
    let picker = |app: &crate::app::App| {
        super::super::models_table::tests::table(app)
            .picker
            .clone()
            .expect("the picker stays open")
    };
    assert_eq!(selected_model(&picker(&app)), Some(mref("codex:gpt-6-sol")));

    // A live reply adds a Codex model ahead of the selected one: the entries are
    // rebuilt and the selection stays on gpt-6 sol.
    let mut codex = codex_catalog();
    codex
        .models
        .insert(0, model("gpt-6-nova", "gpt-6 nova", "New", &["low"]));
    assert!(
        app.on_daemon(DaemonMsg::Models {
            catalogs: vec![codex]
        })
        .is_empty()
    );
    let p = picker(&app);
    assert!(
        p.entries.iter().any(
            |e| matches!(e, PickerEntry::Model { label, efforts, .. } if label == "gpt-6 nova" && efforts == "low")
        ),
        "{:?}",
        p.entries
    );
    assert_eq!(selected_model(&p), Some(mref("codex:gpt-6-sol")));
    assert!(app.catalogs.received(Runtime::Codex).is_some());
}

#[test]
fn custom_asks_the_runtime_then_the_name() {
    let mut p = medium(&fixture_catalogs());
    for _ in 0..12 {
        p.on_key(key(KeyCode::Char('j')));
    }
    assert_eq!(p.entries[p.selected], PickerEntry::Custom);
    assert_eq!(p.on_key(key(KeyCode::Enter)), None);
    assert!(matches!(
        p.custom,
        Some(CustomModel {
            runtime: Runtime::Claude,
            naming: false,
            ..
        })
    ));
    assert_eq!(p.on_key(key(KeyCode::Right)), None);
    assert_eq!(p.custom.as_ref().unwrap().runtime, Runtime::Codex);
    assert_eq!(p.on_key(key(KeyCode::Enter)), None);
    assert!(p.custom.as_ref().unwrap().naming);
    typed(&mut p, "gpt-7 nova");
    assert_eq!(p.on_key(key(KeyCode::Enter)), None);
    assert_eq!(
        p.custom.as_ref().unwrap().error.as_deref(),
        Some("\"gpt-7 nova\" is not a model name (1 to 100 visible characters, no spaces)")
    );
    for _ in 0..20 {
        p.on_key(key(KeyCode::Backspace));
    }
    assert_eq!(p.custom.as_ref().unwrap().error, None, "an edit clears it");
    typed(&mut p, "gpt-7-nova");
    assert_eq!(
        p.on_key(key(KeyCode::Enter)),
        Some(Picked::Model(mref("codex:gpt-7-nova")))
    );
    // `esc` in the custom steps goes back to the list, not out of the picker.
    let mut p = medium(&fixture_catalogs());
    p.selected = p.entries.len() - 1;
    p.on_key(key(KeyCode::Enter));
    assert_eq!(p.on_key(key(KeyCode::Esc)), None);
    assert_eq!(p.custom, None);
}

#[test]
fn a_hostile_catalog_is_kept_out_of_the_entries_text() {
    let mut catalogs = fixture_catalogs();
    catalogs.list[0].models[0].label = "\u{1b}[31mOK\u{202e}".into();
    catalogs.list[0].models[0].description = format!("\u{1b}[2J{}", "d".repeat(500));
    catalogs.list[0].models[0].efforts = vec!["low".into(), "\u{1b}[31m".into(), "high".into()];
    let p = medium(&catalogs);
    // Control characters become spaces, format characters are dropped, then trimmed.
    match &p.entries[1] {
        PickerEntry::Model {
            label,
            description,
            efforts,
            ..
        } => {
            assert_eq!(label, "[31mOK");
            assert_eq!(*description, format!("[2J{}", "d".repeat(500)));
            assert_eq!(
                efforts, "low … high",
                "an effort that is not a name is dropped"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        catalogs.label(&mref("claude:claude-haiku-4-5"), false),
        "Claude · [31mOK"
    );
}

#[test]
fn the_first_entry_and_labels() {
    let catalogs = fixture_catalogs();
    let p = ModelPicker::new(
        PickerFor::Fallback(Role::Research),
        &catalogs,
        None,
        Some(PickerEntry::NoFallback),
    );
    assert_eq!(p.entries[0], PickerEntry::NoFallback);
    assert_eq!(p.selected, 0, "no fallback yet: on `none`");
    let mut p = ModelPicker::new(
        PickerFor::Goal,
        &catalogs,
        None,
        Some(PickerEntry::RoleTable(
            "role table (Claude · Opus 5.5)".into(),
        )),
    );
    assert_eq!(p.on_key(key(KeyCode::Enter)), Some(Picked::RoleTable));
    assert_eq!(
        catalogs.label(&mref("codex:gpt-6-luna"), true),
        "Codex  · gpt-6 luna"
    );
    assert_eq!(
        catalogs.label(&mref("codex:default"), false),
        "Codex · default"
    );
    assert_eq!(
        catalogs.label(&mref("claude:claude-x"), true),
        "Claude · claude-x"
    );
    assert_eq!(
        catalogs.efforts(&mref("claude:claude-haiku-4-5")),
        Vec::<String>::new()
    );
    assert_eq!(
        catalogs.unreported(&mref("claude:claude-x")).as_deref(),
        Some("not reported by claude 2.1.290")
    );
    assert_eq!(
        Catalogs::default().unreported(&mref("claude:claude-x")),
        None
    );
}
