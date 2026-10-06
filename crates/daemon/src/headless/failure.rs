//! Ruling F-1 (amending M8a decision 32): which failed turns are deterministic client
//! errors. Each runtime's `classify` asks here after its own rate-limit, authentication
//! and billing rules, so the engine never matches error text itself.
//!
//! Also the text of a session that died at startup ([`startup_failure`]): a process
//! that exits before saying anything on stdout has only its stderr to explain it.

use proto::Runtime;
use serde_json::Value;

/// M8a.1 item 4b: Claude's text, on stderr before `system/init` (or in a failed
/// `result`), when `failIfUnavailable` stops it starting without its sandbox.
pub const SANDBOX_UNAVAILABLE: &str = "sandbox required but unavailable";
/// What to do about [`SANDBOX_UNAVAILABLE`]: on Linux, Claude's sandbox runs commands
/// under bubblewrap and proxies their network through socat.
pub const SANDBOX_HINT: &str = "; install bubblewrap (bwrap) and socat (on Debian or Ubuntu: sudo apt install bubblewrap socat)";
/// The stderr lines a startup failure keeps, the last ones.
pub const STARTUP_STDERR_LINES: usize = 3;
/// A startup failure's text is cut to this many bytes (invented: enough for Claude's
/// sandbox line and the hint, short enough for a status line).
pub const STARTUP_FAILURE_MAX: usize = 1000;

/// The HTTP statuses of a client error a continue cannot fix. 429 is not one: it is a
/// rate limit.
pub const CLIENT_ERROR_STATUSES: [u64; 5] = [400, 401, 403, 404, 422];

/// The API error types of a client error a continue cannot fix.
pub const CLIENT_ERROR_TYPES: [&str; 3] = [
    "invalid_request_error",
    "not_found_error",
    "permission_error",
];

/// Whether a failure with `status` (when the runtime reports one) and `text` is a
/// deterministic client error: a client status, given or found in the text (a JSON
/// error's `status`, or `API Error: <status>`), or a client error type, as a JSON
/// error's `error.type` or a word of its own in the text.
pub fn is_client_error(status: Option<u64>, text: &str) -> bool {
    let json = text
        .find('{')
        .and_then(|at| serde_json::from_str::<Value>(&text[at..]).ok());
    let json_status = json
        .as_ref()
        .and_then(|v| v.get("status"))
        .and_then(Value::as_u64);
    let api_status = text
        .trim_start()
        .strip_prefix("API Error: ")
        .and_then(|rest| {
            rest.get(..3)
                .filter(|_| !rest[3..].starts_with(|c: char| c.is_ascii_alphanumeric()))
        })
        .and_then(|digits| digits.parse::<u64>().ok());
    let client_status = [status, json_status, api_status]
        .into_iter()
        .flatten()
        .any(|s| CLIENT_ERROR_STATUSES.contains(&s));
    let json_type = json
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("type"))
        .and_then(Value::as_str);
    client_status
        || json_type.is_some_and(|t| CLIENT_ERROR_TYPES.contains(&t))
        || CLIENT_ERROR_TYPES.iter().any(|t| has_word(text, t))
}

/// Why a `runtime` process died before its first turn said anything (2026-10-06): its
/// exit, then its last stderr `lines` (a leading `Error: ` dropped from each), joined
/// with ` | `, with [`SANDBOX_HINT`] when Claude's sandbox was missing, cut to
/// [`STARTUP_FAILURE_MAX`] bytes.
pub fn startup_failure(
    runtime: Runtime,
    code: Option<i32>,
    signal: Option<i32>,
    lines: &[String],
) -> String {
    let exit = match (signal, code) {
        (Some(signal), _) => format!(" (signal {signal})"),
        (None, Some(code)) => format!(" (code {code})"),
        (None, None) => String::new(),
    };
    let mut text = format!("{} exited at startup{exit}", runtime.label());
    let said: Vec<&str> = lines
        .iter()
        .map(|line| line.trim())
        .map(|line| line.strip_prefix("Error: ").unwrap_or(line))
        .filter(|line| !line.is_empty())
        .collect();
    if !said.is_empty() {
        text = format!("{text}: {}", said.join(" | "));
    }
    bounded(&with_sandbox_hint(&text))
}

/// `text` with [`SANDBOX_HINT`] at its end when it is Claude's missing-sandbox failure
/// and does not have it yet. The hint survives the cut.
pub fn with_sandbox_hint(text: &str) -> String {
    if !text.contains(SANDBOX_UNAVAILABLE) || text.ends_with(SANDBOX_HINT) {
        return text.to_string();
    }
    let room = STARTUP_FAILURE_MAX - SANDBOX_HINT.len();
    let mut head = text.to_string();
    if head.len() > room {
        head = proto::conversation::truncate_to_char_boundary(&head, room - '…'.len_utf8());
        head.push('…');
    }
    format!("{head}{SANDBOX_HINT}")
}

fn bounded(text: &str) -> String {
    if text.len() <= STARTUP_FAILURE_MAX {
        return text.to_string();
    }
    let max = STARTUP_FAILURE_MAX - '…'.len_utf8();
    let mut cut = proto::conversation::truncate_to_char_boundary(text, max);
    cut.push('…');
    cut
}

/// The texts of a command Claude's sandbox could not start (2026-10-06, Ubuntu 24.04 and
/// later): AppArmor's `bwrap-userns-restrict` confines bubblewrap, so it cannot set up
/// its user namespace.
pub const SANDBOX_COMMAND_FAILURES: [&str; 2] =
    ["apply-seccomp", "nested userns is capability-restricted"];
/// Where Claude's docs say how to let bubblewrap create user namespaces.
pub const USERNS_HINT: &str = " (on Ubuntu 24.04 and later, allow bubblewrap to create user namespaces: see \"Ubuntu 24.04 and later\" in https://code.claude.com/docs/en/sandboxing)";

/// A failed tool result's `text`, when Claude's sandbox could not run its command: its
/// first such line, with [`USERNS_HINT`], cut to [`STARTUP_FAILURE_MAX`] bytes.
pub fn sandbox_command_failure(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| SANDBOX_COMMAND_FAILURES.iter().any(|t| line.contains(t)))?;
    let head = "Claude's sandbox could not run a command: ";
    let room = STARTUP_FAILURE_MAX - head.len() - USERNS_HINT.len() - '…'.len_utf8();
    let mut shown = proto::conversation::truncate_to_char_boundary(line, room);
    if shown.len() < line.len() {
        shown.push('…');
    }
    Some(format!("{head}{shown}{USERNS_HINT}"))
}

/// Whether `word` occurs in `text` with no ASCII letter, digit or `_` on either side.
fn has_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let part = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    text.match_indices(word).any(|(at, _)| {
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(at + word.len()).copied();
        !before.is_some_and(part) && !after.is_some_and(part)
    })
}

#[cfg(test)]
#[path = "failure_tests.rs"]
mod tests;
