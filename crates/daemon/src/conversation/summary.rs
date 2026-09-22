//! Per-tool one-line summaries (spec decision 5). Pure: no I/O, no clock, never panics.

use serde_json::{Map, Value};
use unicode_segmentation::UnicodeSegmentation;

/// The grapheme-cluster cap every summary is truncated to (spec decision 5), counted the
/// same way `manager::validate_name` counts a window name, for the same reason: this text
/// is rendered in a terminal, where a byte or `char` count can split a multi-codepoint
/// cluster and corrupt the display.
pub const SUMMARY_MAX_GRAPHEMES: usize = 80;

/// The fallback keys tried, in order, for a tool name not covered by an explicit rule
/// below. The first key whose value is a JSON *string* wins; a non-string value at an
/// earlier key is skipped, not stringified.
const FALLBACK_KEYS: [&str; 8] = [
    "file_path",
    "path",
    "command",
    "pattern",
    "url",
    "query",
    "description",
    "prompt",
];

/// One human-readable line describing a tool call, for the conversation view's collapsed
/// row. Always returns a `String`, possibly empty, and never panics on any JSON shape --
/// `input` comes from an agent's own hook payload, which this crate does not control.
pub fn for_tool(name: &str, input: Option<&Value>) -> String {
    let Some(obj) = input.and_then(Value::as_object) else {
        return String::new();
    };
    let raw = match name {
        "Edit" => summarize_edit(obj),
        "MultiEdit" => summarize_multi_edit(obj),
        "Write" => summarize_write(obj),
        "NotebookEdit" => basename(str_field(obj, "notebook_path")).to_string(),
        "Read" => summarize_read(obj),
        "Bash" => first_line(str_field(obj, "command")).to_string(),
        "Grep" => summarize_grep(obj),
        "Glob" => str_field(obj, "pattern").to_string(),
        "Task" | "Agent" => summarize_task_or_agent(obj),
        "WebFetch" => web_host(str_field(obj, "url")).to_string(),
        "TodoWrite" => format!("{} items", array_len(obj, "todos")),
        _ => fallback(obj),
    };
    truncate_graphemes(&raw)
}

/// A key's value when it is a JSON string, `""` otherwise (missing key, wrong type, or
/// `null` all take this branch -- none of them is a reason to panic).
fn str_field<'a>(obj: &'a Map<String, Value>, key: &str) -> &'a str {
    obj.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The part of `path` after its last `/`, or the whole string when there is none.
fn basename(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[idx + 1..],
        None => path,
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

fn array_len(obj: &Map<String, Value>, key: &str) -> usize {
    obj.get(key).and_then(Value::as_array).map_or(0, Vec::len)
}

fn summarize_edit(obj: &Map<String, Value>) -> String {
    format!("{} — 1 hunk", basename(str_field(obj, "file_path")))
}

fn summarize_multi_edit(obj: &Map<String, Value>) -> String {
    let n = array_len(obj, "edits");
    let noun = if n == 1 { "hunk" } else { "hunks" };
    format!("{} — {n} {noun}", basename(str_field(obj, "file_path")))
}

fn summarize_write(obj: &Map<String, Value>) -> String {
    let n = str_field(obj, "content").lines().count();
    format!("{} — {n} lines", basename(str_field(obj, "file_path")))
}

fn summarize_read(obj: &Map<String, Value>) -> String {
    let base = basename(str_field(obj, "file_path"));
    let offset = obj.get("offset").and_then(|v| match v.as_i64() {
        Some(n) => Some(n.to_string()),
        None => v.as_u64().map(|n| n.to_string()),
    });
    match offset {
        Some(o) => format!("{base}:{o}"),
        None => base.to_string(),
    }
}

fn summarize_grep(obj: &Map<String, Value>) -> String {
    let mut out = format!("\"{}\"", str_field(obj, "pattern"));
    if let Some(path) = obj.get("path").and_then(Value::as_str) {
        out.push_str(" in ");
        out.push_str(basename(path));
    }
    out
}

fn summarize_task_or_agent(obj: &Map<String, Value>) -> String {
    let subagent_type = obj.get("subagent_type").and_then(Value::as_str);
    let description = obj.get("description").and_then(Value::as_str);
    match (subagent_type, description) {
        (Some(t), Some(d)) => format!("{t} · {d}"),
        (Some(t), None) => t.to_string(),
        (None, Some(d)) => d.to_string(),
        (None, None) => String::new(),
    }
}

/// Everything between `"://"` and the next `/`, or the whole value when there is no
/// `"://"`.
fn web_host(url: &str) -> &str {
    match url.find("://") {
        Some(idx) => {
            let after = &url[idx + 3..];
            after.split('/').next().unwrap_or("")
        }
        None => url,
    }
}

fn fallback(obj: &Map<String, Value>) -> String {
    for key in FALLBACK_KEYS {
        if let Some(s) = obj.get(key).and_then(Value::as_str) {
            return s.to_string();
        }
    }
    String::new()
}

/// Truncates `s` to [`SUMMARY_MAX_GRAPHEMES`] grapheme clusters, appending `"…"` when
/// truncation occurred. Counting and slicing by grapheme cluster, not by byte or `char`,
/// keeps a multi-codepoint cluster (a family emoji, a combining-mark sequence) intact --
/// slicing by `char` would split it and corrupt what the terminal renders.
///
/// `pub(super)`: `build::apply`'s `ToolResult.summary` (task M6.5.5) truncates a hook's
/// `tool_response` text the same way and by the same cap, so it reuses this rather than
/// duplicating the grapheme-cluster-safe truncation logic.
pub(super) fn truncate_graphemes(s: &str) -> String {
    let graphemes: Vec<&str> = s.graphemes(true).collect();
    if graphemes.len() <= SUMMARY_MAX_GRAPHEMES {
        return s.to_string();
    }
    let mut truncated: String = graphemes[..SUMMARY_MAX_GRAPHEMES].concat();
    truncated.push('…');
    truncated
}

#[cfg(test)]
#[path = "summary_tests.rs"]
mod tests;
