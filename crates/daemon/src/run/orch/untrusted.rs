//! Milestone 9.13 decisions 18 and 19 (NA §2 D1, D2): the sticky record that the run's
//! orchestrator read text from outside the run, and where such text reaches it. Pure.
//!
//! The sources are structural: a pull-request comment the digest quoted, or the block
//! or result of a review or CI fix task. Only the orchestrator's own reads count; the
//! driver computes the source on the blocking thread that builds the answer, and the
//! engine keeps the first ([`OrchEvent::UntrustedRead`]).
//!
//! [`OrchEvent::UntrustedRead`]: crate::run::engine::OrchEvent::UntrustedRead

use proto::TaskOrigin;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::run::model::{Run, Task};

/// `RunOrch.untrusted`: when the session first read text from outside the run, and
/// what it read. Never cleared while the session lives (decision 18).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UntrustedRecord {
    pub at: u64,
    pub source: String,
}

impl UntrustedRecord {
    /// The record as the snapshot shows it.
    pub fn info(&self) -> proto::UntrustedInfo {
        proto::UntrustedInfo {
            at: self.at,
            source: self.source.clone(),
        }
    }
}

/// Decision 20's run log line.
pub fn read_line(source: &str) -> String {
    format!(
        "the orchestrator read {source}; override, accept_red and approve_hold now go to the user"
    )
}

/// `review` for a review fix task (origin `review`, or one that addresses threads),
/// `ci` for a CI fix task; `None` for any other task.
pub(crate) fn fix_kind(task: &Task) -> Option<&'static str> {
    if task.origin == TaskOrigin::Review || !task.spec.addresses.is_empty() {
        Some("review")
    } else if task.origin == TaskOrigin::Ci {
        Some("ci")
    } else {
        None
    }
}

/// Decision 19's first rule, on `run_status`'s digest exactly as answered (after
/// trimming): the first stage whose threads quote a comment, else the first task entry
/// of a review or CI fix task with a block, or whose `last` is its block line. A delivery block of counts only
/// (`threads_omitted`) or a thread with `comment: null` quotes nothing.
pub fn digest_source(digest: &Value, run: &Run) -> Option<String> {
    let stages = digest["delivery"]["stages"]
        .as_array()
        .into_iter()
        .flatten();
    for stage in stages {
        let threads = stage["threads"].as_array().into_iter().flatten();
        if threads.into_iter().any(|t| t["comment"].is_string()) {
            return Some(format!(
                "review comments on stage {}'s pull request",
                stage["stage"]
            ));
        }
    }
    let tasks = || digest["tasks"].as_array().into_iter().flatten();
    let fix = |id: &str| run.task(id).and_then(fix_kind);
    // Controller ruling (task 11): a cleared block whose line is still the entry's
    // `last` (`{HH:MM} blocked ({reason}): …`, `dispatch::block`) shows the same text.
    let is_block_line = |last: &str| line_text(last).starts_with("blocked (");
    let block = tasks().find_map(|entry| {
        let last_blocked = entry["last"].as_str().is_some_and(is_block_line);
        if entry["block"].is_null() && !last_blocked {
            return None;
        }
        let id = entry["id"].as_str()?;
        Some(format!("task {id}'s block ({} fix)", fix(id)?))
    });
    block.or_else(|| note_source(digest, tasks(), fix))
}

/// Task 8 review I1 (ruling): a fix task's note the digest shows, in `task_notes`, as
/// its task entry's `last` (`{HH:MM} note ({kind}): …`), or as an `attention` line
/// (`{id} noted a {kind}: …`).
fn note_source<'a>(
    digest: &'a Value,
    tasks: impl Iterator<Item = &'a Value>,
    fix: impl Fn(&str) -> Option<&'static str>,
) -> Option<String> {
    let line = |id: &str| Some(format!("task {id}'s note ({} fix)", fix(id)?));
    let notes = digest["task_notes"].as_array().into_iter().flatten();
    let in_notes = notes.filter_map(|n| n["task"].as_str()).find_map(line);
    let is_note = |last: &str| line_text(last).starts_with("note (");
    in_notes
        .or_else(|| {
            tasks
                .filter(|e| e["last"].as_str().is_some_and(is_note))
                .filter_map(|e| e["id"].as_str())
                .find_map(line)
        })
        .or_else(|| {
            let attention = digest["attention"].as_array().into_iter().flatten();
            attention
                .filter_map(|l| l.as_str()?.split_once(" noted a ").map(|(id, _)| id))
                .find_map(line)
        })
}

/// A task entry's `last` (`{HH:MM} {text}`) without its time, and without the
/// `racer {lane}: ` prefix a racer's lines keep once its lane's view ends
/// (`engine/race_view.rs`; PR #60 review I1).
fn line_text(last: &str) -> &str {
    let text = last.split_once(' ').map_or("", |(_, t)| t);
    text.strip_prefix("racer ")
        .and_then(|rest| rest.split_once(": "))
        .filter(|(lane, _)| !lane.is_empty() && !lane.contains(' '))
        .map_or(text, |(_, t)| t)
}

/// Decision 19's second rule: a `task_result` of a review or CI fix task.
pub fn task_source(task: &Task) -> Option<String> {
    let kind = fix_kind(task)?;
    Some(format!("task {}'s result ({kind} fix)", task.id()))
}

#[cfg(test)]
#[path = "untrusted_tests.rs"]
pub(crate) mod tests;
