use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::run_wire::request;
use crate::settings::{
    BudgetLimit, Origin, SETTINGS_KEYS, SettingsDoc, SettingsLimits, SettingsReply,
    SettingsRequest, key,
};
use crate::{DaemonMsg, RunReply, RunRequest};

/// Ruling F2: `rmp_serde` directly (`encode` prepends a length), plus JSON.
fn both_ways<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) {
    let bytes = rmp_serde::to_vec_named(value).unwrap();
    assert_eq!(&rmp_serde::from_slice::<T>(&bytes).unwrap(), value);
    let json = serde_json::to_string(value).unwrap();
    assert_eq!(&serde_json::from_str::<T>(&json).unwrap(), value);
}

fn a_doc() -> SettingsDoc {
    SettingsDoc {
        limits: SettingsLimits {
            budget_s: BudgetLimit {
                tool_calls: 40,
                minutes: 15,
            },
            budget_m: BudgetLimit {
                tool_calls: 80,
                minutes: 30,
            },
            budget_l: BudgetLimit {
                tool_calls: 160,
                minutes: 60,
            },
            stall_after_secs: 600,
            max_writers: 3,
            max_readers: 4,
            max_bounces: 2,
        },
        design_default: None,
        roles: a_table(),
    }
}

/// Milestone 9.8: the doc carries the role table, a row and the brainstorm pair.
fn a_table() -> crate::models::ModelTable {
    use crate::models::{BrainstormChoice, ModelRef, Role, RoleChoice};
    let mut table = crate::models::ModelTable::default();
    table.rows.insert(
        Role::ImplementerSmall,
        RoleChoice {
            model: ModelRef::parse("claude:claude-opus-5-5").unwrap(),
            effort: Some("high".into()),
            fallback: Some(ModelRef::parse("codex:default").unwrap()),
        },
    );
    table.brainstorm = Some(BrainstormChoice {
        first: ModelRef::parse("claude:claude-opus-5-5").unwrap(),
        second: ModelRef::parse("codex:gpt-6-sol").unwrap(),
        effort: None,
    });
    table
}

fn an_origin() -> BTreeMap<String, Origin> {
    SETTINGS_KEYS
        .iter()
        .enumerate()
        .map(|(i, k)| {
            (
                k.to_string(),
                if i % 2 == 0 {
                    Origin::File
                } else {
                    Origin::Default
                },
            )
        })
        .collect()
}

fn every_reply() -> Vec<SettingsReply> {
    vec![
        SettingsReply::Current {
            doc: a_doc(),
            origin: an_origin(),
            path: PathBuf::from("/tmp/c/config.toml"),
        },
        SettingsReply::Saved {
            doc: a_doc(),
            origin: an_origin(),
        },
        SettingsReply::Refused {
            problems: vec!["enable at least one model".into()],
        },
    ]
}

#[test]
fn settings_messages_round_trip() {
    both_ways(&SettingsRequest::Get);
    both_ways(&SettingsRequest::Put { settings: a_doc() });
    both_ways(&RunRequest::Settings(SettingsRequest::Get));
    both_ways(&RunRequest::Settings(SettingsRequest::Put {
        settings: a_doc(),
    }));
    for reply in every_reply() {
        both_ways(&reply);
        for request_id in [None, Some(9)] {
            let run = RunReply::Settings {
                reply: Box::new(reply.clone()),
                request_id,
            };
            both_ways(&run);
            both_ways(&DaemonMsg::Run(run));
        }
    }
    // `request_id` is `#[serde(default)]`: absent decodes as `None`.
    let json = serde_json::json!({"Settings": {"reply": {"Refused": {"problems": []}}}});
    let reply: RunReply = serde_json::from_value(json).unwrap();
    assert_eq!(reply.request_id(), None);
    assert_eq!(request::SETTINGS, "settings");
}

#[test]
fn tagged_settings_reply_echoes_its_id() {
    let reply = RunReply::Settings {
        reply: Box::new(SettingsReply::Refused { problems: vec![] }),
        request_id: None,
    };
    assert_eq!(reply.request_id(), None);
    let tagged = reply.clone().tagged(Some(41));
    assert_eq!(tagged.request_id(), Some(41));
    both_ways(&DaemonMsg::Run(tagged.clone()));
    assert_eq!(tagged.tagged(None), reply);
}

fn variant_names<T: DeserializeOwned + std::fmt::Debug>() -> Vec<String> {
    let error = serde_json::from_str::<T>(r#""NoSuchVariant""#)
        .unwrap_err()
        .to_string();
    let list = error.split_once("expected one of ").unwrap().1;
    list.split(", ")
        .map(|n| {
            n.trim_start_matches('`')
                .split('`')
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn settings_variants_are_appended_last() {
    let requests = variant_names::<RunRequest>();
    // Milestone 9.2 appends `Deliver` and `Watch` after `Settings` (protocol 14,
    // `delivery_tests.rs`), milestone 9.3 `Iterate` after `Watch` (protocol 15,
    // `rounds_tests.rs`) and milestone 9.5 `McpReady` after `Iterate` (protocol 16,
    // `tuning_tests.rs`), and milestone 9.6 `DocGate` and `ShowDoc` after it (protocol
    // 17, `design_tests.rs`); `Settings` keeps its index right after `TaskDetail`.
    assert_eq!(
        requests[requests.len() - 9..requests.len() - 3],
        [
            "TaskDetail",
            "Settings",
            "Deliver",
            "Watch",
            "Iterate",
            "McpReady"
        ]
    );
    let replies = variant_names::<RunReply>();
    // Milestone 9.6 appends `Doc` after `Settings` (`design_tests.rs`).
    // Milestone 9.10 appends `Queued` after `Doc` (`profile_tests.rs`).
    assert_eq!(replies.last().map(String::as_str), Some("Queued"));
    assert_eq!(replies[replies.len() - 2], "Doc");
    assert_eq!(replies[replies.len() - 3], "Settings");
    assert_eq!(replies[replies.len() - 4], "TaskDetail");
}

#[test]
fn settings_keys_are_eleven_and_distinct() {
    assert_eq!(SETTINGS_KEYS.len(), 11);
    let mut sorted = SETTINGS_KEYS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), 11);
    // Milestone 9.8 (M9.8.12): the role table replaced the roster and the agent keys.
    assert_eq!(SETTINGS_KEYS[0], key::ROLES);
    assert_eq!(key::ROLES, "models");
    assert_eq!(SETTINGS_KEYS[10], key::MAX_BOUNCES);
    assert!(
        SETTINGS_KEYS[1..]
            .iter()
            .all(|k| k.starts_with("orchestrator."))
    );
}

#[test]
fn ranges_match_the_configs_literals() {
    use crate::settings::*;
    assert_eq!(MAX_WRITERS_RANGE, 1..=8);
    assert_eq!(MAX_READERS_RANGE, 1..=8);
    assert_eq!(MAX_BOUNCES_RANGE, 1..=5);
    assert_eq!(STALL_AFTER_SECS_RANGE, 5..=7200);
    assert_eq!(BUDGET_MIN, 1);
}
