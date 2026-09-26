//! The output filter (milestone 8b decisions 26 to 28): what a headless Claude worker
//! reads of a long test or check command. A `PreToolUse` hook (`anthrex filter-hook`)
//! rewrites a matching `Bash` command into `anthrex filter-run … -c '<command>'`, which
//! runs it unchanged, logs every byte under the task's `TMPDIR`, and prints only the
//! view [`apply`] keeps. Pure: no filesystem, process or clock access.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use proto::OutputFilter;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::launch::shell_quote;

/// The lines `failures-only` keeps with their context (decision 26).
pub const FAILURE_RE: &str = r"(?i)\b(fail(ed|ure|ures|s)?|error(s)?|panic(ked|s)?|assert(ion)?|expected|traceback|exception)\b";
/// Every kept line is cut to this many characters.
pub const LINE_MAX_CHARS: usize = 500;
/// The log directory's name under the task's `TMPDIR` (decision 28).
pub const LOG_DIR_NAME: &str = "anthrex-logs";
/// M8b.1 item 3: Claude Code 2.1.280 applies `updatedInput` without
/// `"permissionDecision":"allow"`, so the hook never adds it.
pub const HOOK_SETS_ALLOW: bool = false;
/// How long `anthrex filter-hook` may take, reading its payload included (decision 28).
pub const FILTER_HOOK_DEADLINE: Duration = Duration::from_secs(1);
/// The largest payload `anthrex filter-hook` reads; a larger one is left alone.
pub const FILTER_HOOK_PAYLOAD_MAX: usize = 1024 * 1024;
/// The longest wrapped command [`rewrite`] produces; a longer one is left alone, so
/// the wrapper never turns a command that runs into one `exec` refuses (`E2BIG`).
pub const WRAPPED_MAX: usize = 128 * 1024;

const TAIL_LINES: usize = 60;
const SUCCESS_LINES: usize = 10;
const CONTEXT_AFTER: usize = 5;
const MATCHED_MAX: usize = 100;
const FAILURE_TAIL: usize = 20;

static FAILURE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(FAILURE_RE).expect("FAILURE_RE compiles"));

/// A Claude worker's filter hook (`HeadlessSpec.output_filter`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterHook {
    pub mode: OutputFilter,
    pub prefixes: Vec<String>,
    pub log_dir: PathBuf,
}

/// The mode's command-line spelling (its serde name).
pub fn mode_label(mode: OutputFilter) -> &'static str {
    match mode {
        OutputFilter::FailuresOnly => "failures-only",
        OutputFilter::Tail => "tail",
        OutputFilter::None => "none",
    }
}

/// The inverse of [`mode_label`].
pub fn parse_mode(label: &str) -> Option<OutputFilter> {
    [
        OutputFilter::FailuresOnly,
        OutputFilter::Tail,
        OutputFilter::None,
    ]
    .into_iter()
    .find(|mode| mode_label(*mode) == label)
}

/// Decision 26: the view of `lines` that `mode` keeps for a command that exited with
/// `exit_code`, each kept line cut to [`LINE_MAX_CHARS`].
pub fn apply(mode: OutputFilter, lines: &[String], exit_code: i32) -> Vec<String> {
    let len = lines.len();
    let (kept, marked) = match mode {
        OutputFilter::None => (vec![last(len, len)], false),
        OutputFilter::Tail => (vec![last(len, TAIL_LINES)], false),
        OutputFilter::FailuresOnly if exit_code == 0 => (vec![last(len, SUCCESS_LINES)], false),
        OutputFilter::FailuresOnly => match failure_ranges(lines) {
            Some(ranges) => (ranges, true),
            None => (vec![last(len, TAIL_LINES)], false),
        },
    };
    let mut out = Vec::new();
    let mut next = 0;
    for range in kept {
        if marked && range.start > next {
            out.push(format!(
                "[anthrex] … {} lines omitted …",
                range.start - next
            ));
        }
        out.extend(lines[range.clone()].iter().map(|line| cut(line)));
        next = range.end;
    }
    out
}

fn last(len: usize, n: usize) -> std::ops::Range<usize> {
    len.saturating_sub(n)..len
}

/// Every matching line with its [`CONTEXT_AFTER`] followers, merged, at most
/// [`MATCHED_MAX`] lines of them, then the last [`FAILURE_TAIL`] lines, sorted and
/// disjoint; `None` when nothing matches.
fn failure_ranges(lines: &[String]) -> Option<Vec<std::ops::Range<usize>>> {
    let len = lines.len();
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut budget = MATCHED_MAX;
    for (at, line) in lines.iter().enumerate() {
        if budget == 0 {
            break;
        }
        if !FAILURE.is_match(line) {
            continue;
        }
        let end = (at + 1 + CONTEXT_AFTER).min(len);
        let start = match ranges.last() {
            Some(prev) if at <= prev.end => prev.end,
            _ => at,
        };
        if end <= start {
            continue;
        }
        let end = end.min(start + budget);
        budget -= end - start;
        match ranges.last_mut() {
            Some(prev) if prev.end == start => prev.end = end,
            _ => ranges.push(start..end),
        }
    }
    if ranges.is_empty() {
        return None;
    }
    let mut tail = last(len, FAILURE_TAIL);
    while let Some(prev) = ranges.last() {
        if prev.end < tail.start {
            break;
        }
        tail.start = tail.start.min(prev.start);
        ranges.pop();
    }
    ranges.push(tail);
    Some(ranges)
}

