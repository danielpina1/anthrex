//! The hidden `anthrex mcp` subcommand, run as the built binary: stdout carries
//! JSON-RPC and nothing else (pitfall 17). No daemon is started or touched: the socket
//! is a path under `/tmp` that nothing listens on, so a tool call exercises the
//! daemon-down path.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const DEADLINE: Duration = Duration::from_secs(20);

#[test]
fn mcp_subcommand_speaks_json_rpc_on_stdout_only() {
    let dir = tempfile::Builder::new()
        .prefix("anthrex-mcp-cli-")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = dir.path().join("nobody.sock");

    let mut child = Command::new(env!("CARGO_BIN_EXE_anthrex"))
        .args(["mcp", "--role", "worker", "--run", "r1", "--task", "t1"])
        .args(["--window", "3", "--socket"])
        .arg(&socket)
        // Belt and braces: even a stray default-path lookup lands under /tmp.
        .env("ANTHREX_SOCKET", dir.path().join("env.sock"))
        .env("ANTHREX_DATA_DIR", dir.path().join("data"))
        .env("RUST_LOG", "trace")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Every stdout line, as it arrives.
    let (tx, rx) = mpsc::channel::<String>();
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut raw = Vec::new();
        let mut rd = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            match rd.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    raw.extend_from_slice(&line);
                    let _ = tx.send(String::from_utf8_lossy(&line).into_owned());
                }
            }
        }
        raw
    });

    let mut stdin = child.stdin.take().unwrap();
    let mut send = |v: Value| {
        let mut line = serde_json::to_vec(&v).unwrap();
        line.push(b'\n');
        stdin.write_all(&line).unwrap();
        stdin.flush().unwrap();
    };
    let started = Instant::now();
    let wait_for = |id: u64| -> Value {
        loop {
            let left = DEADLINE
                .checked_sub(started.elapsed())
                .unwrap_or_else(|| panic!("no response to request {id} within {DEADLINE:?}"));
            let line = match rx.recv_timeout(left) {
                Ok(line) => line,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("no response to request {id} within {DEADLINE:?}")
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("stdout closed before the response to request {id}")
                }
            };
            let v: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("stdout line is not JSON: {line:?}: {e}"));
            if v["id"] == json!(id) {
                return v;
            }
        }
    };

    send(
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "mcp_cli", "version": "0"},
        }}),
    );
    let init = wait_for(1);
    assert_eq!(init["result"]["serverInfo"]["name"], "anthrex", "{init}");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18", "{init}");

    send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}));
    let list = wait_for(2);
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("{list}"))
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["task_done", "task_blocked"]);

    send(
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
            "name": "task_done", "arguments": {"summary": "done"},
        }}),
    );
    let call = wait_for(3);
    assert_eq!(call["result"]["isError"], json!(true), "{call}");
    let text = call["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.starts_with(&format!(
            "cannot reach the anthrex daemon at {}: ",
            socket.display()
        )),
        "{text:?}"
    );

    // Closing stdin ends the session and the process.
    drop(stdin);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > DEADLINE {
            let _ = child.kill();
            panic!("anthrex mcp did not exit within {DEADLINE:?} of stdin closing");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "{status:?}");

    // The whole of stdout, byte for byte: JSON-RPC 2.0 messages, one per line.
    let raw = reader.join().unwrap();
    let out = String::from_utf8(raw).expect("stdout is UTF-8");
    assert!(out.ends_with('\n'), "{out:?}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "exactly the three responses: {out:?}");
    for line in lines {
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("stdout line is not JSON: {line:?}: {e}"));
        assert_eq!(v["jsonrpc"], "2.0", "{line}");
    }
    assert!(
        !socket.exists(),
        "nothing may have created a daemon socket: {}",
        socket.display()
    );
}
