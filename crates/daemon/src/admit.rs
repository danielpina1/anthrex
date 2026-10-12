//! Milestone 9.14 decisions 16–24: admission for agents' own builds, the pure parts.
//! A `PreToolUse` hook (`anthrex admit`) asks the daemon's test scheduler for slots
//! before a heavy `Bash` command runs ([`is_heavy`], decision 18), and the daemon holds
//! the grant in a [`HoldBook`] until a hook event releases it ([`release_for`], decision
//! 22) or the sweep does. Pure: no filesystem, process, socket or clock access; every
//! `Instant` is the caller's.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::hooks::{HookKind, ParsedHook};
use crate::launch::shell_quote;

/// Decision 18 (C13): the cargo subcommands, aliases included, that build or run code.
/// `fix`, `rustc` and `rustdoc` compile as `check` and `build` do (task 7 review M5).
pub const HEAVY_CARGO_SUBCOMMANDS: [&str; 16] = [
    "build", "b", "test", "t", "check", "c", "clippy", "nextest", "bench", "doc", "d", "run", "r",
    "fix", "rustc", "rustdoc",
];
/// Cargo's global flags that take the next word as their value (unless `--flag=value`).
pub const CARGO_FLAGS_WITH_VALUE: [&str; 5] =
    ["-Z", "--config", "--color", "-C", "--manifest-path"];
/// Words that run the rest of the segment as a command.
pub const COMMAND_WRAPPERS: [&str; 6] = ["time", "nice", "env", "command", "nohup", "exec"];
/// A wrapper's options that take the next word as their value (task 7 review M1).
const WRAPPER_FLAGS_WITH_VALUE: [(&str, &str); 6] = [
    ("nice", "-n"),
    ("nice", "--adjustment"),
    ("env", "-u"),
    ("env", "--unset"),
    ("env", "-C"),
    ("env", "--chdir"),
];
/// Words that open a group or a clause and precede a command (task 7 review M2).
const RESERVED_WORDS: [&str; 9] = [
    "{", "!", "if", "then", "do", "else", "elif", "while", "until",
];
/// Programs other than `cargo` that are heavy whatever follows them.
pub const HEAVY_PROGRAMS: [&str; 1] = ["cargo-nextest"];
/// Decision 21: the hook's settings `timeout` is the wait plus this.
pub const ADMIT_HOOK_SLACK_SECS: u64 = 30;
/// Decision 21: `anthrex admit` gives up at `W + ADMIT_CLIENT_GRACE` from its start,
/// whatever it is doing; the daemon answers at W, so the client ends after it.
pub const ADMIT_CLIENT_GRACE: Duration = Duration::from_secs(10);
// The handshake (`proto::HANDSHAKE_TIMEOUT`) and a process spawn fit inside the grace, so
// a client that finishes before `W + ADMIT_CLIENT_GRACE` was ended by the daemon's answer
// (M9.14.8's `admit_returns_after_the_wait_budget_and_exits_zero`).
const _: () = assert!(ADMIT_CLIENT_GRACE.as_secs() > proto::HANDSHAKE_TIMEOUT.as_secs() + 2);
/// The most `anthrex admit` reads of its hook payload from stdin.
pub const ADMIT_PAYLOAD_MAX: usize = 1024 * 1024;
/// An ask's log label, `agent w<window> <command>`, is cut to this many characters.
pub const ADMIT_LABEL_MAX_CHARS: usize = 80;
/// Decision 22: how often the daemon sweeps holds older than `admit_hold_max_secs` and
/// holds of windows that are not live.
pub const ADMIT_SWEEP: Duration = Duration::from_secs(10);

/// A Claude session's admission hook (decision 17): the wait `W` (decision 21), the
/// slots it asks (`Some(n)`: `Want::Count(n)`; `None`: `Want::Half`, decision 20) and
/// the profile's filter prefixes, which count as heavy too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmitHook {
    pub wait_secs: u64,
    pub slots: Option<u32>,
    pub prefixes: Vec<String>,
}

/// Decision 18: whether `command` has a heavy segment. Advisory and fail-open: a
/// command it cannot see into (`bash -c "…"`, a subshell, a script) is not heavy.
pub fn is_heavy(command: &str, prefixes: &[String]) -> bool {
    let prefixes: Vec<Vec<String>> = prefixes
        .iter()
        .map(|p| words(p))
        .filter(|p| !p.is_empty())
        .collect();
    segments(command)
        .iter()
        .any(|segment| segment_is_heavy(strip_wrappers(segment), &prefixes))
}

