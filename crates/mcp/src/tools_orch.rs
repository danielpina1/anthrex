//! Milestone 9's tools (decision 15, Interfaces "MCP"): the orchestrator's six (seven
//! since milestone 9.3's `start_goal`), the sub-planner's two and the worker's
//! `task_note`. Every schema is a closed object at
//! every level. The limits are what the model sees; the daemon parses every call again
//! (`daemon::run::orch::tools::parse_call`) and refuses what breaks them. No tool here
//! approves a plan or a design document, or accepts or merges anything; edit_plan's
//! override sends a task to the engine's merge queue, which still checks it.

use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};

use crate::tools::{closed, one_of, text};

pub const GET_CONTEXT: &str = "get_context";
pub const SPAWN_SCOUT: &str = "spawn_scout";
pub const SPAWN_SUBPLANNER: &str = "spawn_subplanner";
pub const EDIT_PLAN: &str = "edit_plan";
pub const RUN_STATUS: &str = "run_status";
pub const TASK_RESULT: &str = "task_result";
pub const SUBMIT_EPIC: &str = "submit_epic";
pub const TASK_NOTE: &str = "task_note";
pub const START_GOAL: &str = "start_goal";
pub const ASK_USER: &str = "ask_user";

/// The most `run_status` waits, in seconds (decision 16).
pub const RUN_STATUS_MAX_WAIT: u64 = 50;

/// `tools_for(Orchestrator)`, in decision 15's order, then milestone 9.3's `start_goal`
/// (decision 29), then milestone 9.6's three (`tools_design.rs`), then milestone 9.9's
/// `ask_user`.
pub fn orchestrator_tools() -> Vec<Tool> {
    let mut tools = vec![
        get_context(),
        spawn_scout(),
        spawn_subplanner(),
        edit_plan(),
        run_status(),
        task_result(),
        start_goal(),
    ];
    tools.extend(crate::tools_design::orchestrator_design_tools());
    tools.push(ask_user());
    tools
}

/// `tools_for(Planner)`, unchanged by milestone 9.3: a sub-planner sees neither
/// `start_goal` nor `edit_plan`'s `iterate` (KG §4).
pub fn planner_tools() -> Vec<Tool> {
    vec![get_context(), submit_epic()]
}

/// The worker's third tool (decision 42f).
pub fn task_note() -> Tool {
    Tool::new(
        TASK_NOTE,
        "Report a discovery, risk or progress without blocking the task.",
        closed(
            json!({
                "kind": one_of(&["discovery", "risk", "progress"]),
                "text": text(4000),
            }),
            &["kind", "text"],
        ),
    )
}

/// Milestone 9.9 decision 15: the orchestrator's one question to the user. The daemon
/// parses the call again with the same bounds (`orch/tools.rs`).
fn ask_user() -> Tool {
    Tool::new(
        ASK_USER,
        "Ask the user one question when only they can decide, with up to nine short options \
         and the context they need. Returns at once; their choice arrives as a message. \
         Never ask in chat instead.",
        closed(
            json!({
                "question": text(500),
                "options": array(text(200), None, 9),
                "context": text(4000),
            }),
            &["question"],
        ),
    )
}

fn get_context() -> Tool {
    Tool::new(
        GET_CONTEXT,
        "Read the run's context: the repository profile, the role table (which model each \
         size runs on), the limits, scout reports, epics and the plan so far.",
        closed(json!({"scouts": array(text(48), None, 50)}), &[]),
    )
}

fn spawn_scout() -> Tool {
    Tool::new(
        SPAWN_SCOUT,
        "Start a read-only scout on one area with one question. Returns at once; its report \
         appears in run_status and get_context.",
        closed(
            json!({
                "id": pattern("^[a-z0-9][a-z0-9-]{0,31}$"),
                "question": text(2000),
                "area": array(text(300), Some(1), 20),
                "web": boolean(),
            }),
            &["id", "question", "area"],
        ),
    )
}

