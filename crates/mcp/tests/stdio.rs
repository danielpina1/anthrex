//! `serve_on` over an in-memory duplex pipe, against the stub daemon in `support`.

mod support;

use proto::{AgentRole, ClientKind, ToolCall};
use serde_json::json;
use support::{Client, StubDaemon, opts};

#[tokio::test]
async fn initialize_then_list_tools() {
    let stub = StubDaemon::start((true, "unused"));
    let mut c = Client::start(opts(AgentRole::Worker, stub.socket.clone()));
    let init = c.initialize().await;
    assert_eq!(init["result"]["serverInfo"]["name"], "anthrex", "{init}");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18", "{init}");
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );

    let list = c.request("tools/list", json!({})).await;
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("{list}"))
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["task_done", "task_blocked"]);
    assert_eq!(
        list["result"]["tools"][0]["inputSchema"]["additionalProperties"],
        json!(false)
    );

    let mut r = Client::start(opts(AgentRole::Reviewer, stub.socket.clone()));
    r.initialize().await;
    let list = r.request("tools/list", json!({})).await;
    assert_eq!(
        list["result"]["tools"][0]["name"], "submit_review",
        "{list}"
    );
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 1);

    assert_eq!(stub.accepted(), 0, "listing never touches the daemon");
}

#[tokio::test]
async fn tool_call_is_forwarded_with_role_run_task_and_window() {
    let stub = StubDaemon::start((true, "Task t1 recorded as done."));
    let mut c = Client::start(opts(AgentRole::Worker, stub.socket.clone()));
    c.initialize().await;
    let args =
        json!({"summary": "added the token model", "test": "token::expires", "red": "1a2b3c4"});
    let result = c.call("task_done", args.clone()).await;
    assert!(!Client::is_error(&result), "{result}");
    assert_eq!(Client::text(&result), "Task t1 recorded as done.");

    let seen = stub.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].client, ClientKind::Mcp);
    assert_eq!(seen[0].proto_version, proto::PROTO_VERSION);
    assert_eq!(
        seen[0].call,
        Some(ToolCall {
            run_id: "add-reset-3f9a".into(),
            task_id: Some("t1".into()),
            role: AgentRole::Worker,
            window_id: 7,
            tool: "task_done".into(),
            args,
        })
    );

    // A second call opens a fresh connection (the refreshed M8 brief's decision 41).
    let result = c
        .call(
            "task_blocked",
            json!({"kind": "question", "reason": "why?"}),
        )
        .await;
    assert!(!Client::is_error(&result), "{result}");
    assert_eq!(stub.seen().len(), 2);
    assert_eq!(stub.seen()[1].call.as_ref().unwrap().tool, "task_blocked");
}

#[tokio::test]
async fn daemon_error_becomes_is_error() {
    let stub = StubDaemon::start((
        false,
        "run add-reset-3f9a is paused; the user must resume it",
    ));
    let mut c = Client::start(opts(AgentRole::Reviewer, stub.socket.clone()));
    c.initialize().await;
    let result = c
        .call(
            "submit_review",
            json!({"verdict": "approve", "summary": "fine", "findings": []}),
        )
        .await;
    assert!(Client::is_error(&result), "{result}");
    assert_eq!(
        Client::text(&result),
        "run add-reset-3f9a is paused; the user must resume it"
    );
}

#[tokio::test]
async fn daemon_down_is_a_tool_error_not_a_crash() {
    let dir = tempfile::Builder::new()
        .prefix("anthrex-mcp-down-")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = dir.path().join("nobody.sock");
    let mut c = Client::start(opts(AgentRole::Worker, socket.clone()));
    c.initialize().await;
    let result = c.call("task_done", json!({"summary": "done"})).await;
    assert!(Client::is_error(&result), "{result}");
    let prefix = format!("cannot reach the anthrex daemon at {}: ", socket.display());
    assert!(
        Client::text(&result).starts_with(&prefix),
        "{:?} does not start with {prefix:?}",
        Client::text(&result)
    );

    // Still serving.
    let list = c.request("tools/list", json!({})).await;
    assert_eq!(
        list["result"]["tools"].as_array().unwrap().len(),
        2,
        "{list}"
    );
    assert!(!c.server.is_finished());
}

#[tokio::test]
async fn a_tool_outside_the_role_never_reaches_the_daemon() {
    let stub = StubDaemon::start((true, "should not be seen"));
    let mut worker = Client::start(opts(AgentRole::Worker, stub.socket.clone()));
    worker.initialize().await;
    let result = worker
        .call(
            "submit_review",
            json!({"verdict": "approve", "summary": "x", "findings": []}),
        )
        .await;
    assert!(Client::is_error(&result), "{result}");
    assert_eq!(
        Client::text(&result),
        "tool submit_review is not available to the worker role"
    );

    let mut reviewer = Client::start(opts(AgentRole::Reviewer, stub.socket.clone()));
    reviewer.initialize().await;
    let result = reviewer.call("task_done", json!({"summary": "x"})).await;
    assert_eq!(
        Client::text(&result),
        "tool task_done is not available to the reviewer role"
    );

    let mut orchestrator = Client::start(opts(AgentRole::Orchestrator, stub.socket.clone()));
    orchestrator.initialize().await;
    let result = orchestrator
        .call("task_done", json!({"summary": "x"}))
        .await;
    assert_eq!(
        Client::text(&result),
        "tool task_done is not available to the orchestrator role"
    );

    // Raw accepts, not Hellos: even a connection that closes before saying anything
    // would count.
    assert_eq!(stub.accepted(), 0, "{:?}", stub.seen());
}
