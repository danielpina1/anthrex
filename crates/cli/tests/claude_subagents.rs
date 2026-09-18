mod support;

use proto::{Runtime, Status, SubagentInfo, SubagentState};
use serde_json::json;
use support::TestDaemon;

#[test]
fn the_agent_tool_names_its_sub_agent() {
    let daemon = TestDaemon::start(&[
        json!({"hook":"PreToolUse", "payload":{"tool_name":"Agent", "tool_input":{"subagent_type":"Explore", "name":"scout", "model":"haiku", "prompt":"look"}}}),
        json!({"hook":"SubagentStart", "payload":{"agent_id":"a1", "agent_type":"Explore"}}),
        json!({"hook":"PreToolUse", "payload":{"agent_id":"a1", "tool_name":"Grep"}}),
        json!({"read_line":true}),
        json!({"hook":"PostToolUse", "payload":{"agent_id":"a1"}}),
        json!({"hook":"SubagentStop", "payload":{"agent_id":"a1", "agent_type":"Explore"}}),
    ]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "subagent");
    let running = client.wait_window(id, "running named sub-agent", |window| {
        window.subagents.len() == 1
            && window.subagents[0].state == SubagentState::Running
            && window.subagents[0].tool.as_deref() == Some("Grep")
    });
    assert_eq!(
        running.subagents[0],
        SubagentInfo {
            id: "a1".into(),
            parent_id: None,
            kind: "Explore".into(),
            label: Some("scout".into()),
            model: Some("haiku".into()),
            state: SubagentState::Running,
            tool: Some("Grep".into()),
            started_secs: 0,
            ended_secs: None,
            needs_permission: false,
        }
    );

    client.input(id, b"go\r");
    client.wait_window(id, "finished sub-agent", |window| {
        window.subagents.len() == 1
            && window.subagents[0].state == SubagentState::Done
            && window.subagents[0].tool.is_none()
    });
}

#[test]
fn nested_sub_agents_record_their_parent() {
    let daemon = TestDaemon::start(&[
        json!({"hook":"SubagentStart", "payload":{"agent_id":"a1", "agent_type":"Explore"}}),
        json!({"hook":"PreToolUse", "payload":{"agent_id":"a1", "tool_name":"Agent", "tool_input":{"subagent_type":"Plan", "name":"planner", "model":"sonnet", "prompt":"plan"}}}),
        json!({"hook":"SubagentStart", "payload":{"agent_id":"a2", "agent_type":"Plan"}}),
        json!({"read_line":true}),
    ]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "nested");
    let window = client.wait_window(id, "nested sub-agent", |window| {
        window.subagents.iter().any(|agent| agent.id == "a2")
    });
    let nested = window
        .subagents
        .iter()
        .find(|agent| agent.id == "a2")
        .unwrap();
    assert_eq!(nested.parent_id.as_deref(), Some("a1"));
    assert_eq!(nested.label.as_deref(), Some("planner"));
}

#[test]
fn running_sub_agents_fail_when_the_agent_exits() {
    let daemon = TestDaemon::start(&[
        json!({"hook":"SubagentStart", "payload":{"agent_id":"a1", "agent_type":"Explore"}}),
        json!({"exit":1}),
    ]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "exit");
    client.wait_window(id, "failed sub-agent after exit", |window| {
        window.status == Status::Exited
            && window.subagents.len() == 1
            && window.subagents[0].state == SubagentState::Failed
    });
}