fn spawn_subplanner() -> Tool {
    Tool::new(
        SPAWN_SUBPLANNER,
        "Start a sub-planner for one epic with its own area, or a fresh one to re-plan an \
         existing epic. Returns at once.",
        closed(
            json!({
                "epic": pattern("^[a-z0-9][a-z0-9-]{0,10}$"),
                "title": text(80),
                "area": array(text(300), Some(1), 20),
                "brief": text(8000),
                "scout_refs": array(text(48), None, 20),
                // Milestone 9.6 ruling T11-1: the requirement ids the epic owns.
                "covers": crate::tools_design::covers(),
            }),
            &["epic", "title", "area", "brief"],
        ),
    )
}

/// Milestone 9.3 decisions 29 and 30: `iterate` starts a round. Nothing is required: the
/// daemon's `parse_call` reads a missing `edits` as an empty batch (task M9.3.7's fix
/// round 1), so this plain schema (task M9.1.11: no `oneOf`, `anyOf` or `allOf`) is
/// exact. That `iterate` comes alone is the engine's check, so the description says it.
fn edit_plan() -> Tool {
    Tool::new(
        EDIT_PLAN,
        "Apply plan edits as one batch. Set submit to open the plan gate. Add a summary for \
         the user when the run is complete. Set iterate, with no edits and nothing else, to \
         start a round the user asked for. retry, override, resume_run, approve_hold and \
         accept_red each come alone, with a reason. approve_hold never approves a promoted \
         run's promotion hold: that is the user's plan approval. Returns at once.",
        closed(
            json!({
                "edits": array(object(plan_edit()), None, 60),
                "submit": boolean(),
                "summary": text(8000),
                "iterate": text(proto::GOAL_MAX_CHARS as u64),
                // Milestone 9.6 decision 20: the answers to the plan review's findings.
                "responses": crate::tools_design::responses(),
            }),
            &[],
        ),
    )
}

/// Milestone 9.3 decision 29: a next goal on this orchestrator's chain (KG §3.3).
fn start_goal() -> Tool {
    Tool::new(
        START_GOAL,
        "Start a new goal on this orchestrator when the user gives you one; only while your \
         last run has ended.",
        closed(
            json!({"goal": text(proto::GOAL_MAX_CHARS as u64)}),
            &["goal"],
        ),
    )
}

fn run_status() -> Tool {
    Tool::new(
        RUN_STATUS,
        "Read the run digest. With since and wait_secs, wait up to wait_secs seconds (at most \
         50) for it to change.",
        closed(
            json!({
                "since": {"type": "integer", "minimum": 0},
                "wait_secs": {"type": "integer", "minimum": 0, "maximum": RUN_STATUS_MAX_WAIT},
            }),
            &[],
        ),
    )
}

fn task_result() -> Tool {
    Tool::new(
        TASK_RESULT,
        "Read everything about one task: brief, commits, diff size, checks, proofs, reviews, \
         agent rounds and any report.",
        closed(json!({"task_id": text(16)}), &["task_id"]),
    )
}

fn submit_epic() -> Tool {
    Tool::new(
        SUBMIT_EPIC,
        "Submit your epic's tasks as one batch of plan edits. If it returns errors, fix them \
         and call it again. When it is accepted you are done.",
        closed(
            json!({
                "edits": array(object(plan_edit()), Some(1), 60),
                "note": text(2000),
            }),
            &["edits"],
        ),
    )
}

