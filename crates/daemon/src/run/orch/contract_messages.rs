//! The texts a worker receives from the run: messages (decision 42b), a fresh
//! session's notes and a reviewer's messages (42d), and the refresh texts (42e). Split
//! out of `contract.rs` before milestone 9.1 to keep it under the 600-line rule; a pure
//! move. Pure.

use proto::MessageKind;

use crate::run::contract::sha7;
use crate::run::messages::one_line;
use crate::run::orch::{EditSource, TaskMessage};

/// Decision 42b: a message as the worker receives it; the TUI's §12.6 label keys on the
/// prefix.
pub fn message_text(source: &EditSource, kind: MessageKind, text: &str) -> String {
    format!(
        "[anthrex] Message from the {} ({}): {text}",
        sender(source),
        message_kind_label(kind)
    )
}

/// Decision 42d: a task's recorded messages for a fresh session's prompt, oldest first;
/// empty when there are none.
pub fn notes_section(messages: &[TaskMessage]) -> String {
    if messages.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&TaskMessage> = messages.iter().collect();
    sorted.sort_by_key(|m| m.at);
    let mut lines = vec!["Notes from the orchestrator:".to_string()];
    lines.extend(sorted.iter().map(|m| {
        format!(
            "- {} ({}, from {}) {}",
            hh_mm(m.at),
            message_kind_label(m.kind),
            sender(&m.source),
            m.text
        )
    }));
    lines.join("\n")
}

/// Decision 42d: the `change` messages a reviewer is shown; empty when there are none.
pub fn worker_messages_for_review(messages: &[TaskMessage]) -> String {
    let mut changes: Vec<&TaskMessage> = messages
        .iter()
        .filter(|m| m.kind == MessageKind::Change)
        .collect();
    if changes.is_empty() {
        return String::new();
    }
    changes.sort_by_key(|m| m.at);
    let mut lines = vec!["Messages the worker received:".to_string()];
    lines.extend(
        changes
            .iter()
            .map(|m| format!("- {} (change) {}", hh_mm(m.at), m.text)),
    );
    lines.join("\n")
}

// M9.13a re-review, item 4: in the refresh texts, a commit subject and a file name are
// the repository's text, not the engine's, so each is one line (`one_line`) and cannot
// start an `[anthrex]` line of its own.

/// Decision 42e: a clean refresh merged `n` commits; `list` is `(sha, subject)`, newest
/// first, of which at most 10 are named.
pub fn refresh_clean(n: usize, list: &[(String, String)]) -> String {
    format!("{} Rebuild before you continue.", refreshed_line(n, list))
}

/// M9.13a review, item 7: [`refresh_clean`] for a `paused(message)` task, whose worker
/// must not continue until a message releases it.
pub fn refresh_clean_paused(n: usize, list: &[(String, String)]) -> String {
    format!("{} {REBUILD_AND_WAIT}", refreshed_line(n, list))
}

/// A paused task's refresh: what to do, and that it still waits.
const REBUILD_AND_WAIT: &str =
    "Rebuild, then wait for the next message: you were asked to stop and wait.";

fn refreshed_line(n: usize, list: &[(String, String)]) -> String {
    let mut named: Vec<String> = list
        .iter()
        .take(10)
        .map(|(sha, subject)| format!("{} {}", sha7(sha), one_line(subject)))
        .collect();
    let more = n.saturating_sub(named.len());
    if more > 0 {
        named.push(format!("and {more} more"));
    }
    format!(
        "[anthrex] Your branch now includes the latest merged work ({n} commits: {}).",
        named.join(", ")
    )
}

/// Decision 42e: a refresh that conflicted.
pub fn refresh_conflict(files: &[String]) -> String {
    format!(
        "[anthrex] Merging the latest run branch into your worktree conflicted in: {}. Resolve them, commit, and continue.",
        one_line(&files.join(", "))
    )
}

/// M9.13a review, item 7: [`refresh_conflict`] for a `paused(message)` task.
pub fn refresh_conflict_paused(files: &[String]) -> String {
    format!(
        "[anthrex] Merging the latest run branch into your worktree conflicted in: {}. Resolve them and commit, then wait for the next message: you were asked to stop and wait.",
        one_line(&files.join(", "))
    )
}

fn message_kind_label(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Info => "info",
        MessageKind::Change => "change",
        MessageKind::StopAndWait => "stop_and_wait",
    }
}

/// Who a message is from: only the orchestrator and the user send them (decision 42a).
fn sender(source: &EditSource) -> &'static str {
    match source {
        EditSource::User => "user",
        EditSource::Orchestrator | EditSource::Planner { .. } => "orchestrator",
    }
}

/// `hh:mm` of a Unix time, in UTC, as M8a's prompts write times.
fn hh_mm(at: u64) -> String {
    format!("{:02}:{:02}", at % 86_400 / 3600, at % 3600 / 60)
}
