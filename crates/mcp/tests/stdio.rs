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
    assert_eq!(names, ["task_done", "task_blocked", "task_note"]);
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

/// Milestone 9.5 decision 38: the orchestrator's server tells the daemon, once, that it
/// has answered `tools/list`, naming its run and window; no other role's does.
#[tokio::test]
async fn the_orchestrator_server_announces_its_tools_once() {
    let stub = StubDaemon::start((true, "unused"));
    let mut c = Client::start(opts(AgentRole::Orchestrator, stub.socket.clone()));
    c.initialize().await;
    assert_eq!(stub.accepted(), 0, "nothing before tools/list");
    let list = c.request("tools/list", json!({})).await;
    assert!(list["result"]["tools"].as_array().is_some(), "{list}");
    c.request("tools/list", json!({})).await;
    let deadline = std::time::Instant::now() + support::DEADLINE;
    while stub.seen().is_empty() {
        assert!(std::time::Instant::now() < deadline, "no McpReady");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    // A third look, after the notice: still one.
    c.request("tools/list", json!({})).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let seen = stub.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].client, ClientKind::Mcp);
    assert_eq!(seen[0].ready, Some(("add-reset-3f9a".to_string(), 7)));
    assert_eq!(seen[0].call, None);
    assert_eq!(stub.accepted(), 1);

    // A worker's server lists its tools and says nothing.
    let stub = StubDaemon::start((true, "unused"));
    let mut w = Client::start(opts(AgentRole::Worker, stub.socket.clone()));
    w.initialize().await;
    w.request("tools/list", json!({})).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(stub.accepted(), 0, "{:?}", stub.seen());
}

