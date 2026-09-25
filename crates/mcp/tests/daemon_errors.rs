//! How `forward` treats a `DaemonMsg::Error`, against the stub daemon in `support`: a
//! handshake refusal becomes a fixed text that names no command (review I1); an Error
//! labelled for the tool call ends the wait; any other Error is skipped (review M1).

mod support;

use proto::{AgentRole, DaemonMsg, RunReply};
use serde_json::json;
use support::{Client, Script, StubDaemon, opts};

/// The daemon's real mismatch reply (`crates/daemon/src/server.rs`), which is written for
/// a human at a terminal.
const MISMATCH: &str =
    "protocol version mismatch (client 8, daemon 7); run `anthrex daemon stop` and try again";

#[tokio::test]
async fn a_version_mismatch_is_a_fixed_error_that_names_no_command() {
    let stub = StubDaemon::scripted(Script::HelloError(MISMATCH.into()));
    let mut c = Client::start(opts(AgentRole::Worker, stub.socket.clone()));
    c.initialize().await;
    let result = c.call("task_done", json!({"summary": "done"})).await;
    assert!(Client::is_error(&result), "{result}");
    let text = Client::text(&result);
    assert_eq!(
        text,
        "the anthrex daemon speaks a different protocol version; the user must restart it"
    );
    assert!(!text.contains("anthrex daemon stop"), "{text:?}");
    assert!(!text.contains('`'), "{text:?}");
    assert_eq!(stub.seen().len(), 1, "the Hello did reach the stub");
}

#[tokio::test]
async fn an_error_answering_the_tool_call_ends_the_wait() {
    // No `ToolResult` follows: without the branch the call would sit out the 100 s
    // `TOOL_REPLY_TIMEOUT`, far past the client's own 10 s deadline.
    let stub = StubDaemon::scripted(Script::Frames(vec![
        DaemonMsg::WindowsChanged { windows: vec![] },
        DaemonMsg::Error {
            request: proto::run_wire::request::TOOL.into(),
            message: "the run engine is not running".into(),
        },
    ]));
    let mut c = Client::start(opts(AgentRole::Worker, stub.socket.clone()));
    c.initialize().await;
    let result = c.call("task_done", json!({"summary": "done"})).await;
    assert!(Client::is_error(&result), "{result}");
    assert_eq!(Client::text(&result), "the run engine is not running");
}

#[tokio::test]
async fn an_error_for_another_request_is_skipped() {
    let stub = StubDaemon::scripted(Script::Frames(vec![
        DaemonMsg::Error {
            request: "kill".into(),
            message: "no window 9".into(),
        },
        DaemonMsg::Run(RunReply::ToolResult {
            ok: true,
            text: "Task t1 recorded as done.".into(),
        }),
    ]));
    let mut c = Client::start(opts(AgentRole::Worker, stub.socket.clone()));
    c.initialize().await;
    let result = c.call("task_done", json!({"summary": "done"})).await;
    assert!(!Client::is_error(&result), "{result}");
    assert_eq!(Client::text(&result), "Task t1 recorded as done.");
}
