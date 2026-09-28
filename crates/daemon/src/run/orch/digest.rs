//! `run_status`'s digest (decision 16, Interfaces "The digest"): one compact JSON object
//! of what the orchestrator must see, at most [`DIGEST_MAX_BYTES`], and the
//! fingerprint whose change bumps `Run.orch.digest_rev`. Pure: it reads the model, not
//! the snapshot, so its texts are the model's (cut as the Interfaces say).

use proto::{BlockReason, HoldState, IntegrationState, RunPath, RunState, Severity, TaskState};
use serde_json::{Map, Value, json};

use super::json::{cut, cut_opt, fnv1a, fold_all, hh_mm, label, route_text, shrink_strings, size};
use super::{PlannerPhase, RefreshState};
use crate::run::contract::sha7;
use crate::run::engine::schedule::{readers_busy, writers_busy};
use crate::run::model::{Run, Task};

/// The digest's cap, in bytes of compact JSON.
pub const DIGEST_MAX_BYTES: usize = 48 * 1024;
/// A task's `block.text` and a scout's `question`, in characters; `block.text` is cut
/// to [`BLOCK_TEXT_TRIMMED`] when the digest is over its cap.
pub const BLOCK_TEXT_MAX: usize = 500;
pub const BLOCK_TEXT_TRIMMED: usize = 200;
/// A task note's `text` (decision 42f).
pub const TASK_NOTE_MAX: usize = 400;
/// Task notes and edit-log entries shown, and what trimming leaves of each.
pub const TASK_NOTES_SHOWN: usize = 10;
pub const EDITS_SHOWN: usize = 10;
const LISTS_TRIMMED: usize = 3;
/// Wake notes left when trimming.
const NOTES_TRIMMED: usize = 5;
/// An attention line, a wake note, a task's `last` line, a planner's rejection or note,
/// an edit's error: engine lines that quote agents' texts, in characters.
const LINE_MAX: usize = 300;
/// The goal and a task title, in characters.
const GOAL_MAX: usize = 2000;
const TITLE_MAX: usize = 120;

/// The digest of `run` at `now`, trimmed to [`DIGEST_MAX_BYTES`].
pub fn digest(run: &Run, now: u64) -> Value {
    let mut digest = build(run, now, false);
    fold_all(&mut digest);
    trim(&mut digest, run);
    digest
}

/// Decision 16's fingerprint: FNV-1a 64 over the untrimmed digest with every counter
/// removed (`revision`, `now`, `spend`, `slots`, every task's `last`), and with what a
/// read itself changes removed too: the wake `notes` and the holds kept only because
/// they were decided since the last read. So a read, a tool call, tokens or time never
/// bump `digest_rev`; a state, a block, a verdict, a hold, a scout, a planner, a task
/// note, a message or an edit does.
pub fn fingerprint(run: &Run) -> u64 {
    let mut digest = build(run, 0, true);
    if let Some(map) = digest.as_object_mut() {
        for key in ["revision", "now", "spend", "slots", "notes"] {
            map.remove(key);
        }
        if let Some(Value::Array(tasks)) = map.get_mut("tasks") {
            for task in tasks.iter_mut().filter_map(Value::as_object_mut) {
                task.remove("last");
            }
        }
    }
    fnv1a(&serde_json::to_vec(&digest).unwrap_or_default())
}

/// Bumps `run.orch.digest_rev` when the fingerprint changed; true when it did. Called
/// by the reducer after every step that changed the run structurally.
pub fn note_change(run: &mut Run) -> bool {
    let fp = fingerprint(run);
    if fp == run.orch.digest_fp {
        return false;
    }
    run.orch.digest_fp = fp;
    run.orch.digest_rev += 1;
    true
}

