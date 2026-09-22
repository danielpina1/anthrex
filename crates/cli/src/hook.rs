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
/// bytes and is untouched by this — an over-8-MiB payload is still dropped whole.
///
/// Also `<= config::CONVERSATION_MAX_RESULT_BYTES_MIN`, the smallest legally configured
/// `conversation.max_result_bytes` (amendment, review finding F3: the original comment
/// claimed this against only the *default* `max_result_bytes` (16 KiB), which is false at
/// the low end of the range that existed at the time — `max_result_bytes = 2048` was legal
/// and a 4096-byte hook result would trip it). `crates/config` does not define
/// `[conversation]` until task M6.5.3, which raises the range's floor to match this
/// constant and adds a `const _: () = assert!(...)` here to keep the two compiled together
/// from then on — see that task's acceptance criteria.
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
/// Amended by wave-1 review finding F2: an over-limit *object* must not collapse into a
/// plain string, because M6.5.5's `ok` predicate reads an object's `error`/`success` keys
/// to tell a failed tool from a succeeded one — a stringified blob reads as `ok == true` no
/// matter what it said, silently turning a failure into a success. So an object keeps its
/// shape whenever it can be made to fit by shrinking its own string leaves; it is only
/// collapsed to a string as a last resort, and that resort is flagged separately
/// (`tool_result_stringified`) from ordinary truncation so the daemon can tell the two
/// apart.
fn bound_tool_response(object: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(value) = object.get("tool_response").cloned() else {
        return;
    };
    let (tool_response, truncated, stringified) = bound_tool_response_value(value);
    object.insert("tool_response".to_string(), tool_response);
    object.insert(
        "tool_result_truncated".to_string(),
        serde_json::Value::Bool(truncated),
    );
    object.insert(
        "tool_result_stringified".to_string(),
        serde_json::Value::Bool(stringified),
    );
}

/// Truncates one already-extracted `tool_response` value to `TOOL_RESULT_SUMMARY_MAX`,
/// returning the (possibly rewritten) value, whether it was truncated, and whether an
/// object was collapsed into a string in the process (see `bound_tool_response`).
///
/// A `Value::String` is truncated on its own raw UTF-8 content, not on
/// `serde_json::to_string(&value)`'s JSON-quoted form. The reason is semantic: the field
/// carries the result's *text*, so the text is what gets truncated — matching M6.5.5's
/// `summary` rule, which truncates the same field the same way. (An earlier version of
/// this comment argued from byte parity instead — a leading JSON quote shifts every
/// following multi-byte character off an even offset — but that argument is fixture-
/// specific: it happens to distinguish the two readings for a run of 2-byte characters
/// like `é`, and inverts for a run of 3-byte characters like `世`. The semantic argument
/// holds regardless of character width; the parity argument does not, and is not the
/// reason for this choice.)
fn bound_tool_response_value(value: serde_json::Value) -> (serde_json::Value, bool, bool) {
    match value {
        serde_json::Value::String(s) => {
            if s.len() <= TOOL_RESULT_SUMMARY_MAX {
                (serde_json::Value::String(s), false, false)
            } else {
                (
                    serde_json::Value::String(truncate_to_char_boundary(
                        &s,
                        TOOL_RESULT_SUMMARY_MAX,
                    )),
                    true,
                    false,
                )
            }
        }
        serde_json::Value::Object(map) => bound_object(map),
        other => {
            let encoded = serde_json::to_string(&other).unwrap_or_default();
            if encoded.len() <= TOOL_RESULT_SUMMARY_MAX {
                (other, false, false)
            } else {
                (
                    serde_json::Value::String(truncate_to_char_boundary(
                        &encoded,
                        TOOL_RESULT_SUMMARY_MAX,
                    )),
                    true,
                    false,
                )
            }
        }
    }
}

/// The first `max` bytes of `s`, backed off to the nearest char boundary so the result is
/// always valid UTF-8 (slicing a `String` at a byte index inside a multi-byte character
/// panics).
fn truncate_to_char_boundary(s: &str, max: usize) -> String {
    let mut end = max.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// An over-limit JSON object keeps its shape by shrinking its own string leaves — the
/// single longest one first, however many bytes the whole re-encoded object is still over
/// budget by, backed off to a char boundary — walking into nested objects and arrays, until
/// its compact encoding fits or every string leaf is already empty. Each pass strictly
/// shrinks the targeted leaf (the byte count removed is always at least 1, since `overage`
/// is always at least 1 whenever this loop runs), so it always terminates on its own inputs
/// — bounded by the total bytes held in string leaves at the start, itself bounded by
/// `HOOK_PAYLOAD_MAX` — with no separate iteration cap needed.
///
/// Falls back to a truncated, stringified blob only when the object's own skeleton (keys,
/// braces, commas, non-string values) alone exceeds the bound even with every string leaf
/// emptied — a pathological key count. That fallback is flagged with the second return
/// value (`tool_result_stringified`) precisely because it is the one path that can hide a
/// discriminating key like `error` or `success` from a downstream reader that only inspects
/// `Value::Object`.
fn bound_object(
    map: serde_json::Map<String, serde_json::Value>,
) -> (serde_json::Value, bool, bool) {
    let original = serde_json::Value::Object(map);
    let encoded_len = |v: &serde_json::Value| {
        serde_json::to_string(v)
            .map(|s| s.len())
            .unwrap_or(usize::MAX)
    };
    if encoded_len(&original) <= TOOL_RESULT_SUMMARY_MAX {
        return (original, false, false);
    }
    let mut shrinking = original.clone();
    loop {
        let current = encoded_len(&shrinking);
        if current <= TOOL_RESULT_SUMMARY_MAX {
            return (shrinking, true, false);
        }
        let overage = current - TOOL_RESULT_SUMMARY_MAX;
        match largest_string_leaf(&mut shrinking) {
            Some(leaf) if !leaf.is_empty() => {
                let target = leaf.len().saturating_sub(overage);
                let mut end = target.min(leaf.len());
                while end > 0 && !leaf.is_char_boundary(end) {
                    end -= 1;
                }
                leaf.truncate(end);
            }
            _ => break,
        }
    }
    let encoded = serde_json::to_string(&original).unwrap_or_default();
    let stringified = truncate_to_char_boundary(&encoded, TOOL_RESULT_SUMMARY_MAX);
    (serde_json::Value::String(stringified), true, true)
}

/// Depth-first search for the longest non-empty string value anywhere inside `value`
/// (recursing through objects and arrays), returning a mutable handle to it so the caller
/// can shrink it in place. `None` once every string leaf is empty (or there are none).
fn largest_string_leaf(value: &mut serde_json::Value) -> Option<&mut String> {
    match value {
        serde_json::Value::String(s) if !s.is_empty() => Some(s),
        serde_json::Value::Array(items) => items
            .iter_mut()
            .filter_map(largest_string_leaf)
            .max_by_key(|s| s.len()),
        serde_json::Value::Object(map) => map
            .values_mut()
            .filter_map(largest_string_leaf)
            .max_by_key(|s| s.len()),
        _ => None,
    }
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
