//! Best-effort, silent hook forwarding. The caller always exits zero afterwards.

use std::ffi::OsString;
use std::io::Read;
use std::time::{Duration, Instant};

use proto::{ClientKind, ClientMsg, DaemonMsg, HookSource};
use tokio::net::UnixStream;

const HOOK_DEADLINE: Duration = Duration::from_secs(1);
const HOOK_PAYLOAD_MAX: usize = 8 * 1024 * 1024;
/// Spec decision 5a: the bound on what one hook may carry back about a tool's result.
/// Well under `HOOK_PAYLOAD_MAX` (8 MiB, above), which still runs first on the raw stdin
/// bytes and is untouched by this — an over-8-MiB payload is still dropped whole. This
/// bound is smaller than the daemon's own `conversation.max_result_bytes` default
/// (16 KiB) on purpose: a hook-delivered result therefore never trips the daemon's cap,
/// and only enrichment can.
const TOOL_RESULT_SUMMARY_MAX: usize = 4 * 1024;

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

/// Spec decision 5a: bound the top-level `tool_response`, in place, rather than strip it.
///
/// `encoded` is the byte sequence we measure and truncate. For a `Value::String` it is the
/// string's own raw UTF-8 content, *not* `serde_json::to_string`'s JSON-quoted form: the
/// quoted form prepends a one-byte `"`, which shifts every following multi-byte character
/// off an even offset and makes the nearest-char-boundary search back off to an *odd*
/// byte count whenever the true 4 KiB cut point lands inside a multi-byte character (proved
/// by construction with 3000 repetitions of the two-byte `é` — the quoted-form reading
/// truncates to 4095 bytes there, the raw-content reading to exactly 4096). Any other JSON
/// type (object, array, number, bool, null) has no such raw byte form, so it falls back to
/// its compact JSON encoding — truncating that can produce a string that is no longer valid
/// JSON on its own, but it is wrapped in a fresh `Value::String` rather than re-parsed, so
/// that is harmless; only UTF-8 validity of the byte slice matters, and the char-boundary
/// search still guarantees that.
fn bound_tool_response(object: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(value) = object.get("tool_response").cloned() else {
        return;
    };
    let encoded = match &value {
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    let (tool_response, truncated) = if encoded.len() <= TOOL_RESULT_SUMMARY_MAX {
        (value, false)
    } else {
        let mut end = TOOL_RESULT_SUMMARY_MAX.min(encoded.len());
        while end > 0 && !encoded.is_char_boundary(end) {
            end -= 1;
        }
        (serde_json::Value::String(encoded[..end].to_string()), true)
    };
    object.insert("tool_response".to_string(), tool_response);
    object.insert(
        "tool_result_truncated".to_string(),
        serde_json::Value::Bool(truncated),
    );
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
        bound_tool_response(object);
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