/// The untrimmed digest. `for_fingerprint` keeps only the holds not yet approved.
fn build(run: &Run, now: u64, for_fingerprint: bool) -> Value {
    let orch = run.orch.orchestrator.as_ref();
    let state = run.state.label();
    json!({
        "revision": run.orch.digest_rev,
        "now": now,
        "run": {
            "id": run.id,
            "goal": cut(&run.goal, GOAL_MAX),
            "state": state,
            "path": run.path.map(|p| label(&p)),
            "base": format!("{}@{}", run.base_branch, sha7(&run.base_sha)),
            "approved_by": run.approved_by,
            "complete": matches!(run.state, RunState::Complete | RunState::Accepted),
            "halted_reason": cut_opt(run.halted_reason.as_deref(), LINE_MAX),
            "summary_written": orch.is_some_and(|o| o.summary.is_some()),
        },
        "gate": gate(run, for_fingerprint),
        "slots": {
            "writers": format!("{}/{}", writers_busy(run), run.limits.max_writers),
            "readers": format!("{}/{}", readers_busy(run), run.limits.max_readers),
        },
        "counts": counts(run),
        "tasks": run.tasks.iter().map(|t| task_entry(run, t)).collect::<Vec<_>>(),
        "omitted_tasks": 0,
        "scouts": run.orch.run_scouts.iter().map(|s| json!({
            "id": s.id,
            "state": s.state.label(),
            "question": cut(&s.question, BLOCK_TEXT_MAX),
            "failure": match &s.state {
                super::RunScoutState::Failed { reason } => Some(cut(reason, LINE_MAX)),
                _ => None,
            },
        })).collect::<Vec<_>>(),
        "planners": run.orch.epics.iter().map(|e| json!({
            "epic": e.epic,
            "state": match e.phase {
                PlannerPhase::Queued => "queued",
                PlannerPhase::Planning => "planning",
                PlannerPhase::Finished => "finished",
                PlannerPhase::Failed { .. } => "failed",
            },
            "tasks": run.tasks.iter().filter(|t| t.spec.epic.as_deref() == Some(&e.epic)).count(),
            "rejected": e.edits_rejected,
            "last_rejection": cut_opt(e.last_rejection.as_deref(), LINE_MAX),
            "note": cut_opt(e.note.as_deref(), LINE_MAX),
        })).collect::<Vec<_>>(),
        "integration": integration(run),
        "attention": crate::run::snapshot::attention(run).iter().map(|l| cut(l, LINE_MAX)).collect::<Vec<_>>(),
        "notes": orch.map(|o| o.notes.iter().map(|n| cut(n, LINE_MAX)).collect::<Vec<_>>()).unwrap_or_default(),
        "task_notes": task_notes(run),
        "edits": run.plan_edits.iter().rev().take(EDITS_SHOWN).map(|e| json!({
            "at": hh_mm(e.at),
            "source": e.source,
            "text": e.text,
            "accepted": e.accepted,
            "error": cut_opt(e.error.as_deref(), LINE_MAX),
            "recipients": e.recipients,
        })).collect::<Vec<_>>(),
        "spend": spend(run),
    })
}

/// `planning`, `awaiting_approval`, `approved` (with `at`) or `none` (a fast-path run
/// the orchestrator has not taken over), and the holds.
fn gate(run: &Run, for_fingerprint: bool) -> Value {
    let (state, at) = match run.state {
        RunState::Planning => ("planning", None),
        RunState::AwaitingApproval => ("awaiting_approval", None),
        _ if run.path == Some(RunPath::Fast) && run.orch.orchestrator.is_none() => ("none", None),
        _ => match run.approved_at {
            Some(at) => ("approved", Some(hh_mm(at))),
            None => ("none", None),
        },
    };
    let read_at = run.orch.digest_read_at;
    let holds: Vec<Value> = run
        .orch
        .gate_holds
        .iter()
        .filter(|h| {
            h.state != HoldState::Approved
                || (!for_fingerprint
                    && h.decided_at
                        .is_some_and(|d| read_at.is_none_or(|read| d >= read)))
        })
        .map(|h| json!({"id": h.id, "state": label(&h.state), "tasks": h.tasks.len()}))
        .collect();
    json!({"state": state, "at": at, "holds": holds})
}

/// The hold a task waits on: its hold, while that hold is not approved.
fn open_hold<'a>(run: &Run, task: &'a Task) -> Option<&'a str> {
    let id = task.orch.gate_hold.as_deref()?;
    let approved = run
        .orch
        .gate_holds
        .iter()
        .any(|h| h.id == id && h.state == HoldState::Approved);
    (!approved).then_some(id)
}

fn is_message_pause(task: &Task) -> bool {
    task.state == TaskState::Blocked
        && task
            .block
            .as_ref()
            .is_some_and(|b| b.reason == BlockReason::MessagePause)
}