fn cut(line: &str) -> String {
    match line.char_indices().nth(LINE_MAX_CHARS) {
        Some((at, _)) => line[..at].to_string(),
        None => line.to_string(),
    }
}

/// Whether `command` is one the hook wraps: after leading `cd <dir> && ` segments and
/// `NAME=value ` assignments, it starts with a prefix followed by the end or
/// whitespace, and it is not already a `filter-run` command (decision 28).
pub fn matches(command: &str, prefixes: &[String]) -> bool {
    if command.contains(" filter-run ") {
        return false;
    }
    let rest = strip_leading(command);
    prefixes.iter().any(|prefix| {
        !prefix.is_empty()
            && rest
                .strip_prefix(prefix.as_str())
                .is_some_and(|after| after.is_empty() || after.starts_with(char::is_whitespace))
    })
}

/// `command` split after its leading `cd <non-space> && ` segments: those segments
/// (with the whitespace after them), and the rest.
fn split_cd(command: &str) -> (&str, &str) {
    let mut rest = command.trim_start();
    while let Some(after) = strip_cd(rest) {
        rest = after.trim_start();
    }
    command.split_at(command.len() - rest.len())
}

/// `command` without its leading `cd <non-space> && ` segments and `NAME=value `
/// assignments, in any order.
fn strip_leading(command: &str) -> &str {
    let mut rest = command.trim_start();
    loop {
        if let Some(after) = strip_cd(rest).or_else(|| strip_assignment(rest)) {
            rest = after.trim_start();
        } else {
            return rest;
        }
    }
}

fn strip_cd(text: &str) -> Option<&str> {
    let after = text.strip_prefix("cd")?;
    if !after.starts_with(char::is_whitespace) {
        return None;
    }
    let after = after.trim_start();
    let dir_end = after.find(char::is_whitespace)?;
    if dir_end == 0 || after[..dir_end].contains("&&") {
        return None;
    }
    let after = after[dir_end..].trim_start().strip_prefix("&&")?;
    after.starts_with(char::is_whitespace).then_some(after)
}

fn strip_assignment(text: &str) -> Option<&str> {
    let word_end = text.find(char::is_whitespace)?;
    let (name, _value) = text[..word_end].split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    let valid = (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric());
    valid.then(|| &text[word_end..])
}

/// The command that replaces `command`: `'<exe>' filter-run --mode <m> --log-dir
/// '<dir>' -c '<command>'`, every part shell-quoted. Leading `cd <dir> && ` segments
/// stay in front, unquoted and unchanged (ruling on review I2): the Bash tool keeps its
/// working directory between calls, so a `cd` inside the wrapper would no longer move
/// it.
pub fn wrap(exe: &Path, hook: &FilterHook, command: &str) -> String {
    let (cds, command) = split_cd(command);
    format!(
        "{cds}{} filter-run --mode {} --log-dir {} -c {}",
        shell_quote(&exe.display().to_string()),
        mode_label(hook.mode),
        shell_quote(&hook.log_dir.display().to_string()),
        shell_quote(command)
    )
}

/// The hook's answer to a `PreToolUse` payload: every original `tool_input` key with
/// `command` wrapped, or `None` when the payload is not a matching `Bash` call.
pub fn rewrite(payload: &Value, exe: &Path, hook: &FilterHook) -> Option<Value> {
    if payload.get("tool_name")?.as_str()? != "Bash" {
        return None;
    }
    let input = payload.get("tool_input")?.as_object()?;
    let command = input.get("command")?.as_str()?;
    if !matches(command, &hook.prefixes) {
        return None;
    }
    let wrapped = wrap(exe, hook, command);
    if wrapped.len() > WRAPPED_MAX {
        return None;
    }
    let mut updated = input.clone();
    updated.insert("command".into(), Value::String(wrapped));
    let mut out = json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "updatedInput": updated,
    }});
    if HOOK_SETS_ALLOW {
        out["hookSpecificOutput"]["permissionDecision"] = json!("allow");
    }
    Some(out)
}

/// The settings `"command"` of the hook: `'<exe>' filter-hook --mode <m> --log-dir
/// '<dir>' --prefix '<p>'…`.
pub fn hook_command(exe: &Path, hook: &FilterHook) -> String {
    let mut command = format!(
        "{} filter-hook --mode {} --log-dir {}",
        shell_quote(&exe.display().to_string()),
        mode_label(hook.mode),
        shell_quote(&hook.log_dir.display().to_string())
    );
    for prefix in &hook.prefixes {
        command.push_str(" --prefix ");
        command.push_str(&shell_quote(prefix));
    }
    command
}

/// Appends the filter hook as a second `PreToolUse` group, after M3's, to Claude
/// settings. Nothing else changes; `None` changes nothing.
pub fn add_hook(settings: &mut Value, exe: &Path, hook: Option<&FilterHook>) {
    let Some(hook) = hook else {
        return;
    };
    let group = json!({
        "matcher": "Bash",
        "hooks": [{"type": "command", "command": hook_command(exe, hook)}],
    });
    let hooks = &mut settings["hooks"];
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let groups = &mut hooks["PreToolUse"];
    match groups {
        Value::Array(groups) => groups.push(group),
        _ => *groups = json!([group]),
    }
}

#[cfg(test)]
#[path = "output_filter_tests.rs"]
mod tests;
