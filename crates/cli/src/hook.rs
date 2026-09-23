//! Best-effort, silent hook forwarding. The caller always exits zero afterwards.

use std::ffi::OsString;
use std::io::Read;
use std::time::{Duration, Instant};

use proto::{ClientKind, ClientMsg, DaemonMsg, HookSource};
use tokio::net::UnixStream;

const HOOK_DEADLINE: Duration = Duration::from_secs(1);
const HOOK_PAYLOAD_MAX: usize = 8 * 1024 * 1024;
/// `proto::conversation::TOOL_RESULT_SUMMARY_MAX` (moved there by M8a.7, so the daemon can
/// bound a synthesised `tool_response` exactly as this does) must stay within the smallest
/// legal `conversation.max_result_bytes`; see its own comment.
const _: () = assert!(
    proto::conversation::TOOL_RESULT_SUMMARY_MAX as u64
        <= config::CONVERSATION_MAX_RESULT_BYTES_MIN
);

pub fn run(args: Vec<OsString>, started: Instant) {
    std::panic::set_hook(Box::new(|_| {}));
    let (done, completed) = std::sync::mpsc::channel();
    // The main thread owns the deadline, even if parsing, encoding, stdin or
    // runtime teardown stalls. process::exit in main also ends detached readers.
    let _ = std::thread::Builder::new()
        .name("hook".into())
        .spawn(move || {
            if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                runtime.block_on(async {
                    // A panic or ordinary forwarding error is intentionally silent.
                    let _ = tokio::spawn(forward(args)).await;
                });
            }
            let _ = done.send(());
        });
    let _ = completed.recv_timeout(HOOK_DEADLINE.saturating_sub(started.elapsed()));
}

struct Arguments {
    window_id: u32,
    source: HookSource,
    payload: Option<Vec<u8>>,
}

fn parse(args: Vec<OsString>) -> Option<Arguments> {
    let mut args = args.into_iter();
    let mut window_id = None;
    let mut source = None;
    let mut payload = None;
    while let Some(arg) = args.next() {
        let arg = arg.into_string().ok()?;
        match arg.as_str() {
            "--window" => window_id = Some(args.next()?.into_string().ok()?.parse().ok()?),
            "--source" => {
                source = Some(match args.next()?.to_str()? {
                    "claude" => HookSource::Claude,
                    "codex-notify" => HookSource::CodexNotify,
                    "codex-hook" => HookSource::CodexHook,
                    _ => return None,
                });
            }
            arg if arg.starts_with('-') => return None,
            _ => payload = Some(arg.into_bytes()),
        }
    }
    Some(Arguments {
        window_id: window_id?,
        source: source?,
        payload,
    })
}

async fn stdin_payload() -> Option<Vec<u8>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    // Do not use spawn_blocking: runtime shutdown would wait for held-open stdin.
    std::thread::Builder::new()
        .name("hook-stdin".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = std::io::stdin()
                .lock()
                .take((HOOK_PAYLOAD_MAX + 1) as u64)
                .read_to_end(&mut bytes);
            let _ = send.send(result.ok().map(|_| bytes));
        })
        .ok()?;
    receive.await.ok().flatten()
}

async fn forward(args: Vec<OsString>) -> Option<()> {
    let args = parse(args)?;
    let bytes = match args.payload {
        Some(bytes) => bytes,
        None => stdin_payload().await?,
    };
    if bytes.len() > HOOK_PAYLOAD_MAX {
        return None;
    }
    let mut payload: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if let Some(object) = payload.as_object_mut() {
        proto::conversation::bound_tool_response(object);
    }

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
    if !matches!(
        proto::read_frame::<_, DaemonMsg>(&mut stream).await.ok()?,
        Some(DaemonMsg::Welcome { .. })
    ) {
        return None;
    }
    proto::write_frame(
        &mut stream,
        &ClientMsg::HookEvent {
            window_id: args.window_id,
            source: args.source,
            payload,
        },
    )
    .await
    .ok()?;
    loop {
        match proto::read_frame::<_, DaemonMsg>(&mut stream)
            .await
            .ok()??
        {
            DaemonMsg::Ack { request } if request == "hook" => return Some(()),
            DaemonMsg::Error { .. } => return None,
            _ => {}
        }
    }
}
