//! Milestone 9 task M9.7: a planned run and its orchestrator (decisions 6, 26): its
//! start, the route, the plan gate's refusals, the caller check, and what no
//! orchestrator tool can do (approve). `edit_plan` and `submit` are in `orch_edit.rs`,
//! restore and resume in `orch_restore.rs`, the holds in `gate_holds.rs`, the
//! promotion in `promote.rs`.

use std::collections::BTreeMap;

use proto::{
    AgentRole, DeciderSource, Effort, OrchestratorChoice, RunPath, RunState, Runtime, Scale,
    TaskKind, ToolCall, TriageInfo,
};
use serde_json::{Value, json};

use super::dispatch::replies;
use super::fixture::*;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult, OrchEvent};
use crate::run::orch::launch::orchestrator_route;
use crate::run::orch::make_planned;

/// The orchestrator's window in these tests.
pub(super) const ORCH: u32 = 90;

pub(super) fn triage(path: RunPath) -> TriageInfo {
    TriageInfo {
        kinds: vec![TaskKind::Code],
        scale: Scale::Plan,
        path,
        reason: "several modules".into(),
        source: DeciderSource::Decider,
        fallback_reason: None,
        at: 1_000,
    }
}

/// A planned run (decision 26), started; its run branch and orchestrator not yet made.
pub(super) fn planned(yes: bool) -> Fixture {
    planned_on(yes, None)
}

/// [`planned`], its orchestrator on `choice`'s runtime (milestone 9.5 ruling T5a-2).
pub(super) fn planned_on(yes: bool, choice: Option<Runtime>) -> Fixture {
    let text = plan_with(PROFILE, &[task("t0", "S", "auth", "")]);
    let mut fx = Fixture::new(&text);
    // Decision 26: the driver builds a planned run from a plan with no task.
    let mut plan = crate::run::plan::parse_plan(&text).unwrap();
    plan.tasks.clear();
    let mut run = crate::run::plan::build_run(
        plan,
        preflight(),
        crate::run::plan::BuildContext {
            tuning: Default::default(),
            id: RUN_ID.to_string(),
            wt_dir: WT.into(),
            data_dir: format!("/tmp/data/runs/{RUN_ID}").into(),
            config: &fx.config,
            models: crate::run::model_roles::RunModels::resolve(&fx.config.roles, None),
            models_log: Vec::new(),
            testing: &config::Testing::default(),
            now: 1_000,
            yes: false,
            delivery: &config::Delivery::default(),
        },
    )
    .unwrap_or_else(|e| panic!("an empty plan builds: {e:?}"));
    let choice = choice.map(|runtime| OrchestratorChoice {
        runtime,
        model: None,
        effort: None,
    });
    let resolved = orchestrator_route(choice.as_ref(), run.limits.models());
    make_planned(
        &mut run,
        Some(triage(RunPath::Plan)),
        resolved,
        yes,
        BTreeMap::new(),
    );
    let reply = fx.reply();
    fx.next(EventKind::Start {
        reply,
        run: Box::new(run),
    });
    fx
}

/// [`planned`], with the run branch made and the orchestrator in window [`ORCH`], its
/// first turn delivered (milestone 9.5 decision 38: its anthrex server announced its
/// tools, and the driver pasted the first prompt).
pub(super) fn launched(yes: bool) -> Fixture {
    let mut fx = launched_waiting(yes);
    mcp_ready(&mut fx, ORCH);
    first_turn_woken(&mut fx);
    fx
}

/// [`launched`] before its first turn: the window is up, its first prompt not pasted.
pub(super) fn launched_waiting(yes: bool) -> Fixture {
    let mut fx = planned(yes);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let (op, _) = fx.op("CreateOrchestrator");
    fx.done(
        op,
        OpResult::Window {
            window_id: ORCH,
            pid: None,
        },
    );
    fx
}

/// Milestone 9.5 decision 38: window `window`'s anthrex server answered its first
/// `tools/list` (the driver's caller check passed).
pub(super) fn mcp_ready(fx: &mut Fixture, window: u32) -> Vec<Effect> {
    fx.next(EventKind::Orch(OrchEvent::McpReady {
        run_id: RUN_ID.into(),
        window_id: window,
    }))
}

