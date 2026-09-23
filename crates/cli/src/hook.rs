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
/// and a 4096-byte hook result would trip it). Task M6.5.3 raised `[conversation]`'s range
/// floor to match this constant and, below, asserts the relationship at compile time so the
/// two can never drift apart silently again.
const TOOL_RESULT_SUMMARY_MAX: usize = 4 * 1024;
const _: () = assert!(TOOL_RESULT_SUMMARY_MAX as u64 <= config::CONVERSATION_MAX_RESULT_BYTES_MIN);

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
/// Amended by wave-1 review finding F2, then amended again by wave 2: an over-limit
/// *object* never collapses into a plain string at all, in any circumstance. M6.5.5's `ok`
/// predicate reads an object's `error`/`success` keys to tell a failed tool from a succeeded
/// one, and `ToolResult.ok` is a plain `bool` with no honest third "unknown" state — so a
/// choice between "sometimes read a failure as a success" and "sometimes read a success as a
/// failure" is a defect either way, not a tradeoff to make. The wave-2 fix instead
/// guarantees the object always keeps its shape: `bound_object` builds the bounded object up
/// from nothing (`error` first, then `success`, then whatever else fits), rather than
/// shrinking the original down and falling back to text when shrinking runs out of leaves to
/// cut. `tool_result_stringified` still exists, but only ever becomes `true` for a type with
/// no `ok` predicate to protect (array, number, bool, null) — see `bound_tool_response_value`.
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
        // Array, Number, Bool, Null: none of these carry an `ok` predicate downstream (only
        // a `Value::Object`'s `error`/`success` keys do), so there is no structure to
        // protect and converting one to text loses nothing M6.5.5 reads. `stringified` is
        // `true` exactly when that conversion actually happened (wave-2 review ruling): it
        // is honest documentation of what occurred, not a signal anything needs to branch
        // on, since M6.5.5's documented default `ok == true` for a non-object
        // `tool_response` is already correct for these types whether or not they were
        // truncated.
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
                    true,
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

/// An over-limit JSON object is rebuilt from nothing rather than shrunk down (wave-2 review
/// ruling, replacing wave 1's shrink-then-fall-back-to-text approach): `error` goes in
/// first, then `success`, each with its own value truncated if it has to be, so the two keys
/// M6.5.5's `ok` predicate reads always survive, non-null, in an object it can still call
/// `.is_object()` on. Every other key is then added, in whatever order `serde_json::Map`
/// iterates them (this workspace does not enable serde_json's `preserve_order` feature, so
/// that is sorted-by-key order, not the original payload's JSON-text order — "original
/// order" in the sense the brief means it no longer survives parsing by the time this
/// function sees the map), while the whole object's compact encoding still fits, truncating
/// a key's own string content the same way `error`/`success` are. The moment one key cannot
/// be made to fit at all, it and every key after it are dropped: the budget only shrinks as
/// keys are added, so no later key could have fit either.
///
/// The result is always a `Value::Object` — `tool_result_stringified` is always `false` for
/// this function's output; see `bound_tool_response_value` for the (unrelated) case where
/// that flag is `true`.
fn bound_object(
    map: serde_json::Map<String, serde_json::Value>,
) -> (serde_json::Value, bool, bool) {
    let encoded_len = |v: &serde_json::Value| {
        serde_json::to_string(v)
            .map(|s| s.len())
            .unwrap_or(usize::MAX)
    };
    if encoded_len(&serde_json::Value::Object(map.clone())) <= TOOL_RESULT_SUMMARY_MAX {
        return (serde_json::Value::Object(map), false, false);
    }

    let mut result = serde_json::Map::new();
    let mut truncated = false;

    // `error`/`success` are the two keys M6.5.5's `ok` predicate reads; they must survive
    // no matter what else has to give way.
    for key in ["error", "success"] {
        if let Some(value) = map.get(key) {
            insert_shrinking(&mut result, key, value.clone(), false, &mut truncated);
        }
    }

    for (key, value) in map.iter() {
        if key == "error" || key == "success" {
            continue;
        }
        if !insert_shrinking(&mut result, key, value.clone(), true, &mut truncated) {
            break;
        }
    }

    (serde_json::Value::Object(result), truncated, false)
}

/// Inserts `key: value` into `result`, shrinking `value`'s own content (its longest string
/// leaf, recursing into nested objects/arrays, same as wave 1's leaf search) as many times
/// as needed to bring the whole `result` back under `TOOL_RESULT_SUMMARY_MAX`. Returns
/// whether the key ended up present.
///
/// `allow_drop = false` (for `error`/`success`) never removes the key. If leaf-shrinking
/// alone cannot make it fit — no string leaf, or every leaf already empty — the value is
/// replaced *once* by a truncated JSON-text rendering of itself (still present, still
/// non-null), which the next pass through the loop then shrinks as an ordinary string leaf;
/// this cannot recurse a second time, because once the value is a `Value::String` the
/// `matches!` check below returns instead of re-rendering, which is what stops this from
/// oscillating forever between an emptied string and its own 2-byte re-quoted rendering (an
/// empty string is still non-null, so giving up at that point does not lose the signal
/// `error`/`success` exists to carry). `allow_drop = true` (every other key) removes the key
/// outright once it cannot be made to fit; the caller stops walking further keys at that
/// point (see `bound_object`).
fn insert_shrinking(
    result: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: serde_json::Value,
    allow_drop: bool,
    truncated: &mut bool,
) -> bool {
    let encoded_len = |v: &serde_json::Value| {
        serde_json::to_string(v)
            .map(|s| s.len())
            .unwrap_or(usize::MAX)
    };
    result.insert(key.to_string(), value);
    loop {
        let current = encoded_len(&serde_json::Value::Object(result.clone()));
        if current <= TOOL_RESULT_SUMMARY_MAX {
            return true;
        }
        let overage = current - TOOL_RESULT_SUMMARY_MAX;
        let entry = result.get_mut(key).expect("just inserted");
        match largest_string_leaf(entry) {
            Some(leaf) if !leaf.is_empty() => {
                let target = leaf.len().saturating_sub(overage);
                let mut end = target.min(leaf.len());
                while end > 0 && !leaf.is_char_boundary(end) {
                    end -= 1;
                }
                leaf.truncate(end);
                *truncated = true;
            }
            _ if allow_drop => {
                result.remove(key);
                *truncated = true;
                return false;
            }
            _ => {
                if matches!(entry, serde_json::Value::String(_)) {
                    // Already a string, and already fully shrunk: nothing more can be
                    // done. Leave it (possibly empty, still non-null) and stop.
                    *truncated = true;
                    return true;
                }
                let rendered = serde_json::to_string(entry).unwrap_or_default();
                *entry = serde_json::Value::String(rendered);
                *truncated = true;
            }
        }
    }
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