/// `command` as shell words split into segments on unquoted `&&`, `||`, `;`, `|`, `&`,
/// newline, a `(` that starts a word and any `)`. Quotes keep their contents in one word
/// and are removed; a backslash outside single quotes keeps the next character. The `&`
/// of a redirection (`2>&1`, `&>`) is part of its word, and a redirection glued to a
/// word (`test>log`) starts a word of its own unless that word is a file descriptor.
fn segments(command: &str) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut segment: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    let end_word = |word: &mut String, in_word: &mut bool, segment: &mut Vec<String>| {
        if *in_word {
            segment.push(std::mem::take(word));
            *in_word = false;
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => word.extend(chars.next()),
                        _ => word.push(q),
                    }
                }
            }
            '\\' => {
                // A backslash-newline is a line continuation: no word, no separator.
                if let Some(next) = chars.next().filter(|n| *n != '\n') {
                    in_word = true;
                    word.push(next);
                }
            }
            '&' if word.ends_with(['>', '<']) || chars.peek() == Some(&'>') => {
                split_redirection(&mut word, &mut in_word, &mut segment);
                in_word = true;
                word.push(c);
            }
            '>' | '<' => {
                split_redirection(&mut word, &mut in_word, &mut segment);
                in_word = true;
                word.push(c);
            }
            '(' if !in_word => segments.push(std::mem::take(&mut segment)),
            ')' => {
                end_word(&mut word, &mut in_word, &mut segment);
                segments.push(std::mem::take(&mut segment));
            }
            ';' | '|' | '&' | '\n' => {
                end_word(&mut word, &mut in_word, &mut segment);
                if matches!(c, '|' | '&') && chars.peek() == Some(&c) {
                    chars.next();
                }
                segments.push(std::mem::take(&mut segment));
            }
            c if c.is_whitespace() => end_word(&mut word, &mut in_word, &mut segment),
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    end_word(&mut word, &mut in_word, &mut segment);
    segments.push(segment);
    segments.retain(|s| !s.is_empty());
    segments
}

/// Ends `word` before a redirection operator glued to it (`test>log`), unless the word
/// is the operator's file descriptor (`2>`) or an operator already (`>>`, `>&`).
fn split_redirection(word: &mut String, in_word: &mut bool, segment: &mut Vec<String>) {
    let operand = !word.is_empty()
        && !word.bytes().all(|b| b.is_ascii_digit())
        && !word.ends_with(['>', '<', '&']);
    if *in_word && operand {
        segment.push(std::mem::take(word));
        *in_word = false;
    }
}

/// `text` as shell words, as one segment would hold them.
fn words(text: &str) -> Vec<String> {
    segments(text).into_iter().flatten().collect()
}

/// `NAME=value`, as `output_filter`'s leading assignments are recognised.
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// The segment without its leading `NAME=value` words, reserved words
/// (`RESERVED_WORDS`), [`COMMAND_WRAPPERS`] and a wrapper's options (with the value of
/// each of `WRAPPER_FLAGS_WITH_VALUE`). A leading `cd <dir>` is a segment of its own,
/// since its `&&` already split it off.
fn strip_wrappers(segment: &[String]) -> &[String] {
    let mut rest = segment;
    let mut wrapper: Option<&str> = None;
    while let Some((first, tail)) = rest.split_first() {
        let first = first.as_str();
        rest = tail;
        if let Some(name) = wrapper
            && first.len() > 1
            && first.starts_with('-')
        {
            if WRAPPER_FLAGS_WITH_VALUE.contains(&(name, first)) {
                rest = rest.get(1..).unwrap_or_default();
            }
        } else if COMMAND_WRAPPERS.contains(&first) {
            wrapper = Some(first);
        } else if !is_assignment(first) && !RESERVED_WORDS.contains(&first) {
            return &segment[segment.len() - tail.len() - 1..];
        }
    }
    rest
}

fn segment_is_heavy(words: &[String], prefixes: &[Vec<String>]) -> bool {
    let Some(program) = words.first() else {
        return false;
    };
    let name = program.rsplit('/').next().unwrap_or(program);
    if name == "cargo" {
        return cargo_subcommand(&words[1..])
            .is_some_and(|sub| HEAVY_CARGO_SUBCOMMANDS.contains(&sub));
    }
    HEAVY_PROGRAMS.contains(&name) || prefixes.iter().any(|prefix| words.starts_with(prefix))
}

/// The subcommand of `cargo <args>`: after a `+<toolchain>` word and cargo's global
/// flags (with the value of each of [`CARGO_FLAGS_WITH_VALUE`]).
fn cargo_subcommand(args: &[String]) -> Option<&str> {
    let mut args = args.iter().map(String::as_str).peekable();
    if args.peek().is_some_and(|a| a.starts_with('+')) {
        args.next();
    }
    while let Some(arg) = args.next() {
        if !arg.starts_with('-') {
            return Some(arg);
        }
        if CARGO_FLAGS_WITH_VALUE.contains(&arg) {
            args.next();
        }
    }
    None
}

