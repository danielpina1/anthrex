//! [`StubDaemon`]: a stub daemon for `anthrex mcp` on a `/tmp` socket (split out of
//! `mod.rs` to keep it under the 600-line rule, F4).

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use proto::{ClientMsg, DaemonMsg, RunReply, RunRequest, ToolCall};

use super::{Mcp, anthrex, tempdir};

/// A stub daemon on a `/tmp` socket: `Welcome` to every `Hello`, then the scripted
/// `ToolResult { ok, text }` for each `Run(Tool(..))`, recording every call. With
/// [`StubDaemon::replies`], the n-th call gets the n-th reply, and every later call the
/// last one.
pub struct StubDaemon {
    pub socket: PathBuf,
    calls: Arc<Mutex<Vec<ToolCall>>>,
    /// Milestone 9.5 decision 38: each `RunRequest::McpReady`'s `(run_id, window_id)`,
    /// answered `Done` as the daemon answers it.
    readies: Arc<Mutex<Vec<(String, u32)>>>,
    _dir: tempfile::TempDir,
}

impl StubDaemon {
    pub fn start(ok: bool, text: &str) -> Self {
        Self::replies(&[(ok, text)])
    }

    pub fn replies(replies: &[(bool, &str)]) -> Self {
        let replies: Vec<(bool, String)> =
            replies.iter().map(|(ok, t)| (*ok, t.to_string())).collect();
        assert!(!replies.is_empty(), "a stub daemon needs a reply");
        let dir = tempdir();
        let socket = dir.path().join("d.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let record = calls.clone();
        let readies = Arc::new(Mutex::new(Vec::new()));
        let ready_record = readies.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let Some(ClientMsg::Hello { .. }) = read_frame(&mut stream) else {
                    continue;
                };
                write_frame(
                    &mut stream,
                    &DaemonMsg::Welcome {
                        daemon_version: "stub".into(),
                        windows: vec![],
                    },
                );
                let request = read_frame(&mut stream);
                if let Some(ClientMsg::Run(RunRequest::McpReady { run_id, window_id })) = request {
                    ready_record.lock().unwrap().push((run_id, window_id));
                    let done = RunReply::done(proto::run_wire::request::MCP_READY, "");
                    write_frame(&mut stream, &DaemonMsg::Run(done));
                    continue;
                }
                if let Some(ClientMsg::Run(RunRequest::Tool(call))) = request {
                    let n = {
                        let mut calls = record.lock().unwrap();
                        calls.push(call);
                        calls.len()
                    };
                    let (ok, text) = replies[n.min(replies.len()) - 1].clone();
                    let reply = RunReply::tool_result(ok, text);
                    write_frame(&mut stream, &DaemonMsg::Run(reply));
                }
            }
        });
        Self {
            socket,
            calls,
            readies,
            _dir: dir,
        }
    }

    pub fn calls(&self) -> Vec<ToolCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn readies(&self) -> Vec<(String, u32)> {
        self.readies.lock().unwrap().clone()
    }

    pub fn mcp(&self, role: &'static str, task: &'static str) -> Mcp {
        Mcp {
            exe: anthrex(),
            role,
            task: Some(task),
            socket: self.socket.clone(),
        }
    }
}

fn read_frame(stream: &mut impl Read) -> Option<ClientMsg> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).ok()?;
    let mut body = vec![0u8; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut body).ok()?;
    proto::decode(&body).ok()
}

fn write_frame(stream: &mut impl Write, msg: &DaemonMsg) {
    let _ = stream.write_all(&proto::encode(msg).unwrap());
}
