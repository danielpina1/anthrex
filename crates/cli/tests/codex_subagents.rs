mod support;

use proto::{Runtime, SubagentInfo, SubagentState, WindowInfo};
use serde_json::json;
use support::TestDaemon;

#[test]
fn codex_sub_agents_pair_and_never_touch_the_session_id() {
    let daemon = TestDaemon::start(&[
        json!({"hook":"SessionStart", "payload":{"session_id":"root"}}),
        json!({"hook":"PreToolUse", "payload":{
            "session_id":"root",
            "tool_name":"collaborationspawn_agent",
            "tool_input":{
                "task_name":"list_filenames",
                "fork_turns":"none",
                "model":"gpt-6-astra",
                "message":"opaque runtime value"
            }
        }}),
        json!({"hook":"SubagentStart", "payload":{
            "session_id":"c1",
            "agent_id":"c1",
            "agent_type":"default"
        }}),
        json!({"hook":"PreToolUse", "payload":{
            "session_id":"c1",
            "agent_id":"c1",
            "agent_type":"default",
            "tool_name":"Bash"
        }}),
        json!({"read_line":true}),
        json!({"hook":"SubagentStop", "payload":{
            "session_id":"c1",
            "agent_id":"c1",
            "agent_type":"default"
        }}),
    ]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Codex, "subagent");
    let running = client.wait_window(id, "running Codex sub-agent", |window| {
        window.session_id.as_deref() == Some("root")
            && window.subagents.len() == 1
            && window.subagents[0].state == SubagentState::Running
            && window.subagents[0].tool.as_deref() == Some("Bash")
    });
    assert_eq!(
        running.subagents[0],
        SubagentInfo {
            id: "c1".into(),
            parent_id: None,
            kind: "default".into(),
            label: Some("list_filenames".into()),
            model: Some("gpt-6-astra".into()),
            state: SubagentState::Running,
            tool: Some("Bash".into()),
            started_secs: 0,
            ended_secs: None,
            needs_permission: false,
        }
    );

    client.input(id, b"go\r");
    let done = client.wait_window(id, "finished Codex sub-agent", |window| {
        window.session_id.as_deref() == Some("root")
            && window.subagents.len() == 1
            && window.subagents[0].state == SubagentState::Done
            && window.subagents[0].tool.is_none()
    });
    assert_eq!(done.session_id.as_deref(), Some("root"));
    assert_eq!(done.subagents[0].label.as_deref(), Some("list_filenames"));
    assert_eq!(done.subagents[0].model.as_deref(), Some("gpt-6-astra"));

    let output = daemon.anthrex(&["ls", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let windows: Vec<WindowInfo> = serde_json::from_slice(&output.stdout).unwrap();
    let listed = windows.iter().find(|window| window.id == id).unwrap();
    assert_eq!(listed.session_id.as_deref(), Some("root"));
    assert_eq!(listed.subagents, done.subagents);
}
