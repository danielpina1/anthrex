//! M9.8.5: `fake-agent` answers Claude's `initialize` control request and Codex's
//! `app-server` `model/list`, over real pipes, from fixtures.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(5);
const CLAUDE: &[&str] = &[
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
];
const INITIALIZE: &str = r#"{"type":"control_request","request_id":"anthrex-models-1","request":{"subtype":"initialize"}}"#;

fn spawn(
    args: &[&str],
    env: &[(&str, &std::path::Path)],
    extra: &[(&str, &str)],
) -> std::process::Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fake-agent"));
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    for (key, value) in extra {
        command.env(key, value);
    }
    command.spawn().unwrap()
}

/// Writes `lines`, closes stdin, collects every stdout line (each within the
/// deadline), asserts exit 0.
fn talk(args: &[&str], lines: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn(args, &[("FAKE_AGENT_STDIN_FILE", dir.path())], env);
    let mut stdin = child.stdin.take().unwrap();
    for line in lines {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin);
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let mut out = Vec::new();
    while let Ok(line) = rx.recv_timeout(DEADLINE) {
        out.push(line);
    }
    let status = child.wait().unwrap();
    assert!(status.success(), "{status:?} {out:?}");
    out
}

#[test]
fn claude_answers_initialize_with_the_fixture_models_and_runs_no_turn() {
    let out = talk(CLAUDE, &[INITIALIZE], &[]);
    assert_eq!(out.len(), 1, "{out:?}");
    let reply: serde_json::Value = serde_json::from_str(&out[0]).unwrap();
    assert_eq!(reply["type"], "control_response");
    assert_eq!(reply["response"]["subtype"], "success");
    assert_eq!(reply["response"]["request_id"], "anthrex-models-1");
    let models = reply["response"]["response"]["models"].as_array().unwrap();
    assert_eq!(
        models
            .iter()
            .map(|m| m["value"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["claude-haiku-4-5", "claude-sonnet-5", "claude-opus-5-5"]
    );
}

#[test]
fn codex_app_server_pages_model_list() {
    let out = talk(
        &["app-server"],
        &[
            r#"{"id":1,"method":"initialize","params":{"clientInfo":{"name":"anthrex","version":"0"}}}"#,
            r#"{"method":"initialized"}"#,
            r#"{"id":2,"method":"model/list","params":{"includeHidden":false,"cursor":null}}"#,
            r#"{"id":3,"method":"model/list","params":{"includeHidden":false,"cursor":"p2"}}"#,
        ],
        &[],
    );
    assert_eq!(out.len(), 3, "{out:?}");
    let page = |i: usize| serde_json::from_str::<serde_json::Value>(&out[i]).unwrap();
    assert_eq!(page(0)["result"]["userAgent"], "fake-agent");
    assert_eq!(page(1)["id"], 2);
    assert_eq!(page(1)["result"]["nextCursor"], "p2");
    assert_eq!(page(2)["result"]["data"][0]["id"], "gpt-6.1-sol");
    assert!(page(2)["result"]["nextCursor"].is_null());
}

#[test]
fn codex_refuses_an_unknown_method() {
    let out = talk(
        &["app-server"],
        &[r#"{"id":7,"method":"thread/start"}"#],
        &[],
    );
    let reply: serde_json::Value = serde_json::from_str(&out[0]).unwrap();
    assert_eq!(reply["id"], 7);
    assert_eq!(reply["error"]["code"], -32601);
}

#[test]
fn a_version_override_is_printed() {
    let out = Command::new(env!("CARGO_BIN_EXE_fake-agent"))
        .arg("--version")
        .env("FAKE_AGENT_VERSION", "2.1.290 (Claude Code)")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "2.1.290 (Claude Code)"
    );
    let out = Command::new(env!("CARGO_BIN_EXE_fake-agent"))
        .arg("--version")
        .env_remove("FAKE_AGENT_VERSION")
        .env_remove("FAKE_CODEX_VERSION")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "codex-cli 0.160.1"
    );
}

#[test]
fn garbage_comes_before_a_valid_reply() {
    let garbage = [("FAKE_AGENT_DISCOVERY", "garbage")];
    let out = talk(CLAUDE, &[INITIALIZE], &garbage);
    assert_eq!(out.len(), 2, "{out:?}");
    assert_eq!(out[0], "not json");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&out[1]).unwrap()["type"],
        "control_response"
    );
    let out = talk(
        &["app-server"],
        &[r#"{"id":2,"method":"model/list","params":{"cursor":null}}"#],
        &garbage,
    );
    assert_eq!(out.len(), 2, "{out:?}");
    assert_eq!(out[0], "not json");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&out[1]).unwrap()["id"],
        2
    );
}

#[test]
fn empty_answers_no_models() {
    let empty = [("FAKE_AGENT_DISCOVERY", "empty")];
    let out = talk(CLAUDE, &[INITIALIZE], &empty);
    let reply: serde_json::Value = serde_json::from_str(&out[0]).unwrap();
    assert_eq!(
        reply["response"]["response"]["models"],
        serde_json::json!([])
    );
    let out = talk(
        &["app-server"],
        &[r#"{"id":2,"method":"model/list","params":{"cursor":null}}"#],
        &empty,
    );
    let reply: serde_json::Value = serde_json::from_str(&out[0]).unwrap();
    assert_eq!(reply["result"]["data"], serde_json::json!([]));
    assert!(reply["result"]["nextCursor"].is_null());
}

#[test]
fn the_models_dir_replaces_the_fixtures() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("claude-initialize.json"),
        r#"{"commands": [], "models": [{"value": "only-one", "displayName": "Only"}]}"#,
    )
    .unwrap();
    let path = dir.path().to_str().unwrap();
    let out = talk(CLAUDE, &[INITIALIZE], &[("FAKE_AGENT_MODELS_DIR", path)]);
    let reply: serde_json::Value = serde_json::from_str(&out[0]).unwrap();
    let models = reply["response"]["response"]["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["value"], "only-one");
}

#[test]
fn hang_answers_nothing_until_killed() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn(
        CLAUDE,
        &[("FAKE_AGENT_STDIN_FILE", dir.path())],
        &[("FAKE_AGENT_DISCOVERY", "hang")],
    );
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{INITIALIZE}").unwrap();
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let _ = tx.send(line);
        }
        let _ = tx.send(Ok("EOF".into()));
    });
    drop(stdin);
    assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
    assert!(child.try_wait().unwrap().is_none(), "still alive after EOF");
    child.kill().unwrap();
    child.wait().unwrap();
}
