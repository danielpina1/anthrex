//! Task M9.6.18, rulings T18-1 and T18-2: a design run's brainstormers and document
//! reviewers on `RunInfo.design_agents`, and the configured design mode on
//! `SettingsDoc.design_default`. Both are appended under protocol 17, with a serde
//! default, and left out while empty or unknown: what an earlier build sent still decodes,
//! and a run without the flow writes what it wrote before.

use super::*;
use crate::settings::{BudgetLimit, SettingsDoc, SettingsLimits};

fn agent(role: AgentRole, label: &str, state: DesignAgentStatus) -> DesignAgentInfo {
    DesignAgentInfo {
        role,
        label: label.into(),
        runtime: Runtime::Codex,
        state,
        sessions: 1,
        window_id: None,
        doc: None,
        review: None,
    }
}

#[test]
fn run_info_carries_the_design_agents() {
    for (name, text) in [
        ("m95_run_info.json", include_str!("m95_run_info.json")),
        ("m93_run_info.json", include_str!("m93_run_info.json")),
    ] {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        let json: RunInfo = serde_json::from_str(text).unwrap();
        let packed: RunInfo = rmp_serde::from_slice(&p16_bytes(&value)).unwrap();
        assert!(json.design_agents.is_empty() && packed.design_agents.is_empty());
        let again = serde_json::to_value(&json).unwrap();
        assert!(again.get("design_agents").is_none(), "{name} writes none");
    }
    let relaunched = DesignAgentInfo {
        sessions: 2,
        window_id: Some(41),
        ..agent(AgentRole::Brainstormer, "codex", DesignAgentStatus::Running)
    };
    let reviewer = DesignAgentInfo {
        doc: Some(DocKind::Spec),
        review: Some(2),
        ..agent(AgentRole::DocReviewer, "spec-r2", DesignAgentStatus::Failed)
    };
    for state in [
        DesignAgentStatus::Queued,
        DesignAgentStatus::Running,
        DesignAgentStatus::Submitted,
        DesignAgentStatus::Done,
        DesignAgentStatus::Failed,
    ] {
        both_ways(&agent(AgentRole::Brainstormer, "claude", state));
    }
    let run = RunInfo {
        design_agents: vec![relaunched, reviewer],
        ..a_run_info()
    };
    both_ways(&run);
    let json = serde_json::to_value(&run).unwrap();
    let agents = &json["design_agents"];
    assert_eq!(agents[0]["role"], "brainstormer");
    assert_eq!(agents[0]["state"], "running");
    assert_eq!(agents[0]["sessions"], 2);
    assert_eq!(agents[0]["window_id"], 41);
    assert!(agents[0].get("doc").is_none() && agents[0].get("review").is_none());
    assert_eq!(agents[1]["role"], "doc_reviewer");
    assert_eq!(agents[1]["state"], "failed");
    assert_eq!(agents[1]["doc"], "spec");
    assert_eq!(agents[1]["review"], 2);
}

fn a_settings_doc() -> SettingsDoc {
    let budget = BudgetLimit {
        tool_calls: 10,
        minutes: 5,
    };
    SettingsDoc {
        limits: SettingsLimits {
            budget_s: budget,
            budget_m: budget,
            budget_l: budget,
            stall_after_secs: 60,
            max_writers: 3,
            max_readers: 3,
            max_bounces: 2,
        },
        design_default: None,
        roles: Default::default(),
    }
}

#[test]
fn settings_carry_the_design_default() {
    let old = a_settings_doc();
    let mut value = serde_json::to_value(&old).unwrap();
    assert!(
        value.get("design_default").is_none(),
        "unknown: not written"
    );
    value.as_object_mut().unwrap().remove("design_default");
    let back: SettingsDoc = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(back.design_default, None);
    let back: SettingsDoc = rmp_serde::from_slice(&p16_bytes(&value)).unwrap();
    assert_eq!(back.design_default, None);
    for mode in [DesignMode::Full, DesignMode::Off] {
        let doc = SettingsDoc {
            design_default: Some(mode),
            ..a_settings_doc()
        };
        both_ways(&doc);
        let json = serde_json::to_value(&doc).unwrap();
        let want = if mode == DesignMode::Full {
            "full"
        } else {
            "off"
        };
        assert_eq!(json["design_default"], want);
    }
}

fn two_rows() -> crate::models::ModelTable {
    use crate::models::{ModelRef, Role, RoleChoice};
    let mut table = crate::models::ModelTable::default();
    table.rows.insert(
        Role::Reviewer,
        RoleChoice {
            model: ModelRef::parse("codex:gpt-6.1-sol").unwrap(),
            effort: Some("high".into()),
            fallback: Some(ModelRef::parse("claude:claude-opus-5-5").unwrap()),
        },
    );
    table.rows.insert(
        Role::ImplementerSmall,
        RoleChoice {
            model: ModelRef::default_of(Runtime::Claude),
            effort: None,
            fallback: None,
        },
    );
    table
}

#[test]
fn settings_doc_without_roles_decodes() {
    let doc = a_settings_doc();
    let mut value = serde_json::to_value(&doc).unwrap();
    assert!(value.get("roles").is_none(), "empty: not written");
    value.as_object_mut().unwrap().remove("roles");
    let back: SettingsDoc = serde_json::from_value(value.clone()).unwrap();
    assert!(back.roles.is_empty());
    let back: SettingsDoc = rmp_serde::from_slice(&p16_bytes(&value)).unwrap();
    assert!(back.roles.is_empty());
}

#[test]
fn settings_doc_roles_round_trip() {
    let doc = SettingsDoc {
        roles: two_rows(),
        ..a_settings_doc()
    };
    both_ways(&doc);
    let json = serde_json::to_value(&doc).unwrap();
    assert_eq!(json["roles"]["rows"]["reviewer"]["effort"], "high");
}
