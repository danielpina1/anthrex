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
fn fix_kind(task: &Task) -> Option<&'static str> {
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
/// of a review or CI fix task with a block. A delivery block of counts only
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
    let tasks = digest["tasks"].as_array().into_iter().flatten();
    tasks.into_iter().find_map(|entry| {
        if entry["block"].is_null() {
            return None;
        }
        let id = entry["id"].as_str()?;
        let kind = fix_kind(run.task(id)?)?;
        Some(format!("task {id}'s block ({kind} fix)"))
    })
}

/// Decision 19's second rule: a `task_result` of a review or CI fix task.
pub fn task_source(task: &Task) -> Option<String> {
    let kind = fix_kind(task)?;
    Some(format!("task {}'s result ({kind} fix)", task.id()))
}

#[cfg(test)]
#[path = "untrusted_tests.rs"]
pub(crate) mod tests;
