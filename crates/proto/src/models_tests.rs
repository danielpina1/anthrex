//! Milestone 9.8 (M9.8.2, M9.8.4): the role table's types and the protocol-19 wire
//! messages that carry catalogs, the table and the effort string.

use std::collections::BTreeMap;

use serde_json::json;

use crate::delivery::tests::variant_names;
use crate::models::{Role, valid_model_id};
use crate::*;

fn both<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) {
    let back: T = serde_json::from_value(serde_json::to_value(v).unwrap()).unwrap();
    assert_eq!(&back, v);
    let back: T = rmp_serde::from_slice(&rmp_serde::to_vec_named(v).unwrap()).unwrap();
    assert_eq!(&back, v);
}

fn catalog() -> ModelCatalog {
    ModelCatalog {
        runtime: Runtime::Codex,
        cli_version: "0.160.1".into(),
        fetched_at: 1_780_000_000,
        source: CatalogSource::Live,
        models: vec![CatalogModel {
            id: "gpt-6-sol".into(),
            resolved: None,
            label: "gpt-6 sol".into(),
            description: "Balanced".into(),
            efforts: vec!["low".into(), "medium".into(), "high".into(), "max".into()],
            default_effort: Some("medium".into()),
            is_default: true,
        }],
        problem: None,
    }
}

/// M9.8.14: a protocol-18 peer's route still carries `strength`; it decodes (JSON and
/// MessagePack) with the key ignored, and a route is written without it.
#[test]
fn a_protocol_18_route_with_strength_decodes() {
    let old = json!({"runtime": "codex", "model": "", "strength": "standard", "effort": "medium"});
    let want = Route {
        runtime: Runtime::Codex,
        model: String::new(),
        effort: Effort::MEDIUM,
    };
    let route: Route = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(route, want);
    let back: Route = rmp_serde::from_slice(&rmp_serde::to_vec_named(&old).unwrap()).unwrap();
    assert_eq!(back, want);
    let written = serde_json::to_value(&want).unwrap();
    assert_eq!(
        written,
        json!({"runtime": "codex", "model": "", "effort": "medium"})
    );
    both(&want);
}

#[test]
fn effort_is_a_string_and_a_protocol_18_value_decodes() {
    let old = json!({"runtime": "claude", "model": "claude-opus-5-5", "strength": "frontier", "effort": "high"});
    let route: Route = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(route.effort, Effort::HIGH);
    // MessagePack, as a protocol-18 peer wrote the enum: its variant name.
    let back: Route = rmp_serde::from_slice(&rmp_serde::to_vec_named(&old).unwrap()).unwrap();
    assert_eq!(back, route);
    let xhigh: Effort = serde_json::from_value(json!("xhigh")).unwrap();
    assert_eq!(xhigh.as_str(), "xhigh");
    assert!(Effort::DEFAULT.is_default() && !Effort::LOW.is_default());
    assert!(
        Effort::DEFAULT < Effort::LOW
            && Effort::LOW < Effort::MEDIUM
            && Effort::MEDIUM < Effort::HIGH
            && Effort::HIGH < xhigh
    );
    assert_eq!(Effort::new("low"), Effort::LOW);
    assert_eq!(Effort::of(None), Effort::DEFAULT);
    assert_eq!(Effort::of(Some("high")), Effort::HIGH);
    assert_eq!(Effort::DEFAULT.to_string(), "default");
    assert_eq!(Effort::HIGH.to_string(), "high");
}

#[test]
fn catalogs_round_trip_and_problem_is_left_out_while_none() {
    both(&catalog());
    assert!(
        serde_json::to_value(catalog())
            .unwrap()
            .get("problem")
            .is_none()
    );
    let mut missing = catalog();
    missing.source = CatalogSource::Builtin;
    missing.problem = Some("codex not found".into());
    both(&missing);
}

#[test]
fn list_models_and_models_round_trip() {
    both(&ClientMsg::ListModels {
        runtime: Some(Runtime::Claude),
        refresh: true,
    });
    both(&ClientMsg::ListModels {
        runtime: None,
        refresh: false,
    });
    both(&DaemonMsg::Models {
        catalogs: vec![catalog()],
    });
}

#[test]
fn the_new_variants_are_appended_last() {
    assert_eq!(
        variant_names::<ClientMsg>().last().map(String::as_str),
        Some("ListModels")
    );
    assert_eq!(
        variant_names::<DaemonMsg>().last().map(String::as_str),
        Some("Models")
    );
    let requests = variant_names::<SettingsRequest>();
    assert_eq!(
        requests[requests.len() - 2..],
        ["RepoModels", "PutRepoModels"]
    );
    let replies = variant_names::<SettingsReply>();
    assert_eq!(replies[replies.len() - 2..], ["RepoModels", "RepoSaved"]);
}

