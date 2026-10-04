//! Milestone 9.5 task 2: a racer's and a test writer's `anthrex mcp` argv. In its own
//! file, registered from `argv.rs`, because `argv_tests.rs` is at its size budget.

use super::*;
use std::path::Path;

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// A racer's `anthrex mcp` names its role and its lane, after `--task` and before
/// `--window`; a test writer's names its role spelt as `--role` takes it, and no lane.
#[test]
fn mcp_args_carry_the_lane() {
    let racer = McpTarget {
        role: AgentRole::Racer,
        run_id: "r-3f9a".into(),
        task_id: Some("t1".into()),
        scout_id: None,
        epic: None,
        chain: None,
        lane: Some(proto::RaceLane::A),
    };
    assert_eq!(
        mcp_args(&racer, 4, Path::new("/s")),
        Some(strs(&[
            "mcp", "--role", "racer", "--run", "r-3f9a", "--task", "t1", "--lane", "a", "--window",
            "4", "--socket", "/s"
        ]))
    );
    let lane_b = McpTarget {
        lane: Some(proto::RaceLane::B),
        ..racer.clone()
    };
    let args = mcp_args(&lane_b, 4, Path::new("/s")).unwrap();
    assert_eq!(args[7..9], strs(&["--lane", "b"]));

    let writer = McpTarget {
        role: AgentRole::TestWriter,
        lane: None,
        ..racer
    };
    assert_eq!(
        mcp_args(&writer, 5, Path::new("/s")),
        Some(strs(&[
            "mcp",
            "--role",
            "test_writer",
            "--run",
            "r-3f9a",
            "--task",
            "t1",
            "--window",
            "5",
            "--socket",
            "/s"
        ]))
    );
}

/// Task M9.5.18 (task 2 review m2): only a racer's `anthrex mcp` names a lane, since
/// the CLI refuses `--lane` for any other role. A lane left on another role's target is
/// not written.
#[test]
fn only_a_racer_names_its_lane() {
    for role in [
        AgentRole::Worker,
        AgentRole::TestWriter,
        AgentRole::Reviewer,
        AgentRole::Orchestrator,
    ] {
        let target = McpTarget {
            role,
            run_id: "r-3f9a".into(),
            task_id: Some("t1".into()),
            scout_id: None,
            epic: None,
            chain: None,
            lane: Some(proto::RaceLane::A),
        };
        let args = mcp_args(&target, 4, Path::new("/s")).unwrap();
        assert!(!args.iter().any(|a| a == "--lane"), "{role:?}: {args:?}");
    }
}