/// The driver pasted the first turn.
pub(super) fn first_turn_woken(fx: &mut Fixture) -> Vec<Effect> {
    fx.next(EventKind::Orch(OrchEvent::OrchestratorWoken {
        run_id: RUN_ID.into(),
        digest_revision: 0,
        notes_seq: 0,
        request: None,
        first_turn: true,
    }))
}

/// A tool call of the orchestrator's role from `window`.
pub(super) fn orch_tool(fx: &mut Fixture, window: u32, tool: &str, args: Value) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Orch(OrchEvent::Tool {
        reply,
        call: ToolCall {
            run_id: RUN_ID.into(),
            task_id: None,
            role: AgentRole::Orchestrator,
            window_id: window,
            tool: tool.into(),
            args,
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
        },
        refusals: Vec::new(),
    }))
}

pub(super) fn edit_plan(fx: &mut Fixture, args: Value) -> Vec<Effect> {
    orch_tool(fx, ORCH, "edit_plan", args)
}

/// An `add_task` edit of an S task owning `crates/<module>/**`.
pub(super) fn add(id: &str, module: &str) -> Value {
    json!({"op": "add_task", "task": {
        "id": id, "title": format!("Title {id}"), "size": "S",
        "owns": [format!("crates/{module}/**")],
        "brief": format!("Brief {id}"), "acceptance": [format!("Accept {id}")]
    }})
}

/// The one reply's JSON, and whether it was a success.
pub(super) fn answer(effects: &[Effect]) -> (bool, Value) {
    let replies = replies(effects);
    assert_eq!(replies.len(), 1, "{replies:?}");
    match &replies[0] {
        Ok(text) => (true, serde_json::from_str(text).unwrap()),
        Err(text) => (false, serde_json::from_str(text).unwrap()),
    }
}

pub(super) fn error(effects: &[Effect]) -> String {
    let (ok, value) = answer(effects);
    assert!(!ok, "{value}");
    value["error"].as_str().unwrap_or_default().to_string()
}

fn ops_named(effects: &[Effect]) -> Vec<&'static str> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Op { kind, .. } => Some(op_name(kind)),
            _ => None,
        })
        .collect()
}

#[test]
fn planned_run_starts_in_planning_and_creates_branch_then_orchestrator() {
    let mut fx = planned(false);
    let run = fx.run();
    assert_eq!(run.state, RunState::Planning);
    assert!(run.tasks.is_empty());
    assert_eq!(run.path, Some(RunPath::Plan));
    assert_eq!(
        ops_named(&fx.log),
        vec!["CreateRunBranch", "CreateOrchestrator"]
    );
    let (op, kind) = fx.op("CreateOrchestrator");
    let OpKind::CreateOrchestrator {
        spec,
        role,
        project,
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(
        spec.name.as_deref(),
        Some(format!("{H4}/orchestrator").as_str())
    );
    assert_eq!(spec.cwd, run.root);
    assert_eq!(spec.worktree_branch, None);
    assert_eq!(spec.runtime, Runtime::Claude);
    assert_eq!(spec.model.as_deref(), Some("claude-opus-5-5"));
    // Milestone 9.5 decision 38: the first prompt is pasted once the window is ready,
    // never passed at launch.
    assert_eq!(spec.initial_prompt, None);
    let first = fx
        .run()
        .orch
        .orchestrator
        .as_ref()
        .unwrap()
        .first_prompt
        .clone();
    assert!(first.starts_with(&format!(
        "[anthrex] You are the orchestrator of run {RUN_ID}"
    )));
    assert_eq!(project, run.project);
    assert_eq!(role.run_ref.role, AgentRole::Orchestrator);
    assert_eq!(
        (role.run_ref.task_id.clone(), role.run_ref.session),
        (None, 1)
    );
    assert_eq!(role.mcp.role, AgentRole::Orchestrator);
    assert_eq!(role.effort, Effort::HIGH);
    let o = run.orch.orchestrator.as_ref().unwrap();
    assert_eq!((o.launch_op, o.live, o.window_id), (Some(op), false, None));
    assert_eq!(o.first_prompt, first);

    // The run branch comes back: still no task, no pre-warm, no gate.
    let (branch, _) = fx.op("CreateRunBranch");
    let effects = fx.done(branch, OpResult::Worktree { head: BASE.into() });
    assert!(ops_named(&effects).is_empty(), "{effects:?}");
    assert_eq!(fx.run().state, RunState::Planning);
    let windows = fx.run().windows_created;
    fx.done(
        op,
        OpResult::Window {
            window_id: ORCH,
            pid: None,
        },
    );
    let o = fx.run().orch.orchestrator.clone().unwrap();
    assert_eq!((o.window_id, o.live, o.launch_op), (Some(ORCH), true, None));
    assert_eq!(
        fx.run().windows_created,
        windows + 1,
        "it counts toward max_windows"
    );
    // A planning run never completes.
    fx.tick();
    assert_eq!(fx.run().state, RunState::Planning);
}

#[test]
fn approve_while_planning_is_refused() {
    let mut fx = launched(false);
    let before = fx.run().clone();
    assert_eq!(
        replies(&fx.approve()),
        vec![Err(format!(
            "run {RUN_ID} is still being planned; approve it when the orchestrator has submitted the plan"
        ))]
    );
    assert_eq!(*fx.run(), before);
}

fn reject(fx: &mut Fixture) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    })
}