#[test]
fn orchestrator_choice_effort_defaults_to_none() {
    let old: OrchestratorChoice =
        serde_json::from_value(json!({"runtime": "codex", "model": "gpt-6-sol"})).unwrap();
    assert_eq!(old.effort, None);
    both(&OrchestratorChoice {
        runtime: Runtime::Codex,
        model: Some("gpt-6-sol".into()),
        effort: Some("xhigh".into()),
    });
}

#[test]
fn repository_requests_round_trip() {
    let mut table = ModelTable::default();
    table.rows.insert(
        Role::Reviewer,
        RoleChoice {
            model: ModelRef::parse("codex:gpt-6.1-sol").unwrap(),
            effort: None,
            fallback: None,
        },
    );
    both(&SettingsRequest::RepoModels {
        project: "/p".into(),
    });
    both(&SettingsRequest::PutRepoModels {
        project: "/p".into(),
        table: table.clone(),
    });
    both(&SettingsReply::RepoModels {
        project: "/p".into(),
        table: table.clone(),
        path: "/d/repos/p-1234abcd/models.toml".into(),
        problems: vec![],
    });
    // M9.8.12 fix round 1 (I2): the rows the daemon could not read travel with it.
    both(&SettingsReply::RepoModels {
        project: "/p".into(),
        table: table.clone(),
        path: "/d/repos/p-1234abcd/models.toml".into(),
        problems: vec!["/d/models.toml: models.reviewer.model: expected a string".into()],
    });
    both(&SettingsReply::RepoSaved {
        project: "/p".into(),
        table,
    });
}

#[test]
fn roles_print_and_parse_their_keys() {
    let all: Vec<Role> = Role::all().collect();
    assert_eq!(all.len(), 15);
    for role in all {
        assert_eq!(Role::parse_key(&role.key()), Some(role), "{role:?}");
    }
    assert_eq!(Role::ImplementerMedium.key(), "implementer.medium");
    assert_eq!(
        Role::Helper(HelperKind::CiSummary).key(),
        "helpers.ci_summary"
    );
    assert_eq!(Role::parse_key("helpers.nope"), None);
}

#[test]
fn a_model_table_is_a_json_map_keyed_by_role() {
    let table = ModelTable {
        rows: BTreeMap::from([
            (
                Role::ImplementerSmall,
                RoleChoice {
                    model: ModelRef::parse("codex:gpt-6-sol").unwrap(),
                    effort: None,
                    fallback: None,
                },
            ),
            (
                Role::Reviewer,
                RoleChoice {
                    model: ModelRef::default_of(Runtime::Claude),
                    effort: Some("high".into()),
                    fallback: None,
                },
            ),
        ]),
        brainstorm: None,
    };
    let value = serde_json::to_value(&table).unwrap();
    assert_eq!(
        value,
        json!({"rows": {
            "implementer.small": {"model": "codex:gpt-6-sol"},
            "reviewer": {"model": "claude:default", "effort": "high"},
        }})
    );
    let back: ModelTable = serde_json::from_value(value).unwrap();
    assert_eq!(back, table);
}

/// M9.8.13 fix round 1 (I1): the model-name rule, shared by `ModelRef::parse` and a
/// user's route: 1 to 100 visible characters, no whitespace, control or hidden-format
/// character, and no leading `-` (a CLI would read it as an option).
#[test]
fn a_model_name_never_starts_with_a_dash() {
    for bad in ["", "a b", "a\n", "a\u{200b}", "-m", "--model"] {
        assert!(!valid_model_id(bad), "{bad:?}");
    }
    assert!(!valid_model_id(&"g".repeat(101)));
    for good in ["gpt-6-sol", "claude-opus-5-5", "a-", &"g".repeat(100)] {
        assert!(valid_model_id(good), "{good:?}");
    }
    let refused = ModelRef::parse("codex:-m").unwrap_err();
    assert_eq!(
        refused,
        "\"-m\" is not a model name (it may not start with -)"
    );
}

