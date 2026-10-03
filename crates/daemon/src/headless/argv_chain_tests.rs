//! Milestone 9.3 (KG §3.4): a chained orchestrator's `anthrex mcp` argv. In its own
//! file because `argv_tests.rs` is at its size budget.

use super::*;
use std::path::Path;

/// Milestone 9.3 (KG §3.4): a chained orchestrator's `anthrex mcp` names its chain,
/// after `--run` and before `--window`.
#[test]
fn mcp_args_carry_the_chain() {
    let target = McpTarget {
        role: AgentRole::Orchestrator,
        run_id: "r-3f9a".into(),
        task_id: None,
        scout_id: None,
        epic: None,
        chain: Some("o-3f9a".into()),
    };
    assert_eq!(
        mcp_args(&target, 4, Path::new("/s")),
        Some(strs(&[
            "mcp",
            "--role",
            "orchestrator",
            "--run",
            "r-3f9a",
            "--chain",
            "o-3f9a",
            "--window",
            "4",
            "--socket",
            "/s"
        ]))
    );
    let unchained = McpTarget {
        chain: None,
        ..target
    };
    assert_eq!(
        mcp_args(&unchained, 4, Path::new("/s")),
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
