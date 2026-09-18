mod support;

use proto::{Runtime, Status, WindowSpec};
use serde_json::json;
use support::{ANTHREX, TestDaemon};

#[test]
fn titles_and_notify_drive_a_codex_window() {
    let daemon = TestDaemon::start(&[
        json!({"title":"Working"}),
        json!({"read_line":true}),
        json!({"title":"Ready"}),
        json!({"read_line":true}),
        json!({"notify":{"thread-id":"t1"}}),
    ]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Codex, "titles");
    client.wait_window(id, "working title", |w| w.status == Status::Working);
    client.input(id, b"go\r");
    client.wait_window(id, "ready title", |w| w.status == Status::Done);
    client.input(id, b"go\r");
    client.wait_window(id, "root notify", |w| {
        w.status == Status::Done && w.session_id.as_deref() == Some("t1")
    });
}

#[test]
fn waiting_title_and_bell_mean_attention() {
    let daemon = TestDaemon::start(&[
        json!({"title":"Waiting"}),
        json!({"read_line":true}),
        json!({"title":"Working"}),
        json!({"read_line":true}),
        json!({"bell":true}),
    ]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Codex, "attention");
    client.wait_window(id, "waiting title", |w| w.status == Status::Attention);
    client.input(id, b"go\r");
    client.wait_window(id, "working", |w| w.status == Status::Working);
    client.input(id, b"go\r");
    client.wait_window(id, "bell", |w| w.status == Status::Attention);
}

#[test]
fn lifecycle_hooks_drive_a_codex_window() {
    let daemon = TestDaemon::start(&[
        json!({"hook":"SessionStart", "payload":{"session_id":"root"}}),
        json!({"read_line":true}),
        json!({"hook":"UserPromptSubmit", "payload":{}}),
        json!({"hook":"PreToolUse", "payload":{"tool_name":"shell"}}),
        json!({"read_line":true}),
        json!({"hook":"SessionStart", "payload":{"session_id":"child"}}),
        json!({"notify":{"thread-id":"child"}}),
        json!({"hook":"PreToolUse", "payload":{"tool_name":"child-marker"}}),
        json!({"read_line":true}),
        json!({"hook":"Stop", "payload":{}}),
    ]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Codex, "hooks");
    client.wait_window(id, "root session", |w| {
        w.status == Status::Idle && w.session_id.as_deref() == Some("root")
    });
    client.input(id, b"go\r");
    client.wait_window(id, "shell tool", |w| {
        w.status == Status::Working && w.tool.as_deref() == Some("shell")
    });
    client.input(id, b"go\r");
    client.wait_window(id, "child notify ignored", |w| {
        w.status == Status::Working
            && w.tool.as_deref() == Some("child-marker")
            && w.session_id.as_deref() == Some("root")
    });
    client.input(id, b"go\r");
    client.wait_window(id, "root turn done", |w| {
        w.status == Status::Done && w.tool.is_none() && w.session_id.as_deref() == Some("root")
    });
}

#[test]
fn launch_arguments_reach_the_codex_agent() {
    let daemon = TestDaemon::start(&[json!({"hook":"SessionStart", "payload":{}})]);
    let mut client = daemon.client();
    let id = client.create_spec(WindowSpec {
        name: Some("argv".into()),
        runtime: Runtime::Codex,
        cwd: daemon.data_dir().into(),
        worktree_branch: None,
        model: Some("gpt-5-codex".into()),
        initial_prompt: Some("-x".into()),
    });
    client.wait_window(id, "args consumed", |w| w.session_id.is_some());
    let args: Vec<String> =
        serde_json::from_slice(&std::fs::read(daemon.data_dir().join("args.json")).unwrap())
            .unwrap();
    assert_eq!(&args[..2], ["-C", daemon.data_dir().to_str().unwrap()]);
    let notify: Vec<String> =
        serde_json::from_str(args[3].strip_prefix("notify=").unwrap()).unwrap();
    assert_eq!(
        notify,
        [
            ANTHREX,
            "hook",
            "--window",
            &id.to_string(),
            "--source",
            "codex-notify"
        ]
    );
    assert_eq!(&args[args.len() - 4..], ["-m", "gpt-5-codex", "--", "-x"]);
}
