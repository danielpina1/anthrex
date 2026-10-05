//! Milestone 9.6 task M9.6.6: a brainstormer's and a document reviewer's `anthrex mcp`
//! argv, and ruling T1-O3's `--agent-label`, which only a design agent's argv names.

use super::*;
use std::path::{Path, PathBuf};

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn target(role: AgentRole, label: Option<&str>) -> McpTarget {
    McpTarget {
        role,
        run_id: "r-3f9a".into(),
        task_id: None,
        scout_id: None,
        epic: None,
        chain: None,
        lane: None,
        agent_label: label.map(String::from),
    }
}

/// The label follows the run and precedes `--window`, when it is set.
#[test]
fn agent_label_appears_in_argv_when_set_and_is_absent_otherwise() {
    assert_eq!(
        mcp_args(
            &target(AgentRole::Brainstormer, Some("codex")),
            4,
            Path::new("/s")
        ),
        Some(strs(&[
            "mcp",
            "--role",
            "brainstormer",
            "--run",
            "r-3f9a",
            "--agent-label",
            "codex",
            "--window",
            "4",
            "--socket",
            "/s"
        ]))
    );
    assert_eq!(
        mcp_args(
            &target(AgentRole::DocReviewer, Some("spec-r1")),
            5,
            Path::new("/s")
        ),
        Some(strs(&[
            "mcp",
            "--role",
            "doc_reviewer",
            "--run",
            "r-3f9a",
            "--agent-label",
            "spec-r1",
            "--window",
            "5",
            "--socket",
            "/s"
        ]))
    );
    for (role, name) in [
        (AgentRole::Brainstormer, "brainstormer"),
        (AgentRole::DocReviewer, "doc_reviewer"),
    ] {
        let args = mcp_args(&target(role, None), 4, Path::new("/s")).expect("an mcp role");
        assert_eq!(
            args,
            strs(&[
                "mcp", "--role", name, "--run", "r-3f9a", "--window", "4", "--socket", "/s"
            ])
        );
    }
    // Only a design agent names one: the CLI refuses it for every other role.
    let orchestrator = mcp_args(
        &target(AgentRole::Orchestrator, Some("A")),
        4,
        Path::new("/s"),
    );
    assert_eq!(
        orchestrator,
        Some(strs(&[
            "mcp",
            "--role",
            "orchestrator",
            "--run",
            "r-3f9a",
            "--window",
            "4",
            "--socket",
            "/s"
        ]))
    );
}

/// Task M9.6.8 fix round 1 (m2): the read denials join the settings' own permissions,
/// never replace them.
#[test]
fn read_denials_merge_into_the_settings_permissions() {
    let mut settings = serde_json::json!({
        "permissions": {"allow": ["Read"], "deny": ["Bash(rm:*)"]},
        "hooks": {}
    });
    deny_reads(&mut settings, &[PathBuf::from("/d"), PathBuf::from("/e")]);
    assert_eq!(
        settings,
        serde_json::json!({
            "permissions": {
                "allow": ["Read"],
                "deny": ["Bash(rm:*)", "Read(//d/**)", "Read(//e/**)"]
            },
            "hooks": {}
        })
    );
    let mut bare = serde_json::json!({"hooks": {}});
    deny_reads(&mut bare, &[PathBuf::from("/d")]);
    assert_eq!(
        bare["permissions"],
        serde_json::json!({"deny": ["Read(//d/**)"]})
    );
}
