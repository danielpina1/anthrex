//! Milestone 9.8 (M9.8.2): the role table's types. The wire tests come in M9.8.4.

use std::collections::BTreeMap;

use serde_json::json;

use crate::models::{HelperKind, ModelRef, ModelTable, Role, RoleChoice};
use crate::types::Runtime;

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