/// A notice that cannot reach the daemon is dropped: the server goes on serving.
#[tokio::test]
async fn a_lost_ready_notice_changes_nothing() {
    let dir = tempfile::Builder::new()
        .prefix("anthrex-mcp-ready-")
        .tempdir_in("/tmp")
        .unwrap();
    let mut c = Client::start(opts(AgentRole::Orchestrator, dir.path().join("none.sock")));
    c.initialize().await;
    let list = c.request("tools/list", json!({})).await;
    assert!(list["result"]["tools"].as_array().is_some(), "{list}");
    let list = c.request("tools/list", json!({})).await;
    assert!(list["result"]["tools"].as_array().is_some(), "{list}");
    assert!(!c.server.is_finished());
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
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
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
        3,
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

/// M8b decision 15: a scout's call names its scout, and a repository-level scout's has
/// no run.
#[tokio::test]
async fn a_scout_call_carries_its_scout_id() {
    let stub = StubDaemon::start((true, "Report recorded. You are done; end your turn now."));
    let mut options = opts(AgentRole::Scout, stub.socket.clone());
    options.run_id = String::new();
    options.task_id = None;
    options.scout_id = Some("onboarding-1".into());
    let mut c = Client::start(options);
    c.initialize().await;
    let list = c.request("tools/list", json!({})).await;
    assert_eq!(list["result"]["tools"][0]["name"], "submit_scout_report");
    let args = json!({"summary": "s", "files": []});
    let result = c.call("submit_scout_report", args.clone()).await;
    assert!(!Client::is_error(&result), "{result}");
    assert_eq!(
        stub.seen()[0].call,
        Some(ToolCall {
            run_id: String::new(),
            task_id: None,
            role: AgentRole::Scout,
            window_id: 7,
            tool: "submit_scout_report".into(),
            args,
            scout_id: Some("onboarding-1".into()),
            epic: None,
            chain: None,
            lane: None,
        })
    );
}

/// Milestone 9 decision 15: `anthrex mcp` refuses a tool outside its role's list before
/// it reaches the daemon. The socket does not exist, so a call that got through would
/// say it cannot reach the daemon instead.
#[tokio::test]
async fn tool_outside_the_role_is_refused_without_a_daemon() {
    let dir = tempfile::Builder::new()
        .prefix("anthrex-mcp-none-")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = dir.path().join("nobody.sock");
    let cases: [(AgentRole, &str, &str); 9] = [
        (AgentRole::Orchestrator, "task_done", "orchestrator"),
        (AgentRole::Orchestrator, "submit_epic", "orchestrator"),
        (AgentRole::Orchestrator, "task_note", "orchestrator"),
        (AgentRole::Planner, "edit_plan", "planner"),
        (AgentRole::Planner, "run_status", "planner"),
        (AgentRole::Planner, "spawn_scout", "planner"),
        (AgentRole::Worker, "run_status", "worker"),
        (AgentRole::Reviewer, "task_note", "reviewer"),
        (AgentRole::Decider, "get_context", "decider"),
    ];
    for (role, tool, name) in cases {
        let mut options = opts(role, socket.clone());
        options.epic = (role == AgentRole::Planner).then(|| "mail".to_string());
        let mut c = Client::start(options);
        c.initialize().await;
        let result = c.call(tool, json!({})).await;
        assert!(Client::is_error(&result), "{result}");
        assert_eq!(
            Client::text(&result),
            format!("tool {tool} is not available to the {name} role")
        );
    }
    // A tool inside the role's list does go to the socket.
    for (role, tool) in [
        (AgentRole::Orchestrator, "run_status"),
        (AgentRole::Planner, "get_context"),
        (AgentRole::Worker, "task_note"),
    ] {
        let mut c = Client::start(opts(role, socket.clone()));
        c.initialize().await;
        let result = c.call(tool, json!({})).await;
        assert!(
            Client::text(&result).starts_with("cannot reach the anthrex daemon at "),
            "{role:?} {tool}: {result}"
        );
    }
}

/// Milestone 9.3 (KG §3.4): a chained orchestrator's call names its chain.
#[tokio::test]
async fn an_orchestrator_call_carries_its_chain() {
    let stub = StubDaemon::start((true, "{}"));
    let mut options = opts(AgentRole::Orchestrator, stub.socket.clone());
    options.task_id = None;
    options.chain = Some("o-3f9a".into());
    let mut c = Client::start(options);
    c.initialize().await;
    let result = c.call("run_status", json!({})).await;
    assert!(!Client::is_error(&result), "{result}");
    let call = stub.seen()[0].call.clone().expect("the call");
    assert_eq!(call.role, AgentRole::Orchestrator);
    assert_eq!(call.chain.as_deref(), Some("o-3f9a"));
    assert_eq!(call.tool, "run_status");
}

/// Milestone 9.5: a racer's call names its lane, through `ToolCall.lane`.
#[tokio::test]
async fn a_racer_call_carries_its_lane() {
    let stub = StubDaemon::start((true, "accepted"));
    let mut options = opts(AgentRole::Racer, stub.socket.clone());
    options.lane = Some(proto::RaceLane::B);
    let mut c = Client::start(options);
    c.initialize().await;
    let result = c.call("task_done", json!({"summary": "done"})).await;
    assert!(!Client::is_error(&result), "{result}");
    let call = stub.seen()[0].call.clone().expect("the call");
    assert_eq!(call.role, AgentRole::Racer);
    assert_eq!(call.lane, Some(proto::RaceLane::B));
    assert_eq!(call.task_id.as_deref(), Some("t1"));
    assert_eq!(call.tool, "task_done");

    // Any other role's call carries none.
    let stub = StubDaemon::start((true, "accepted"));
    let mut c = Client::start(opts(AgentRole::TestWriter, stub.socket.clone()));
    c.initialize().await;
    c.call("task_done", json!({"summary": "red"})).await;
    let call = stub.seen()[0].call.clone().expect("the call");
    assert_eq!((call.role, call.lane), (AgentRole::TestWriter, None));
}

/// Milestone 9 decision 15: a sub-planner's call names its epic.
#[tokio::test]
async fn a_planner_call_carries_its_epic() {
    let stub = StubDaemon::start((true, "{}"));
    let mut options = opts(AgentRole::Planner, stub.socket.clone());
    options.task_id = None;
    options.epic = Some("mail".into());
    let mut c = Client::start(options);
    c.initialize().await;
    let result = c.call("get_context", json!({})).await;
    assert!(!Client::is_error(&result), "{result}");
    let call = stub.seen()[0].call.clone().expect("the call");
    assert_eq!(call.role, AgentRole::Planner);
    assert_eq!(call.epic.as_deref(), Some("mail"));
    assert_eq!(call.tool, "get_context");
}

/// Claude Code 2.1.291 speaks MCP `2026-07-28`: it opens with `server/discover`, with no
/// `initialize`, and drops every tool from a `tools/list` result that lacks the cache
/// hints (`ttlMs`, `cacheScope`). rmcp 3.4.0 left them out, so a scout never saw
/// `submit_scout_report` and every detection failed with "the scout ended two turns
/// without a report". The `_meta` below is the one that Claude Code sent.
#[tokio::test]
async fn a_2026_07_28_peer_gets_cache_hints_on_tools_list() {
    let stub = StubDaemon::start((true, "unused"));
    let mut options = opts(AgentRole::Scout, stub.socket.clone());
    options.run_id = String::new();
    options.task_id = None;
    options.scout_id = Some("onboarding-1".into());
    let mut c = Client::start(options);
    let meta = json!({"_meta": {
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "claude-code", "version": "2.1.291"},
        "io.modelcontextprotocol/clientCapabilities": {},
    }});
    let discover = c.request("server/discover", meta.clone()).await;
    assert!(
        discover["result"]["supportedVersions"]
            .as_array()
            .is_some_and(|v| v.contains(&json!("2026-07-28"))),
        "{discover}"
    );
    let list = c.request("tools/list", meta).await;
    assert_eq!(
        list["result"]["tools"][0]["name"], "submit_scout_report",
        "{list}"
    );
    assert!(list["result"]["ttlMs"].is_u64(), "{list}");
    assert!(list["result"]["cacheScope"].is_string(), "{list}");
    assert_eq!(stub.accepted(), 0, "listing never touches the daemon");
}