/// The settings `"command"` of the hook: `'<exe>' admit --window <id> --wait-secs <W>
/// [--slots <n>] [--prefix '<p>']…`.
pub fn hook_command(exe: &Path, window_id: u32, hook: &AdmitHook) -> String {
    let mut command = format!(
        "{} admit --window {window_id} --wait-secs {}",
        shell_quote(&exe.display().to_string()),
        hook.wait_secs
    );
    if let Some(slots) = hook.slots {
        command.push_str(&format!(" --slots {slots}"));
    }
    for prefix in &hook.prefixes {
        command.push_str(" --prefix ");
        command.push_str(&shell_quote(prefix));
    }
    command
}

/// Appends the admission hook as the last `PreToolUse` group of Claude settings, with
/// its `timeout` at `W + ADMIT_HOOK_SLACK_SECS` (decision 21). Nothing else changes;
/// `None` changes nothing.
pub fn add_hook(settings: &mut Value, exe: &Path, window_id: u32, hook: Option<&AdmitHook>) {
    let Some(hook) = hook else {
        return;
    };
    let group = json!({
        "matcher": "Bash",
        "hooks": [{
            "type": "command",
            "command": hook_command(exe, window_id, hook),
            "timeout": hook.wait_secs.saturating_add(ADMIT_HOOK_SLACK_SECS),
        }],
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

/// Which of a window's holds a hook event releases (decision 22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Release {
    /// That tool use's hold.
    One(String),
    /// Every hold of the window but that tool use's (C8 (c)).
    AllBut(String),
    /// Every hold of the window.
    Window,
    Nothing,
}

/// Decision 22: a tool use's `PostToolUse` or `PostToolUseFailure` (fact F4) releases
/// its hold; a `PreToolUse` releases the window's other holds (a call admitted and then
/// denied gets no `Post*` event); the end of a turn or a session releases them all. An
/// event without a `tool_use_id` releases nothing by id.
pub fn release_for(parsed: &ParsedHook) -> Release {
    let id = parsed.tool_use_id.clone();
    match (parsed.kind, id) {
        (HookKind::PostToolUse | HookKind::PostToolUseFailure, Some(id)) => Release::One(id),
        (HookKind::PreToolUse, Some(id)) => Release::AllBut(id),
        (
            HookKind::Stop
            | HookKind::StopFailure
            | HookKind::UserPromptSubmit
            | HookKind::SessionEnd,
            _,
        ) => Release::Window,
        _ => Release::Nothing,
    }
}

/// Granted agent asks by `(window id, tool_use_id)`, each with when it was granted
/// (decisions 22 and 23). Releasing returns the grants; the caller drops them after it
/// has let go of the book's lock.
#[derive(Debug)]
pub struct HoldBook<G> {
    holds: BTreeMap<(u32, String), (G, Instant)>,
}

impl<G> Default for HoldBook<G> {
    fn default() -> Self {
        HoldBook {
            holds: BTreeMap::new(),
        }
    }
}

impl<G> HoldBook<G> {
    /// The grant held for this tool use (decision 23: a repeated ask takes nothing new).
    pub fn get(&self, window: u32, tool_use_id: &str) -> Option<&G> {
        self.holds
            .get(&(window, tool_use_id.to_string()))
            .map(|(grant, _)| grant)
    }

    /// Holds `grant` from `now`. The caller checks [`HoldBook::get`] first.
    pub fn insert(&mut self, window: u32, tool_use_id: String, grant: G, now: Instant) {
        self.holds.insert((window, tool_use_id), (grant, now));
    }

    /// Removes and returns the window's holds that `release` names.
    pub fn release(&mut self, window: u32, release: &Release) -> Vec<G> {
        self.take(|(w, id), _| {
            *w == window
                && match release {
                    Release::One(one) => id == one,
                    Release::AllBut(kept) => id != kept,
                    Release::Window => true,
                    Release::Nothing => false,
                }
        })
    }

    /// Removes and returns every hold older than `max_age` at `now`, and every hold of
    /// a window `live` says is gone (decision 22).
    pub fn sweep(&mut self, now: Instant, max_age: Duration, live: &dyn Fn(u32) -> bool) -> Vec<G> {
        self.take(|(window, _), at| now.saturating_duration_since(at) > max_age || !live(*window))
    }

    fn take(&mut self, mut released: impl FnMut(&(u32, String), Instant) -> bool) -> Vec<G> {
        let keys: Vec<(u32, String)> = self
            .holds
            .iter()
            .filter(|(key, (_, at))| released(key, *at))
            .map(|(key, _)| key.clone())
            .collect();
        keys.into_iter()
            .filter_map(|key| self.holds.remove(&key))
            .map(|(grant, _)| grant)
            .collect()
    }
}

#[cfg(test)]
#[path = "admit_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "admit_matcher_tests.rs"]
mod matcher_tests;
