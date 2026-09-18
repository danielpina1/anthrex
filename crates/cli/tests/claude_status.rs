mod support;

use std::time::Duration;

use proto::{Runtime, Status, WindowInfo, WindowSpec};
use serde_json::{Value, json};
use support::{ANTHREX, TestDaemon};

fn turn_script() -> Vec<Value> {
    vec![
        json!({"hook":"SessionStart", "payload":{}}),
        json!({"read_line":true}),
        json!({"hook":"UserPromptSubmit", "payload":{}}),
        json!({"hook":"PreToolUse", "payload":{"tool_name":"Bash"}}),
        json!({"read_line":true}),
        json!({"hook":"PostToolUse", "payload":{}}),
        json!({"hook":"Stop", "payload":{}}),
    ]
}

#[test]
fn a_turn_reports_working_the_tool_and_done() {
    let daemon = TestDaemon::start(&turn_script());
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "turn");
    client.wait_window(id, "initial session", |w| {
        w.status == Status::Idle && w.session_id.as_deref() == Some("fake-session-1")
    });
    client.input(id, b"go\r");
    client.wait_window(id, "Bash tool", |w| {
        w.status == Status::Working && w.tool.as_deref() == Some("Bash")
    });
    client.input(id, b"go\r");
    client.wait_window(id, "unviewed done", |w| {
        w.status == Status::Done && w.tool.is_none()
    });
}

#[test]
fn a_viewed_window_ends_idle() {
    let daemon = TestDaemon::start(&turn_script());
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "viewed");
    client.subscribe(id);
    client.wait_window(id, "session started", |w| {
        w.session_id.is_some() && w.status == Status::Idle
    });
    client.input(id, b"go\r");
    client.wait_window(id, "Bash tool", |w| {
        w.status == Status::Working && w.tool.as_deref() == Some("Bash")
    });
    client.input(id, b"go\r");
    client.wait_window(id, "viewed idle", |w| {
        w.status == Status::Idle && w.tool.is_none()
    });
}

#[test]
fn permission_request_waits_for_input() {
    let daemon = TestDaemon::start(&[
        json!({"hook":"SessionStart", "payload":{}}),
        json!({"hook":"PermissionRequest", "payload":{}}),
        json!({"read_line":true}),
    ]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "permission");
    client.wait_window(id, "permission attention", |w| {
        w.status == Status::Attention
    });
    client.input(id, b"y\r");
    client.wait_window(id, "permission answered", |w| w.status == Status::Working);
}

#[test]
fn idle_prompt_and_bell_after_hooks() {
    let daemon = TestDaemon::start(&[
        json!({"hook":"SessionStart", "payload":{}}),
        json!({"read_line":true}),
        json!({"hook":"Notification", "payload":{"notification_type":"idle_prompt"}}),
        json!({"bell":true}),
        json!({"print":"AFTER-BELL"}),
    ]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "bell");
    // A snapshot/output marker proves that the bell has actually reached the daemon.
    client.subscribe(id);
    client.wait_window(id, "session before bell", |w| w.session_id.is_some());
    client.input(id, b"go\r");
    client.receive(|msg| match msg {
        proto::DaemonMsg::Output { bytes, .. } => {
            String::from_utf8_lossy(bytes).contains("AFTER-BELL")
        }
        _ => false,
    });
    client.wait_window(id, "idle after bell", |w| {
        w.status == Status::Idle && w.session_id.is_some()
    });
    client.remains(id, Duration::from_millis(500), |w| w.status == Status::Idle);
}

#[test]
fn fallback_until_the_first_hook() {
    let daemon = TestDaemon::start(&[
        json!({"print":"booting"}),
        json!({"wait_ms":3500}),
        json!({"read_line":true}),
        json!({"hook":"SessionStart", "payload":{}}),
        json!({"print":"more"}),
    ]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "fallback");
    client.wait_window(id, "fallback working", |w| {
        w.status == Status::Working && w.session_id.is_none()
    });
    client.wait_window(id, "fallback quiet", |w| {
        w.status == Status::Idle && w.session_id.is_none()
    });
    client.input(id, b"go\r");
    client.wait_window(id, "hook takes over", |w| {
        w.status == Status::Idle && w.session_id.is_some()
    });
    client.remains(id, Duration::from_millis(500), |w| w.status == Status::Idle);
}

#[test]
fn launch_arguments_reach_the_agent() {
    let daemon = TestDaemon::start(&[json!({"hook":"SessionStart", "payload":{}})]);
    let mut client = daemon.client();
    let id = client.create_spec(WindowSpec {
        name: Some("argv".into()),
        runtime: Runtime::Claude,
        cwd: daemon.data_dir().into(),
        worktree_branch: None,
        model: Some("opus".into()),
        initial_prompt: Some("-x".into()),
    });
    client.wait_window(id, "agent consumed args", |w| w.session_id.is_some());
    let args: Vec<String> =
        serde_json::from_slice(&std::fs::read(daemon.data_dir().join("args.json")).unwrap())
            .unwrap();
    assert_eq!(&args[..2], ["--name", "argv"]);
    assert!(args.windows(2).any(|pair| pair == ["--model", "opus"]));
    assert_eq!(&args[args.len() - 2..], ["--", "-x"]);
    let index = args
        .iter()
        .position(|a| a == "--settings")
        .expect("settings missing");
    let settings: Value = serde_json::from_str(&args[index + 1]).unwrap();
    let command = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert_eq!(
        command,
        format!("'{ANTHREX}' hook --window {id} --source claude")
    );
}

#[test]
fn ls_json_lists_session_and_model() {
    let daemon = TestDaemon::start(&[json!({"hook":"SessionStart", "payload":{}})]);
    let mut client = daemon.client();
    let id = client.create_spec(WindowSpec {
        name: Some("json".into()),
        runtime: Runtime::Claude,
        cwd: daemon.data_dir().into(),
        worktree_branch: None,
        model: Some("opus".into()),
        initial_prompt: None,
    });
    client.wait_window(id, "session for json", |w| w.session_id.is_some());
    let output = daemon.anthrex(&["ls", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let windows: Vec<WindowInfo> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].id, id);
    assert_eq!(windows[0].session_id.as_deref(), Some("fake-session-1"));
    assert_eq!(windows[0].model.as_deref(), Some("opus"));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\n  {"),
        "expected pretty JSON"
    );
}