fn counts(run: &Run) -> Value {
    let mut counts: Map<String, Value> = [
        "total",
        "merged",
        "reported",
        "working",
        "checking",
        "review",
        "merging",
        "blocked",
        "paused",
        "waiting",
        "held",
        "cancelled",
    ]
    .into_iter()
    .map(|k| (k.to_string(), json!(0)))
    .collect();
    let mut bump = |key: &str| {
        if let Some(Value::Number(n)) = counts.get_mut(key) {
            *n = (n.as_u64().unwrap_or(0) + 1).into();
        }
    };
    for task in &run.tasks {
        bump("total");
        bump(match task.state {
            TaskState::Merged => "merged",
            TaskState::Reported => "reported",
            TaskState::Working => "working",
            TaskState::Proof | TaskState::Check => "checking",
            TaskState::Review => "review",
            TaskState::MergeQueue => "merging",
            TaskState::Blocked if is_message_pause(task) => "paused",
            TaskState::Blocked => "blocked",
            TaskState::Pending | TaskState::Queued | TaskState::Preparing => "waiting",
            TaskState::Cancelled => "cancelled",
        });
        if open_hold(run, task).is_some() {
            bump("held");
        }
    }
    Value::Object(counts)
}

fn task_entry(run: &Run, t: &Task) -> Value {
    let undelivered = t.orch.messages.iter().filter(|m| !m.delivered).count();
    json!({
        "id": t.spec.id,
        "title": cut(&t.spec.title, TITLE_MAX),
        "epic": t.spec.epic,
        "kind": label(&t.spec.kind),
        "size": label(&t.size),
        "hub": t.hub,
        "state": t.state.label(),
        "hold": open_hold(run, t),
        "block": t.block.as_ref().map(|b| json!({
            "reason": label(&b.reason),
            "text": cut(&b.text, BLOCK_TEXT_MAX),
        })),
        "rung": t.rung,
        "deps": t.spec.deps,
        "route": route_text(&t.route),
        "review": review_text(t),
        "messages": {
            "count": t.orch.messages.len(),
            "undelivered": undelivered,
            "refresh": t.orch.refresh.as_ref().map(|r| match r {
                RefreshState::Due => "due",
                RefreshState::InFlight(_) => "in_flight",
            }),
        },
        "last": t.history.last().map(|e| cut(&format!("{} {}", hh_mm(e.at), e.text), LINE_MAX)),
    })
}

/// `r<round> <verdict>`, with its blocking findings counted: `r1 changes (1 critical)`.
fn review_text(task: &Task) -> Option<String> {
    let review = task.reviews.iter().rev().find(|r| r.verdict.is_some())?;
    let verdict = label(&review.verdict?);
    let findings = findings_text(&review.findings, false);
    Some(if findings.is_empty() {
        format!("r{} {verdict}", review.round)
    } else {
        format!("r{} {verdict} ({findings})", review.round)
    })
}

/// `1 critical, 2 important`: every severity with findings, or with `all` the critical
/// and important counts even when zero.
fn findings_text(findings: &[proto::Finding], all: bool) -> String {
    let mut parts = Vec::new();
    for severity in [Severity::Critical, Severity::Important, Severity::Minor] {
        let n = findings.iter().filter(|f| f.severity == severity).count();
        if n > 0 || (all && severity != Severity::Minor) {
            parts.push(format!("{n} {}", label(&severity)));
        }
    }
    parts.join(", ")
}

/// Each epic whose integration review has started: its state, round, latest review
/// task and that task's findings.
fn integration(run: &Run) -> Vec<Value> {
    run.orch
        .epics
        .iter()
        .filter(|e| e.integration_state != IntegrationState::NotYet)
        .map(|e| {
            let task = run
                .tasks
                .iter()
                .rev()
                .find(|t| t.orch.integration_of.as_deref() == Some(&e.epic));
            let findings = task
                .and_then(|t| t.reviews.iter().rev().find(|r| r.verdict.is_some()))
                .map(|r| findings_text(&r.findings, true));
            json!({
                "epic": e.epic,
                "state": label(&e.integration_state),
                "round": e.integration_rounds,
                "task": task.map(|t| t.id()),
                "findings": findings,
            })
        })
        .collect()
}

