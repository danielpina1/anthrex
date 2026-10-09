//! Real-CLI manual check fix (2026-10-09): the role table against the catalogs the real
//! CLIs report (`fake-agent/fixtures/`, captured from claude 2.1.280 and codex 0.160.1
//! and parsed by the probes' own parsers). Claude lists aliases (`opus[1m]`, `sonnet`,
//! `haiku`) with the model each resolves to; Codex offers `ultra` past `max`.

use proto::models::{ModelRef, ModelTable, Role, RoleChoice};
use proto::{CatalogSource, Effort, ModelCatalog, Route, Runtime};

use super::*;
use crate::run::role_step::{escalate, steps};

fn m(s: &str) -> ModelRef {
    ModelRef::parse(s).unwrap()
}

fn live(runtime: Runtime, models: Vec<proto::CatalogModel>) -> ModelCatalog {
    ModelCatalog {
        runtime,
        cli_version: "real".into(),
        fetched_at: 1,
        source: CatalogSource::Live,
        models,
        problem: None,
    }
}

/// Both real catalogs, as the probes parse the fixtures.
pub(super) fn real_catalogs() -> Vec<ModelCatalog> {
    let claude: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fake-agent/fixtures/claude-initialize.json"
    ))
    .unwrap();
    let codex: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fake-agent/fixtures/codex-model-list.json"
    ))
    .unwrap();
    let codex_models = (codex.as_array().unwrap().iter())
        .flat_map(|page| crate::models::codex_probe::parse_models(&page["data"]))
        .collect();
    vec![
        live(
            Runtime::Claude,
            crate::models::claude_probe::parse_models(&claude["models"]),
        ),
        live(Runtime::Codex, codex_models),
    ]
}

fn at(model: &str, effort: &str) -> Route {
    RunModels::route_of(&m(model), (!effort.is_empty()).then_some(effort))
}

fn one_row(role: Role, model: &str, effort: Option<&str>, fallback: Option<&str>) -> RunModels {
    let row = RoleChoice {
        model: m(model),
        effort: effort.map(Into::into),
        fallback: fallback.map(m),
    };
    let table = ModelTable {
        rows: [(role, row)].into(),
        brainstorm: None,
    };
    let mut models = RunModels::resolve(&table, None);
    models.validate(&real_catalogs());
    models
}

/// D1, D2: the stock built-in table names full ids; the real Claude catalog lists
/// aliases. Every row finds its entry, keeps its effort, and gets an effort ladder.
#[test]
fn the_stock_table_finds_every_claude_row_in_the_real_catalog() {
    let mut models = RunModels::resolve(&ModelTable::default(), None);
    let lines = models.validate(&real_catalogs());
    // Gate fix B1: the built-in research row is effortless on Haiku, which reports no
    // effort, so a stock install logs nothing. No row is "not reported".
    assert_eq!(lines, Vec::<String>::new());
    let five = ["low", "medium", "high", "xhigh", "max"];
    assert_eq!(models.efforts[&m("claude:claude-opus-5-5")].efforts, five);
    assert_eq!(models.efforts[&m("claude:claude-sonnet-5")].efforts, five);
    // Haiku reports no effort: listed, with an empty list.
    assert!(
        models.efforts[&m("claude:claude-haiku-4-5")]
            .efforts
            .is_empty()
    );
    assert_eq!(
        models.choice(Role::ImplementerMedium).effort.as_deref(),
        Some("medium")
    );
    // Escalation climbs the medium implementer's own model, xhigh before max.
    let medium = models.route(Role::ImplementerMedium);
    assert_eq!(
        steps(&models, Role::ImplementerMedium, &medium)[..3],
        [
            at("claude:claude-sonnet-5", "high"),
            at("claude:claude-sonnet-5", "xhigh"),
            at("claude:claude-sonnet-5", "max"),
        ]
    );
}

/// D3: `opus[1m]`, `default` and `claude-opus-5-5` are one model, so a reviewer on
/// `claude:opus[1m]` does not review an author on `claude-opus-5-5`.
#[test]
fn an_alias_and_its_resolved_model_are_the_same_model() {
    let models = one_row(
        Role::Reviewer,
        "claude:opus[1m]",
        Some("high"),
        Some("codex:gpt-6-sol"),
    );
    let author = at("claude:claude-opus-5-5", "high");
    assert!(models.same_model(&at("claude:opus[1m]", ""), &author));
    assert!(models.same_model(&at("claude:default", ""), &author));
    assert!(models.same_model(&at("claude:claude-opus-5-5[1m]", ""), &author));
    assert!(!models.same_model(&at("claude:sonnet", ""), &author));
    let (route, line) = models.reviewer_route(&author);
    assert_eq!(route, at("codex:gpt-6-sol", ""));
    assert_eq!(line, None);
    // The alias's efforts serve the full id's route.
    assert_eq!(
        models
            .efforts_of(&m("claude:claude-opus-5-5"))
            .map(|e| e.efforts.len()),
        Some(5)
    );
}

/// Decision 4: Codex offers `ultra` past `max`; escalation stops at `max`.
#[test]
fn escalation_never_steps_into_ultra() {
    let models = one_row(
        Role::ImplementerMedium,
        "codex:gpt-6.1-sol",
        Some("high"),
        None,
    );
    let role = Role::ImplementerMedium;
    assert_eq!(
        steps(&models, role, &at("codex:gpt-6.1-sol", "high")),
        [
            at("codex:gpt-6.1-sol", "xhigh"),
            at("codex:gpt-6.1-sol", "max")
        ]
    );
    assert_eq!(
        escalate(&models, role, &at("codex:gpt-6.1-sol", "max"), &[]),
        None
    );
    // A hand-picked `ultra` is the top: nothing above it.
    assert_eq!(
        escalate(&models, role, &at("codex:gpt-6.1-sol", "ultra"), &[]),
        None
    );
    // From the model's default (`low`), up to `max` and no further.
    let from_default = steps(
        &models,
        role,
        &RunModels::route_of(&m("codex:gpt-6.1-sol"), None),
    );
    assert_eq!(
        from_default
            .iter()
            .map(|r| r.effort.as_str())
            .collect::<Vec<_>>(),
        ["medium", "high", "xhigh", "max"]
    );
    assert!(
        from_default
            .iter()
            .all(|r| r.effort != Effort::new("ultra"))
    );
}

/// Decision 2 in escalation: a route on the fallback's alias climbs the fallback's list
/// and never re-takes the fallback.
#[test]
fn a_route_on_the_fallbacks_alias_counts_as_the_fallback() {
    let models = one_row(
        Role::ImplementerMedium,
        "codex:gpt-6-sol",
        Some("medium"),
        Some("claude:claude-opus-5-5"),
    );
    assert_eq!(
        steps(
            &models,
            Role::ImplementerMedium,
            &at("claude:opus[1m]", "high")
        ),
        [at("claude:opus[1m]", "xhigh"), at("claude:opus[1m]", "max")]
    );
}
