//! `task_result` (decision 18, Interfaces "The task result"): everything about one
//! task, at most [`TASK_RESULT_MAX_BYTES`]. Pure: it reads the model (full texts, not
//! the snapshot's), and the driver passes the two git reads in.

use serde_json::{Map, Value, json};

use super::json::{fold_all, hh_mm, label, shrink_strings, size};
use crate::run::messages::summary;
use crate::run::model::{Run, Task};

/// The answer's cap, in bytes of compact JSON.
pub const TASK_RESULT_MAX_BYTES: usize = 64 * 1024;
/// A research report's summary past the cap, in characters.
pub const RESEARCH_SUMMARY_MAX: usize = 16_000;
/// What trimming leaves of the checks and the agent rounds: the last ones.
const CHECKS_TRIMMED: usize = 3;
const ROUNDS_TRIMMED: usize = 5;

/// The driver's git reads (decision 18): `git log` of the task branch since its start
/// (`(sha7, subject)`) and its `git diff --stat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGit {
    pub commits: Vec<(String, String)>,
    pub diffstat: String,
}

/// The result for `task` of `run`. `git` is `None` for a task with no start commit
/// (no git keys at all), `Some(Err)` when the reads failed (`git: "<error>"`).
pub fn task_result(_run: &Run, task: &Task, git: Option<&Result<TaskGit, String>>) -> Value {
    let spec = &task.spec;
    let mut answer = json!({
        "task": {
            "id": spec.id,
            "title": spec.title,
            "epic": spec.epic,
            "kind": spec.kind,
            "size": task.size,
            "hub": task.hub,
            "test_mode": task.test_mode,
            "test_mode_reason": spec.test_mode_reason,
            "owns": spec.owns,
            "deps": spec.deps,
            "implicit_deps": task.implicit_deps,
            "route": task.route,
            "review_route": task.review_route,
            "state": task.state,
            "block": task.block,
            "rung": task.rung,
            "failures": task.failures,
            "bounces": task.bounces,
            "stalls": task.stalls,
            "hold": task.orch.gate_hold,
            "brief": spec.brief,
            "acceptance": spec.acceptance,
            "test_to_write": spec.test_to_write,
            "scout_refs": spec.scout_refs,
            "review_target": spec.review_target,
            "notes": task.notes,
            "messages": task.orch.messages,
            "task_notes": task.orch.worker_notes,
        },
        "done": task.done.as_ref().map(|d| json!({
            "signal": d.signal, "summary": d.summary, "test": d.test, "red": d.red,
        })),
        "checks": task.checks.iter().map(|c| json!({
            "at": hh_mm(c.at),
            "ok": c.ok,
            "code": c.code,
            "timed_out": c.timed_out,
            "summary": c.summary.clone().unwrap_or_else(|| summary(&c.tail)),
        })).collect::<Vec<_>>(),
        "proofs": task.proofs.iter().map(|p| json!({
            "at": hh_mm(p.at),
            "test": p.test,
            "red": p.red,
            "ok": p.red_failed && p.head_passed && p.matched,
        })).collect::<Vec<_>>(),
        "reviews": task.reviews.iter().map(|r| json!({
            "round": r.round,
            "runtime": r.route.runtime.label(),
            "verdict": r.verdict,
            "summary": r.summary,
            "findings": r.findings,
        })).collect::<Vec<_>>(),
        "rounds": task.rounds.iter().map(|r| json!({
            "role": label(&r.role),
            "session": r.session,
            "round": r.round,
            "runtime": r.route.runtime.label(),
            "model": r.route.model,
            "effort": r.route.effort,
            "turns": r.turns,
            "tool_calls": r.tool_calls,
            "tokens": r.usage.input.saturating_add(r.usage.cache_write).saturating_add(r.usage.output),
            "ended": r.ended,
        })).collect::<Vec<_>>(),
        "research": task.orch.research.as_ref().map(|r| json!({
            "summary": r.summary,
            "files": r.files,
            "modules": r.modules,
            "interfaces": r.interfaces,
            "risks": r.risks,
        })),
        "history": task.history.iter().map(|e| format!("{} {}", hh_mm(e.at), e.text)).collect::<Vec<_>>(),
    });
    if let (Some(map), Some(git)) = (answer.as_object_mut(), git) {
        insert_git(map, git);
    }
    fold_all(&mut answer);
    trim(&mut answer);
    answer
}

fn insert_git(map: &mut Map<String, Value>, git: &Result<TaskGit, String>) {
    match git {
        Ok(git) => {
            let commits: Vec<Value> = git
                .commits
                .iter()
                .map(|(sha, subject)| json!({"sha": sha, "subject": subject}))
                .collect();
            map.insert("commits".into(), json!(commits));
            map.insert("diffstat".into(), json!(git.diffstat));
        }
        Err(error) => {
            map.insert("git".into(), json!(error));
        }
    }
}

/// Decision 18's trimming past the cap: the last 3 checks, the last 5 rounds, the
/// research summary cut to 16 000 characters. Beyond the decision, so the cap always
/// holds: the last 20 history lines, the last 10 messages and task notes, the last 2
/// reviews, then every string cut to 2000, 500, then 200 characters.
fn trim(answer: &mut Value) {
    let fits = |a: &Value| size(a) <= TASK_RESULT_MAX_BYTES;
    let keep_last = |a: &mut Value, pointer: &str, keep: usize| {
        if let Some(Value::Array(items)) = a.pointer_mut(pointer) {
            let excess = items.len().saturating_sub(keep);
            items.drain(..excess);
        }
    };
    if fits(answer) {
        return;
    }
    keep_last(answer, "/checks", CHECKS_TRIMMED);
    keep_last(answer, "/rounds", ROUNDS_TRIMMED);
    if let Some(Value::String(text)) = answer.pointer_mut("/research/summary")
        && text.chars().nth(RESEARCH_SUMMARY_MAX).is_some()
    {
        *text = super::json::cut(text, RESEARCH_SUMMARY_MAX);
    }
    for (pointer, keep) in [
        ("/history", 20),
        ("/task/messages", 10),
        ("/task/task_notes", 10),
        ("/reviews", 2),
    ] {
        if fits(answer) {
            return;
        }
        keep_last(answer, pointer, keep);
    }
    for max in [2000, 500, 200] {
        if fits(answer) {
            return;
        }
        shrink_strings(answer, max);
    }
}

#[cfg(test)]
#[path = "result_tests.rs"]
mod tests;