#[test]
fn reject_while_planning_discards() {
    let mut fx = launched(false);
    let effects = reject(&mut fx);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!("run {RUN_ID} rejected; discarding it"))]
    );
    assert_eq!(fx.ops("Discard").len(), 1);
}

#[test]
fn rejected_plan_is_seen_by_run_status() {
    let mut fx = launched(false);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    reject(&mut fx);
    let (op, _) = fx.op("Discard");
    fx.done(
        op,
        OpResult::Finished {
            outcome: "discarded".into(),
            kept_branches: Vec::new(),
        },
    );
    let digest = crate::run::orch::digest::digest(fx.run(), fx.now);
    assert_eq!(digest["run"]["state"], "discarded");
    let effects = edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]}));
    assert_eq!(error(&effects), format!("run {RUN_ID} is discarded"));
}

#[test]
fn orchestrator_calls_from_another_window_are_refused() {
    let mut fx = launched(false);
    let before = fx.run().clone();
    let effects = orch_tool(
        &mut fx,
        ORCH + 1,
        "edit_plan",
        json!({"edits": [add("t1", "auth")]}),
    );
    assert_eq!(
        error(&effects),
        format!("this window is not the orchestrator of run {RUN_ID}")
    );
    assert_eq!(*fx.run(), before);
    // Before its window exists, no window is the orchestrator.
    let mut early = planned(false);
    let effects = orch_tool(&mut early, ORCH, "edit_plan", json!({"edits": []}));
    assert_eq!(
        error(&effects),
        format!("this window is not the orchestrator of run {RUN_ID}")
    );
}

/// Design rule: no model approves the plan. Milestone 9.9 gave the orchestrator
/// `override` and `approve_hold`, but only on a running run (the user path's
/// preconditions): at the plan gate both reach the engine and are refused, `approve`
/// and `accept` are still no ops, and no tool of that name exists.
#[test]
fn orchestrator_tools_cannot_approve_a_plan() {
    let mut fx = launched(false);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    let tasks = fx.run().tasks.clone();
    for edit in [json!({"op": "approve"}), json!({"op": "accept"})] {
        let effects = edit_plan(&mut fx, json!({"edits": [edit]}));
        assert!(
            error(&effects).starts_with("invalid arguments: edits[0]: unknown variant"),
            "{effects:?}"
        );
    }
    for edit in [
        json!({"op": "override", "task_id": "t1", "reason": "fine"}),
        json!({"op": "approve_hold", "hold": "promotion", "reason": "fine"}),
    ] {
        let effects = edit_plan(&mut fx, json!({"edits": [edit]}));
        let text = error(&effects);
        assert!(!text.starts_with("invalid arguments"), "it parses: {text}");
        assert!(text.contains(RUN_ID), "the engine's refusal: {text}");
    }
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert!(fx.run().orch.handled.is_empty());
    assert_eq!(fx.run().approved_by, None);
    assert_eq!(fx.run().tasks, tasks);
    for tool in ["approve", "approve_hold", "accept", "override"] {
        let effects = orch_tool(&mut fx, ORCH, tool, json!({"hold": "promotion"}));
        assert_eq!(
            error(&effects),
            format!("tool {tool} is not available to the orchestrator role")
        );
    }
    // Submitting again at the gate approves nothing.
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert_eq!(fx.run().approved_by, None);
}