/// The real Claude 2.1.280 `initialize` catalog (`fake-agent/fixtures/claude-initialize.json`):
/// entries by alias, each with its resolved model; Haiku offers no effort.
fn real_claude() -> ModelCatalog {
    let five = ["low", "medium", "high", "xhigh", "max"];
    let entry = |id: &str, resolved: &str, efforts: &[&str]| CatalogModel {
        id: id.into(),
        resolved: Some(resolved.into()),
        label: id.into(),
        description: String::new(),
        efforts: efforts.iter().map(|e| e.to_string()).collect(),
        default_effort: None,
        is_default: id == "default",
    };
    ModelCatalog {
        runtime: Runtime::Claude,
        cli_version: "2.1.280".into(),
        fetched_at: 1,
        source: CatalogSource::Live,
        models: vec![
            entry("default", "claude-opus-5-5[1m]", &five),
            entry("opus[1m]", "claude-opus-5-5[1m]", &five),
            entry("claude-fable-5-1[1m]", "claude-fable-5-1", &five),
            entry("sonnet", "claude-sonnet-5", &five),
            entry("haiku", "claude-haiku-4-5-20251001", &[]),
        ],
        problem: None,
    }
}

/// Real-CLI manual check fix (decision 1): a built-in id finds the alias entry the CLI
/// resolves to it, past a `[1m]` tag and a `-YYYYMMDD` date; the default entry only when
/// nothing else matches.
#[test]
fn a_built_in_id_finds_the_alias_entry_by_its_resolved_model() {
    let c = real_claude();
    let found = |text: &str| {
        c.find(&ModelRef::parse(text).unwrap())
            .map(|m| m.id.as_str())
    };
    assert_eq!(found("claude:claude-opus-5-5"), Some("opus[1m]"));
    assert_eq!(found("claude:claude-opus-5-5[1m]"), Some("opus[1m]"));
    assert_eq!(found("claude:claude-sonnet-5"), Some("sonnet"));
    assert_eq!(found("claude:claude-haiku-4-5"), Some("haiku"));
    assert_eq!(found("claude:claude-haiku-4-5-20251001"), Some("haiku"));
    assert_eq!(
        found("claude:claude-fable-5-1"),
        Some("claude-fable-5-1[1m]")
    );
    assert_eq!(found("claude:sonnet"), Some("sonnet"));
    assert_eq!(found("claude:default"), Some("default"));
    assert_eq!(found("claude:claude-opus-5"), None);
    assert_eq!(found("codex:claude-opus-5-5"), None);
}

/// Decision 2: one model, however the row spells it.
#[test]
fn the_canonical_id_strips_the_context_tag_and_the_date() {
    use crate::models::{base_model_id, canonical_id};
    assert_eq!(base_model_id("claude-opus-5-5[1m]"), "claude-opus-5-5");
    assert_eq!(
        base_model_id("claude-haiku-4-5-20251001"),
        "claude-haiku-4-5"
    );
    assert_eq!(base_model_id("gpt-6.1-sol"), "gpt-6.1-sol");
    assert_eq!(base_model_id("model-2025"), "model-2025");
    let c = [real_claude()];
    let canon = |text: &str| canonical_id(&c, &ModelRef::parse(text).unwrap());
    assert_eq!(canon("claude:opus[1m]"), "claude-opus-5-5");
    assert_eq!(canon("claude:default"), "claude-opus-5-5");
    assert_eq!(canon("claude:claude-opus-5-5"), "claude-opus-5-5");
    assert_eq!(canon("claude:haiku"), "claude-haiku-4-5");
    assert_eq!(canon("claude:claude-opus-5"), "claude-opus-5");
    assert_eq!(canon("codex:gpt-6-sol"), "gpt-6-sol");
}

#[test]
fn a_catalog_entry_keeps_its_resolved_model_on_the_wire() {
    both(&real_claude());
    let plain = catalog();
    assert!(
        serde_json::to_value(&plain).unwrap()["models"][0]
            .get("resolved")
            .is_none()
    );
}

/// Decision 4: `xhigh` sits between `high` and `max`; escalation climbs only the
/// ordered efforts, so Codex's `ultra` is off the ladder.
#[test]
fn xhigh_orders_between_high_and_max_and_ultra_is_off_the_ladder() {
    let (xhigh, max, ultra) = (
        Effort::new("xhigh"),
        Effort::new("max"),
        Effort::new("ultra"),
    );
    assert!(Effort::new("minimal") < Effort::LOW);
    assert!(Effort::HIGH < xhigh && xhigh < max && max < ultra);
    assert_eq!(Effort::ladder_rank("xhigh"), Some(4));
    assert_eq!(Effort::ladder_rank("minimal"), Some(0));
    assert_eq!(Effort::ladder_rank("max"), Some(5));
    assert_eq!(Effort::ladder_rank("ultra"), None);
}
