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
            label: "gpt-6 sol".into(),
            description: "Balanced".into(),
            efforts: vec!["low".into(), "medium".into(), "high".into(), "max".into()],
            default_effort: Some("medium".into()),
            is_default: true,
        }],
        problem: None,
    }
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