/// M8a's `PlanEdit` as the model writes it; `stage` is `amend_task`'s (M9.1 decision 43),
/// `pr`, `thread` and `body` are `reply_comment`'s (M9.2 decision 30). Which keys each `op` needs is the daemon's
/// serde shape; this schema only bounds them. `race` and `pair` are `amend_task`'s (M9.5
/// decision 32).
fn plan_edit() -> JsonObject {
    closed(
        json!({
            "op": one_of(&[
                "add_task", "split_task", "cancel_task", "amend_task", "add_dep", "answer",
                "pause", "resume", "finish", "message", "refresh", "reply_comment",
                "retry", "override", "resume_run", "approve_hold", "accept_red",
            ]),
            "task": object(plan_task()),
            "task_id": text(16),
            "into": array(object(plan_task()), Some(1), 12),
            "brief": text(8000),
            "acceptance": array(text(500), Some(1), 20),
            "route": object(route()),
            "test_mode": test_mode(),
            "test_mode_reason": text(300),
            "priority": integer(),
            "size": size(),
            "deps": array(text(16), None, 20),
            "dep": text(16),
            "text": text(8000),
            "to": {"oneOf": [
                array(text(16), Some(1), 20),
                pattern("^(running|stage:[0-9]{1,4})$"),
            ]},
            "kind": one_of(&["info", "change", "stop_and_wait"]),
            "stage": stage(),
            "pr": {"type": "integer", "minimum": 1},
            "thread": text(64),
            "body": text(4000),
            "race": boolean(),
            "pair": boolean(),
            // Task M9.9.10: the five resolving ops' reason and hold.
            "reason": text(500),
            "hold": text(64),
        }),
        &["op"],
    )
}

/// A task as `add_task` and `split_task` take it. `budget` is deliberately absent
/// (decision 23.1); `stage`, `atomic` and `atomic_reason` are M9.1 decision 43's,
/// `addresses` (review thread refs, `<pr>:<key>`) M9.2 decision 31's, `race` and `pair` M9.5's,
/// `covers` M9.6's.
fn plan_task() -> JsonObject {
    closed(
        json!({
            "id": pattern("^[a-z0-9][a-z0-9-]{0,15}$"),
            "title": text(120),
            "epic": text(11),
            "kind": one_of(&["code", "docs", "research", "review"]),
            "size": size(),
            "interface_change": boolean(),
            "test_mode": test_mode(),
            "test_mode_reason": text(300),
            "owns": array(text(300), None, 20),
            "deps": array(text(16), None, 20),
            "priority": integer(),
            "brief": text(8000),
            "acceptance": array(text(500), Some(1), 20),
            "test_to_write": text(300),
            "scout_refs": array(text(48), None, 20),
            "route": object(route()),
            "review_target": text(200),
            "stage": stage(),
            "atomic": boolean(),
            "atomic_reason": text(300),
            "addresses": array(text(64), None, 20),
            "race": boolean(),
            "pair": boolean(),
            // Milestone 9.6 decision 17: the spec requirements the task delivers.
            "covers": crate::tools_design::covers(),
        }),
        &["id", "title", "size", "owns", "brief", "acceptance"],
    )
}

/// Milestone 9.8 decision 32: kept so an older orchestrator's call is not rejected, its
/// four old keys with their values unchecked; the engine ignores it (decision 31).
fn route() -> JsonObject {
    let mut route = closed(
        json!({
            "runtime": {"type": "string"},
            "model": {"type": "string"},
            "strength": {"type": "string"},
            "effort": {"type": "string"},
        }),
        &[],
    );
    let ignored = "Ignored: models come from the role table.";
    route.insert("description".into(), json!(ignored));
    route
}

fn size() -> Value {
    one_of(&["S", "M", "L"])
}

fn test_mode() -> Value {
    one_of(&["tdd", "check", "none"])
}

/// Milestone 9.1 decision 43: a task's stage, a plain integer (M9.1.11 keeps the
/// schemas free of `oneOf`, `anyOf` and `allOf`).
fn stage() -> Value {
    json!({"type": "integer", "minimum": 1, "maximum": proto::STAGES_MAX})
}

fn boolean() -> Value {
    json!({"type": "boolean"})
}

fn integer() -> Value {
    json!({"type": "integer"})
}

fn pattern(re: &str) -> Value {
    json!({"type": "string", "pattern": re})
}

fn object(o: JsonObject) -> Value {
    Value::Object(o)
}

/// An array of `items`, `min` to `max` of them (no lower bound when `min` is `None`).
fn array(items: Value, min: Option<u64>, max: u64) -> Value {
    match min {
        Some(min) => json!({"type": "array", "minItems": min, "maxItems": max, "items": items}),
        None => json!({"type": "array", "maxItems": max, "items": items}),
    }
}

#[cfg(test)]
#[path = "tools_orch_tests.rs"]
mod tests;
