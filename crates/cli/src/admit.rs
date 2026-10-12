//! `anthrex admit` (milestone 9.14 decisions 16–21): a Claude `PreToolUse` hook that
//! asks the daemon for test slots before a heavy `Bash` command and blocks until a
//! grant or the wait runs out. Fail-open and silent: it always exits 0 with no output,
//! and it never rewrites the command.
//!
//! `anthrex admit --window <id> --wait-secs <W> [--slots <n>] [--prefix <p>]…`, with
//! the hook payload on stdin. A call that is not a heavy `Bash` call with a
//! `tool_use_id` leaves at once without connecting (decisions 18 and 22). The whole
//! process is bounded by `W + ADMIT_CLIENT_GRACE` from its start, in the main thread.

use std::ffi::OsString;
use std::io::Read;
use std::time::{Duration, Instant};

use daemon::admit::{ADMIT_CLIENT_GRACE, ADMIT_LABEL_MAX_CHARS, ADMIT_PAYLOAD_MAX, is_heavy};
use proto::{ClientKind, ClientMsg, DaemonMsg};
use tokio::net::UnixStream;

struct Arguments {
    window_id: u32,
    wait_secs: u64,
    slots: Option<u32>,
    prefixes: Vec<String>,
}

pub fn run(args: Vec<OsString>, started: Instant) {
    std::panic::set_hook(Box::new(|_| {}));
    let Some(args) = parse(args) else {
        return;
    };
    let deadline = Duration::from_secs(args.wait_secs).saturating_add(ADMIT_CLIENT_GRACE);
    let (done, completed) = std::sync::mpsc::channel();
    // The main thread owns the deadline, whatever stalls: stdin, the connection or the
    // runtime's teardown. `process::exit` in main also ends a detached reader.
    let _ = std::thread::Builder::new()
        .name("admit".into())
        .spawn(move || {
            if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                runtime.block_on(async {
                    let _ = tokio::spawn(ask(args)).await;
                });
            }
            let _ = done.send(());
        });
    let _ = completed.recv_timeout(deadline.saturating_sub(started.elapsed()));
}

fn parse(args: Vec<OsString>) -> Option<Arguments> {
    let mut args = args.into_iter();
    let (mut window_id, mut wait_secs, mut slots) = (None, None, None);
    let mut prefixes = Vec::new();
    while let Some(arg) = args.next() {
        let mut value = || args.next()?.into_string().ok();
        match arg.to_str()? {
            "--window" => window_id = Some(value()?.parse().ok()?),
            "--wait-secs" => wait_secs = Some(value()?.parse().ok()?),
            "--slots" => slots = Some(value()?.parse().ok()?),
            "--prefix" => prefixes.push(value()?),
            _ => return None,
        }
    }
    Some(Arguments {
        window_id: window_id?,
        wait_secs: wait_secs?,
        slots,
        prefixes,
    })
}

async fn stdin_payload() -> Option<Vec<u8>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    // Not `spawn_blocking`: the runtime's shutdown would wait for a held-open stdin.
    std::thread::Builder::new()
        .name("admit-stdin".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = std::io::stdin()
                .lock()
                .take((ADMIT_PAYLOAD_MAX + 1) as u64)
                .read_to_end(&mut bytes);
            let _ = send.send(result.ok().map(|_| bytes));
        })
        .ok()?;
    receive.await.ok().flatten()
}

/// The `tool_use_id` and command of a heavy `Bash` call, else `None`.
fn heavy_call(payload: &serde_json::Value, prefixes: &[String]) -> Option<(String, String)> {
    if payload["tool_name"] != "Bash" {
        return None;
    }
    let id = payload["tool_use_id"]
        .as_str()
        .filter(|id| !id.is_empty())?;
    let command = payload["tool_input"]["command"].as_str()?;
    is_heavy(command, prefixes).then(|| (id.to_string(), command.to_string()))
}

async fn ask(args: Arguments) -> Option<()> {
    let bytes = stdin_payload().await?;
    if bytes.len() > ADMIT_PAYLOAD_MAX {
        return None;
    }
    let payload: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let (tool_use_id, command) = heavy_call(&payload, &args.prefixes)?;
    let label: String = format!("agent w{} {command}", args.window_id)
        .chars()
        .take(ADMIT_LABEL_MAX_CHARS)
        .collect();

    let mut stream = UnixStream::connect(proto::paths::socket_path())
        .await
        .ok()?;
    proto::write_frame(
        &mut stream,
        &ClientMsg::Hello {
            proto_version: proto::PROTO_VERSION,
            client: ClientKind::Hook,
        },
    )
    .await
    .ok()?;
    let welcome = tokio::time::timeout(
        proto::HANDSHAKE_TIMEOUT,
        proto::read_frame::<_, DaemonMsg>(&mut stream),
    )
    .await
    .ok()?
    .ok()?;
    if !matches!(welcome, Some(DaemonMsg::Welcome { .. })) {
        return None;
    }
    proto::write_frame(
        &mut stream,
        &ClientMsg::Admit {
            window_id: args.window_id,
            tool_use_id,
            slots: args.slots,
            wait_secs: args.wait_secs,
            label,
        },
    )
    .await
    .ok()?;
    // Any answer ends the wait; the daemon's other traffic (git, window lists) does not.
    loop {
        match proto::read_frame::<_, DaemonMsg>(&mut stream)
            .await
            .ok()??
        {
            DaemonMsg::Admitted { .. } | DaemonMsg::AdmitTimedOut | DaemonMsg::Error { .. } => {
                return Some(());
            }
            _ => {}
        }
    }
}
