//! Shared by the `serve_on` tests: a stub daemon and a minimal JSON-RPC client over an
//! in-memory duplex pipe. The stub is a real `UnixListener` under `/tmp` that answers
//! as its `Script` says and records what it received. No real daemon, no real agent.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
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

pub const DEADLINE: Duration = Duration::from_secs(10);

/// What the stub daemon saw on one connection that got as far as `Hello`.
#[derive(Debug, Clone)]
pub struct Seen {
    pub client: ClientKind,
    pub proto_version: u32,
    pub call: Option<ToolCall>,
}

/// How the stub answers.
#[derive(Debug, Clone)]
pub enum Script {
    /// `Welcome`, then a `WindowsChanged` broadcast, then `ToolResult { ok, text }`.
    Reply(bool, String),
    /// `Hello` is answered with this `DaemonMsg::Error` (request `"hello"`, as the
    /// daemon's `server.rs` labels it) and the connection is closed.
    HelloError(String),
    /// `Welcome`, then these frames in order after the tool call; the connection is then
    /// held open without a `ToolResult` unless one is among them.
    Frames(Vec<DaemonMsg>),
}

pub struct StubDaemon {
    pub socket: PathBuf,
    seen: Arc<Mutex<Vec<Seen>>>,
    accepted: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

impl StubDaemon {
    /// Answers every tool call with `ToolResult { ok, text }`, after a broadcast the
    /// client must skip.
    pub fn start(reply: (bool, &str)) -> Self {
        Self::scripted(Script::Reply(reply.0, reply.1.to_string()))
    }

    pub fn scripted(script: Script) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("anthrex-mcp-stub-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("d.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicUsize::new(0));
        let (record, count) = (seen.clone(), accepted.clone());
        tokio::spawn(async move {
            let mut held = Vec::new();
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                // Every raw connection counts, whether or not it ever says `Hello`.
                count.fetch_add(1, Ordering::SeqCst);
                let (mut rd, mut wr) = stream.into_split();
                let Ok(Some(ClientMsg::Hello {
                    proto_version,
                    client,
                })) = read_frame::<_, ClientMsg>(&mut rd).await
                else {
                    continue;
                };
                if let Script::HelloError(message) = &script {
                    record.lock().unwrap().push(Seen {
                        client,
                        proto_version,
                        call: None,
                    });
                    let error = DaemonMsg::Error {
                        request: "hello".into(),
                        message: message.clone(),
                    };
                    write_frame(&mut wr, &error).await.unwrap();
                    continue;
                }
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
                if call.is_none() {
                    continue;
                }
                let frames = match &script {
                    Script::Reply(ok, text) => vec![
                        // A broadcast ahead of the reply, which the forwarder must skip.
                        DaemonMsg::WindowsChanged { windows: vec![] },
                        DaemonMsg::Run(RunReply::ToolResult {
                            ok: *ok,
                            text: text.clone(),
                        }),
                    ],
                    Script::Frames(frames) => frames.clone(),
                    Script::HelloError(_) => unreachable!(),
                };
                for frame in &frames {
                    write_frame(&mut wr, frame).await.unwrap();
                }
                // Held open, so a client waiting for more sees silence, not EOF.
                held.push((rd, wr));
            }
        });
        Self {
            socket,
            seen,
            accepted,
            _dir: dir,
        }
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// Raw `accept()`s, counted before anything is read.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

pub fn opts(role: AgentRole, socket: PathBuf) -> McpOptions {
    McpOptions {
        role,
        run_id: "add-reset-3f9a".into(),
        task_id: Some("t1".into()),
        window_id: 7,
        socket,
    }
}

/// A minimal newline-delimited JSON-RPC client over the duplex pipe.
pub struct Client {
    rd: BufReader<ReadHalf<DuplexStream>>,
    wr: WriteHalf<DuplexStream>,
    next_id: u64,
    pub server: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Client {
    pub fn start(opts: McpOptions) -> Self {
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

    pub async fn send(&mut self, msg: Value) {
        let mut line = serde_json::to_vec(&msg).unwrap();
        line.push(b'\n');
        self.wr.write_all(&line).await.unwrap();
        self.wr.flush().await.unwrap();
    }

    /// Sends a request and returns its response, bounded by `DEADLINE`.
    pub async fn request(&mut self, method: &str, params: Value) -> Value {
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

    pub async fn initialize(&mut self) -> Value {
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

    pub async fn call(&mut self, tool: &str, args: Value) -> Value {
        let reply = self
            .request("tools/call", json!({"name": tool, "arguments": args}))
            .await;
        assert!(reply.get("error").is_none(), "a JSON-RPC error: {reply}");
        reply["result"].clone()
    }

    pub fn is_error(result: &Value) -> bool {
        result["isError"] == json!(true)
    }

    pub fn text(result: &Value) -> String {
        result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }
}