/// The last [`TASK_NOTES_SHOWN`] task notes across tasks, newest first.
fn task_notes(run: &Run) -> Vec<Value> {
    let mut notes: Vec<(&Task, &super::WorkerNote)> = run
        .tasks
        .iter()
        .flat_map(|t| t.orch.worker_notes.iter().map(move |n| (t, n)))
        .collect();
    // Stable: equal times keep plan order, reversed with the rest.
    notes.sort_by_key(|(_, n)| n.at);
    notes
        .iter()
        .rev()
        .take(TASK_NOTES_SHOWN)
        .map(|(t, n)| {
            json!({
                "task": t.id(),
                "kind": label(&n.kind),
                "text": cut(&n.text, TASK_NOTE_MAX),
                "at": hh_mm(n.at),
            })
        })
        .collect()
}

/// Billable tokens across the run (saturating) and every round's tool calls.
fn spend(run: &Run) -> Value {
    let usage = crate::run::snapshot::run_usage(run).total;
    let tokens = usage
        .input
        .saturating_add(usage.cache_write)
        .saturating_add(usage.output);
    let tool_calls: u64 = run
        .tasks
        .iter()
        .flat_map(|t| &t.rounds)
        .map(|r| u64::from(r.tool_calls))
        .sum();
    json!({"tokens": tokens, "tool_calls": tool_calls})
}

/// Decision 16's trimming past the cap, in order: finished tasks dropped oldest first
/// (counted in `omitted_tasks`), `edits` cut to 3, `task_notes` to 3, `block.text` to
/// 200 characters (and the attention lines with it), `notes` to 5. Beyond the
/// Interfaces, so the cap always holds: unfinished tasks dropped from the end of the
/// plan (counted too), then every string cut to 120 characters, then the other lists
/// cut to 10 entries.
fn trim(digest: &mut Value, run: &Run) {
    let fits = |d: &Value| size(d) <= DIGEST_MAX_BYTES;
    if fits(digest) {
        return;
    }
    // Finished tasks, oldest first: by their newest history entry, then plan order.
    let mut finished: Vec<(u64, usize)> = run
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.state.is_finished())
        .map(|(i, t)| (t.history.last().map_or(0, |e| e.at), i))
        .collect();
    finished.sort();
    let order: Vec<&str> = finished.iter().map(|(_, i)| run.tasks[*i].id()).collect();
    for id in order {
        if fits(digest) {
            return;
        }
        drop_task(digest, id);
    }
    for key in ["edits", "task_notes"] {
        if fits(digest) {
            return;
        }
        truncate(digest, key, LISTS_TRIMMED);
    }
    if fits(digest) {
        return;
    }
    if let Some(Value::Array(tasks)) = digest.get_mut("tasks") {
        for text in tasks
            .iter_mut()
            .filter_map(|t| t.pointer_mut("/block/text"))
        {
            if let Value::String(s) = text {
                *s = cut(s, BLOCK_TEXT_TRIMMED);
            }
        }
    }
    if let Some(Value::Array(lines)) = digest.get_mut("attention") {
        for line in lines.iter_mut() {
            if let Value::String(s) = line {
                *s = cut(s, BLOCK_TEXT_TRIMMED);
            }
        }
    }
    if fits(digest) {
        return;
    }
    truncate(digest, "notes", NOTES_TRIMMED);
    while !fits(digest) {
        let last = match digest.get("tasks") {
            Some(Value::Array(tasks)) => tasks.last().and_then(|t| t["id"].as_str()),
            _ => None,
        };
        match last.map(str::to_string) {
            Some(id) => drop_task(digest, &id),
            None => break,
        }
    }
    if fits(digest) {
        return;
    }
    shrink_strings(digest, 120);
    for key in ["attention", "scouts", "planners", "integration", "notes"] {
        if fits(digest) {
            return;
        }
        truncate(digest, key, 10);
    }
}

/// Removes task `id` from `tasks` and counts it in `omitted_tasks`.
fn drop_task(digest: &mut Value, id: &str) {
    if let Some(Value::Array(tasks)) = digest.get_mut("tasks") {
        tasks.retain(|t| t["id"].as_str() != Some(id));
    }
    if let Some(n) = digest.get_mut("omitted_tasks") {
        *n = json!(n.as_u64().unwrap_or(0) + 1);
    }
}

fn truncate(digest: &mut Value, key: &str, keep: usize) {
    if let Some(Value::Array(items)) = digest.get_mut(key) {
        items.truncate(keep);
    }
}

#[cfg(test)]
#[path = "digest_tests.rs"]
mod tests;
