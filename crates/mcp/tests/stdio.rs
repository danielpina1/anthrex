//! `serve_on` over an in-memory duplex pipe, against a stub daemon: a real
//! `UnixListener` under `/tmp` that answers `Hello` with `Welcome` and each
//! `Run(Tool)` with a scripted `ToolResult`, recording what it received. No real
//! daemon, no real agent.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mcp::McpOptions;
use proto::{
    AgentRole, ClientKind, ClientMsg, DaemonMsg, RunReply, RunRequest, ToolCall, read_frame,
    write_frame,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::net::UnixListener;

const DEADLINE: Duration = Duration::from_secs(10);

/// What the stub daemon saw on one connection.
#[derive(Debug, Clone)]
struct Seen {
    client: ClientKind,
    proto_version: u32,
    call: Option<ToolCall>,
}

struct StubDaemon {
    socket: PathBuf,
    seen: Arc<Mutex<Vec<Seen>>>,
    _dir: tempfile::TempDir,
}

impl StubDaemon {
    /// Answers every tool call with `reply`, after a broadcast the client must skip.
    fn start(reply: (bool, &str)) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("anthrex-mcp-stub-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("d.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        let (ok, text) = (reply.0, reply.1.to_string());
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (mut rd, mut wr) = stream.into_split();
                let Ok(Some(ClientMsg::Hello {
                    proto_version,
                    client,
                })) = read_frame::<_, ClientMsg>(&mut rd).await
                else {
                    continue;
                };
                write_frame(
                    &mut wr,
                    &DaemonMsg::Welcome {
                        daemon_version: "stub".into(),
                        windows: vec![],
                    },
                )
                .await
                .unwrap();
                let call = match read_frame::<_, ClientMsg>(&mut rd).await {
                    Ok(Some(ClientMsg::Run(RunRequest::Tool(call)))) => Some(call),
                    _ => None,
                };
                record.lock().unwrap().push(Seen {
                    client,
                    proto_version,
                    call: call.clone(),
                });
                if call.is_some() {
                    // A broadcast ahead of the reply, which the forwarder must skip.
                    write_frame(&mut wr, &DaemonMsg::WindowsChanged { windows: vec![] })
                        .await
                        .unwrap();
                    write_frame(
                        &mut wr,
                        &DaemonMsg::Run(RunReply::ToolResult {
                            ok,
                            text: text.clone(),
                        }),
                    )
                    .await
                    .unwrap();
                }
            }
        });
        Self {
            socket,
            seen,
            _dir: dir,
        }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn opts(role: AgentRole, socket: PathBuf) -> McpOptions {
    McpOptions {
        role,
        run_id: "add-reset-3f9a".into(),
        task_id: Some("t1".into()),
        window_id: 7,
        socket,
    }
}

/// A minimal newline-delimited JSON-RPC client over the duplex pipe.
struct Client {
    rd: BufReader<ReadHalf<DuplexStream>>,
    wr: WriteHalf<DuplexStream>,
    next_id: u64,
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Client {
    fn start(opts: McpOptions) -> Self {
        let (ours, theirs) = tokio::io::duplex(1 << 16);
        let (sr, sw) = tokio::io::split(theirs);
        let server = tokio::spawn(mcp::serve_on(opts, sr, sw));
        let (rd, wr) = tokio::io::split(ours);
        Self {
            rd: BufReader::new(rd),
            wr,
            next_id: 1,
            server,
        }
    }

    async fn send(&mut self, msg: Value) {
        let mut line = serde_json::to_vec(&msg).unwrap();
        line.push(b'\n');
        self.wr.write_all(&line).await.unwrap();
        self.wr.flush().await.unwrap();
    }

    /// Sends a request and returns its response, bounded by `DEADLINE`.
    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        tokio::time::timeout(DEADLINE, async {
            loop {
                let mut line = String::new();
                let n = self.rd.read_line(&mut line).await.unwrap();
                assert!(
                    n > 0,
                    "the server closed its output before answering {method}"
                );
                let v: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("not JSON-RPC: {line:?}: {e}"));
                if v["id"] == json!(id) {
                    return v;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no response to {method} within {DEADLINE:?}"))
    }

    async fn initialize(&mut self) -> Value {
        let reply = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "0"},
                }),
            )
            .await;
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        reply
    }

    async fn call(&mut self, tool: &str, args: Value) -> Value {
        let reply = self
            .request("tools/call", json!({"name": tool, "arguments": args}))
            .await;
        assert!(reply.get("error").is_none(), "a JSON-RPC error: {reply}");
        reply["result"].clone()
    }

    fn is_error(result: &Value) -> bool {
        result["isError"] == json!(true)
    }

    fn text(result: &Value) -> String {
        result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }
}

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

    assert!(stub.seen().is_empty(), "listing never touches the daemon");
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

    assert!(stub.seen().is_empty(), "{:?}", stub.seen());
}
