//! Milestone 9.8 decision 21, M9.8.8 fix round 1 (controller ruling): a run's frozen
//! effort lists come from the `Live` and `Cached` catalogs, else, for a runtime with
//! neither, from its `Builtin` catalog or the built-in list, so a default install with
//! no discovery escalates (decision 29). Only `Live` and `Cached` validate.

use proto::models::{ModelRef, ModelTable, Role, RoleChoice};
use proto::{CatalogModel, CatalogSource, ModelCatalog, Runtime};

use super::*;

fn m(s: &str) -> ModelRef {
    ModelRef::parse(s).unwrap()
}

fn catalog(runtime: Runtime, id: &str, efforts: &[&str], source: CatalogSource) -> ModelCatalog {
    ModelCatalog {
        runtime,
        cli_version: "1.0.0".into(),
        fetched_at: 1,
        source,
        problem: None,
        models: vec![CatalogModel {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            efforts: efforts.iter().map(|e| e.to_string()).collect(),
            default_effort: None,
            is_default: false,
        }],
    }
}

fn luna_at(effort: &str) -> RunModels {
    let row = RoleChoice {
        model: m("codex:gpt-6-luna"),
        effort: Some(effort.into()),
        fallback: None,
    };
    let table = ModelTable {
        rows: [(Role::ImplementerSmall, row)].into(),
        brainstorm: None,
    };
    RunModels::resolve(&table, None)
}

#[test]
fn with_no_catalog_the_built_in_lists_fill_the_efforts() {
    let mut models = RunModels::resolve(&ModelTable::default(), None);
    assert!(models.validate(&[]).is_empty());
    let three = ["low", "medium", "high"];
    assert_eq!(models.efforts[&m("claude:claude-sonnet-5")].efforts, three);
    assert_eq!(models.efforts[&m("claude:claude-opus-5-5")].efforts, three);
    assert_eq!(models.efforts[&m("codex:default")].efforts, three);
}

#[test]
fn a_builtin_catalog_fills_efforts_but_validates_nothing() {
    let mut models = luna_at("max");
    let builtin = catalog(
        Runtime::Codex,
        "gpt-6-luna",
        &["low"],
        CatalogSource::Builtin,
    );
    assert!(models.validate(&[builtin]).is_empty());
    let choice = models.choice(Role::ImplementerSmall);
    assert_eq!(choice.effort.as_deref(), Some("max"));
    assert_eq!(models.efforts[&m("codex:gpt-6-luna")].efforts, ["low"]);
}

#[test]
fn a_live_catalog_keeps_its_runtimes_built_in_list_out() {
    let mut models = luna_at("low");
    // Codex's live catalog does not list Luna: it launches as configured, no list.
    let live = catalog(Runtime::Codex, "gpt-6-sol", &["low"], CatalogSource::Live);
    assert!(models.validate(&[live]).is_empty());
    assert!(!models.efforts.contains_key(&m("codex:gpt-6-luna")));
    // Claude has no catalog: its rows' models take the built-in lists.
    assert!(models.efforts.contains_key(&m("claude:claude-sonnet-5")));
}
