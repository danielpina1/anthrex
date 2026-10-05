//! `run_status`'s digest (decision 16, Interfaces "The digest"): one compact JSON object
//! of what the orchestrator must see, at most [`DIGEST_MAX_BYTES`], and the
//! fingerprint whose change bumps `Run.orch.digest_rev`. Pure: it reads the model, not
//! the snapshot, so its texts are the model's (cut as the Interfaces say).

use proto::{BlockReason, HoldState, IntegrationState, RunPath, RunState, Severity, TaskState};
use serde_json::{Map, Value, json};

use super::json::{cut, cut_opt, fnv1a, fold_all, hh_mm, label, route_text};
use super::{PlannerPhase, RefreshState};
use crate::run::contract::sha7;
use crate::run::delivery::digest as delivery;
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
/// An edit's recipients shown; the rest are counted.
pub const RECIPIENTS_SHOWN: usize = 20;
const LISTS_TRIMMED: usize = 3;
/// Wake notes left when trimming.
const NOTES_TRIMMED: usize = 5;
/// An attention line, a wake note, a task's `last` line, a planner's rejection or note,
/// an edit's error: engine lines that quote agents' texts, in characters.
const LINE_MAX: usize = 300;
/// The goal and a task title, in characters.
const GOAL_MAX: usize = 2000;
const TITLE_MAX: usize = 120;

#[path = "digest_trim.rs"]
mod trim;
use trim::trim;

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
            // Milestone 9.3 decision 28: the current round, and the stage its tasks start at.
            "round": run.round(),
            "first_stage": run.current_round().map_or(1, |r| r.first_stage),
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
        // Milestone 9.1 decision 58: the snapshot's stages, every string cut as the
        // other engine lines are (ruling C-24).
        "stages": stages(run),
        "delivery": delivery::block(run, delivery::Shape::digest(for_fingerprint)),
        "attention": crate::run::snapshot::attention(run, now).iter().map(|l| cut(l, LINE_MAX)).collect::<Vec<_>>(),
        "notes": orch.map(|o| o.notes.iter().map(|n| cut(n, LINE_MAX)).collect::<Vec<_>>()).unwrap_or_default(),
        "task_notes": task_notes(run),
        "edits": run.plan_edits.iter().rev().take(EDITS_SHOWN).map(edit_entry).collect::<Vec<_>>(),
        "spend": spend(run),
    })
}

/// The snapshot's `StageInfo`s with every string (branches, heads, test names, the
/// note) cut to [`LINE_MAX`] characters; each list already holds at most 20 names.
fn stages(run: &Run) -> Value {
    let mut stages = json!(crate::run::snapshot_stages::stage_infos(run));
    super::json::shrink_strings(&mut stages, LINE_MAX);
    stages
}

/// An edit-log entry, its `recipients` cut to [`RECIPIENTS_SHOWN`] with the rest
/// counted in `recipients_omitted` (present only then; second review, Minor 3).
fn edit_entry(e: &crate::run::edit_log::PlanEditRecord) -> Value {
    let mut entry = json!({
        "at": hh_mm(e.at),
        "source": e.source,
        "text": e.text,
        "accepted": e.accepted,
        "error": cut_opt(e.error.as_deref(), LINE_MAX),
        "recipients": e.recipients.iter().take(RECIPIENTS_SHOWN).collect::<Vec<_>>(),
    });
    let omitted = e.recipients.len().saturating_sub(RECIPIENTS_SHOWN);
    if omitted > 0 {
        entry["recipients_omitted"] = json!(omitted);
    }
    entry
}

/// `planning`, `awaiting_approval`, `approved` (with `at`) or `none` (a fast-path run
/// the orchestrator has not taken over), and the holds.
fn gate(run: &Run, for_fingerprint: bool) -> Value {
    let (state, at) = match run.state {
        // Milestone 9.6 decision 4 (M-2): the design phases, by name.
        RunState::Brainstorming => ("brainstorming", None),
        RunState::Specifying => ("specifying", None),
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
    let mut gate = json!({"state": state, "at": at, "holds": holds});
    // Milestone 9.6: the document gate the run waits at (kind, version, the user's
    // note while the orchestrator revises, disputed findings).
    if let Some(doc) = crate::run::engine::design_gate::digest(run) {
        gate["doc_gate"] = doc;
    }
    // Task M9.6.10: the spec review the orchestrator's next `ready` submit answers.
    if let Some(review) = crate::run::engine::design::review::digest(run) {
        gate["spec_review"] = review;
    }
    // Task M9.6.11: the plan review the orchestrator's next submit answers.
    if let Some(review) = crate::run::engine::design::plan::digest(run) {
        gate["plan_review"] = review;
    }
    gate
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
        "stage": t.stage(),
        "origin": label(&t.origin),
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
        "race": race_text(t),
        "pair": pair_text(t),
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

/// Milestone 9.5 decision 32: `a claude working · b codex review` while the lanes race,
/// `won by <lane>` once one became the task.
pub(super) fn race_text(task: &Task) -> Option<String> {
    let race = task.race.as_ref()?;
    if let Some(winner) = race.winner {
        return Some(format!("won by {}", winner.label()));
    }
    let lane = |l: &crate::run::model::Lane| {
        let runtime = l.route.runtime.label();
        format!("{} {runtime} {}", l.lane.label(), label(&l.state))
    };
    Some(race.lanes.iter().map(lane).collect::<Vec<_>>().join(" · "))
}

/// Decision 32: `writing the test`, then `implementing, red <sha7>`.
pub(super) fn pair_text(task: &Task) -> Option<String> {
    let pair = task.pair.as_ref()?;
    Some(match (pair.phase, &pair.red) {
        (proto::PairPhase::Writing, _) => "writing the test".to_string(),
        (proto::PairPhase::Implementing, Some(red)) => format!("implementing, red {}", sha7(red)),
        (proto::PairPhase::Implementing, None) => "implementing".to_string(),
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
    // Equal times: the note added last is the newest (`seq`, review fix M-2); notes
    // with no `seq` keep plan order, reversed with the rest (the sort is stable).
    notes.sort_by_key(|(_, n)| (n.at, n.seq));
    notes
        .iter()
        .rev()
        .take(TASK_NOTES_SHOWN)
        .map(|(t, n)| {
            let mut entry = json!({
                "task": t.id(),
                "kind": label(&n.kind),
                "text": cut(&n.text, TASK_NOTE_MAX),
                "at": hh_mm(n.at),
            });
            // Milestone 9.5 ruling RR-4: a racer's note says its lane.
            if let Some(lane) = n.lane {
                entry["lane"] = json!(format!("lane {}", lane.label()));
            }
            entry
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

#[cfg(test)]
#[path = "digest_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests_delivery.rs"]
mod tests_delivery;

#[cfg(test)]
#[path = "digest_tests_rounds.rs"]
mod tests_rounds;

#[cfg(test)]
#[path = "digest_tests_patterns.rs"]
mod tests_patterns;
