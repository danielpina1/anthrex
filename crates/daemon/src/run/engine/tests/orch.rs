//! Milestone 9 task M9.7: a planned run and its orchestrator (decisions 6, 26): its
//! start, the route, the plan gate's refusals, the caller check, and what no
//! orchestrator tool can do (approve). `edit_plan` and `submit` are in `orch_edit.rs`,
//! restore and resume in `orch_restore.rs`, the holds in `gate_holds.rs`, the
//! promotion in `promote.rs`.

use std::collections::BTreeMap;

use proto::{
    AgentRole, DeciderSource, Effort, ModelEntry, OrchestratorChoice, RoutingCandidate, RunPath,
    RunState, Runtime, Scale, Strength, TaskKind, ToolCall, TriageInfo,
};
use serde_json::{Value, json};

use super::dispatch::replies;
use super::fixture::*;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult, OrchEvent};
use crate::run::orch::launch::resolve_orchestrator;
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
    let text = plan_with(PROFILE, &[task("t0", "S", "auth", "")]);
    let mut fx = Fixture::new(&text);
    // Decision 26: the driver builds a planned run from a plan with no task.
    let mut plan = crate::run::plan::parse_plan(&text).unwrap();
    plan.tasks.clear();
    let mut run = crate::run::plan::build_run(
        plan,
        preflight(),
        crate::run::plan::BuildContext {
            id: RUN_ID.to_string(),
            wt_dir: WT.into(),
            data_dir: format!("/tmp/data/runs/{RUN_ID}").into(),
            config: &fx.config,
            testing: &config::Testing::default(),
            now: 1_000,
            yes: false,
            delivery: &config::Delivery::default(),
        },
    )
    .unwrap_or_else(|e| panic!("an empty plan builds: {e:?}"));
    let agent = config::AgentConfig::default();
    let resolved =
        resolve_orchestrator(None, &agent, run.limits.default_runtime, &run.roster).unwrap();
    make_planned(
        &mut run,
        triage(RunPath::Plan),
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

/// [`planned`], with the run branch made and the orchestrator in window [`ORCH`].
pub(super) fn launched(yes: bool) -> Fixture {
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
    let first = spec.initial_prompt.clone().unwrap();
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
    assert_eq!(role.effort, Effort::High);
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

fn roster() -> Vec<ModelEntry> {
    config::default_roster()
}

fn agent(runtime: Option<Runtime>, model: &str) -> config::AgentConfig {
    config::AgentConfig {
        runtime,
        model: model.into(),
        effort: Effort::Medium,
    }
}

#[test]
fn resolve_orchestrator_order() {
    let pick = |choice: Option<OrchestratorChoice>, agent: config::AgentConfig| {
        resolve_orchestrator(choice.as_ref(), &agent, Runtime::Claude, &roster()).map(|r| {
            (
                r.route.runtime,
                r.route.model,
                r.route.strength,
                r.route.effort,
            )
        })
    };
    let claude = |model: &str| {
        Some(OrchestratorChoice {
            runtime: Runtime::Claude,
            model: Some(model.into()),
        })
    };
    // Default: the runtime's first frontier entry, at the agent's effort.
    assert_eq!(
        pick(None, agent(None, "")),
        Ok((
            Runtime::Claude,
            "claude-opus-5-5".into(),
            Strength::Frontier,
            Effort::Medium
        ))
    );
    // The agent's model, then the choice's, which wins.
    assert_eq!(
        pick(None, agent(None, "claude-sonnet-5")).map(|r| r.1),
        Ok("claude-sonnet-5".to_string())
    );
    assert_eq!(
        pick(claude("claude-haiku-4-5"), agent(None, "claude-sonnet-5")).map(|r| r.1),
        Ok("claude-haiku-4-5".to_string())
    );
    // The agent's runtime, then the choice's.
    let codex = Some(OrchestratorChoice {
        runtime: Runtime::Codex,
        model: None,
    });
    assert_eq!(
        pick(codex, agent(Some(Runtime::Claude), "")),
        Ok((
            Runtime::Codex,
            String::new(),
            Strength::Standard,
            Effort::Medium
        )),
        "Codex has no frontier entry: its strongest, the default model at standard"
    );
    assert_eq!(
        pick(None, agent(Some(Runtime::Codex), "")).map(|r| r.0),
        Ok(Runtime::Codex)
    );
    // `codex:` names the default model, which the roster has.
    let codex_default = Some(OrchestratorChoice {
        runtime: Runtime::Codex,
        model: Some(String::new()),
    });
    assert_eq!(
        pick(codex_default, agent(None, "")).map(|r| r.0),
        Ok(Runtime::Codex)
    );
    // A model the roster does not have is refused, the choice's or the agent's.
    assert_eq!(
        pick(claude("gpt-x"), agent(None, "")),
        Err("claude:gpt-x is not in the roster".to_string())
    );
    assert_eq!(
        pick(None, agent(Some(Runtime::Codex), "o9")),
        Err("codex:o9 is not in the roster".to_string())
    );
}

#[test]
fn resolve_orchestrator_keeps_the_candidate_snapshot() {
    let route = |model: &str, strength| proto::Route {
        runtime: Runtime::Claude,
        model: model.into(),
        strength,
        effort: Effort::Medium,
    };
    let taken = |model: &str, strength, why: Option<&str>| RoutingCandidate {
        route: route(model, strength),
        skipped_reason: why.map(str::to_string),
    };
    let default = resolve_orchestrator(None, &agent(None, ""), Runtime::Claude, &roster()).unwrap();
    assert_eq!(default.source, "roster_default");
    let earlier = Some("an earlier candidate was taken");
    assert_eq!(
        default.candidates,
        vec![
            taken("claude-haiku-4-5", Strength::Fast, earlier),
            taken("claude-sonnet-5", Strength::Standard, earlier),
            taken("claude-opus-5-5", Strength::Frontier, None),
        ]
    );
    let configured = resolve_orchestrator(
        None,
        &agent(None, "claude-sonnet-5"),
        Runtime::Claude,
        &roster(),
    )
    .unwrap();
    assert_eq!(configured.source, "agent_config");
    let listed = Some("not in the configured list");
    assert_eq!(
        configured.candidates,
        vec![
            taken("claude-haiku-4-5", Strength::Fast, listed),
            taken("claude-sonnet-5", Strength::Standard, None),
            taken("claude-opus-5-5", Strength::Frontier, listed),
        ]
    );
    let choice = OrchestratorChoice {
        runtime: Runtime::Codex,
        model: None,
    };
    let chosen =
        resolve_orchestrator(Some(&choice), &agent(None, ""), Runtime::Claude, &roster()).unwrap();
    assert_eq!(chosen.source, "explicit_choice");
    assert_eq!(chosen.candidates.len(), 1);
    assert_eq!(chosen.candidates[0].route, chosen.route);
    // A runtime with no roster entry: its default model, appended as the one candidate.
    let bare =
        resolve_orchestrator(None, &agent(None, ""), Runtime::Codex, &roster()[..3]).unwrap();
    assert_eq!(bare.route.model, "");
    assert_eq!(bare.candidates.len(), 1);
    assert_eq!(bare.candidates[0].skipped_reason, None);
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

/// Design rule: no model approves. The orchestrator's tools reach no approval of the
/// plan, a task or a hold; only the user's requests do.
#[test]
fn orchestrator_tools_cannot_approve() {
    let mut fx = launched(false);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    let before = fx.run().clone();
    for edit in [
        json!({"op": "override", "task_id": "t1", "reason": "fine"}),
        json!({"op": "approve"}),
        json!({"op": "approve_hold", "hold": "promotion"}),
    ] {
        let effects = edit_plan(&mut fx, json!({"edits": [edit]}));
        assert!(
            error(&effects).starts_with("invalid arguments: edits[0]: unknown variant"),
            "{effects:?}"
        );
    }
    for tool in ["approve", "approve_hold", "accept", "override"] {
        let effects = orch_tool(&mut fx, ORCH, tool, json!({"hold": "promotion"}));
        assert_eq!(
            error(&effects),
            format!("tool {tool} is not available to the orchestrator role")
        );
    }
    // Submitting again at the gate approves nothing.
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    assert_eq!(*fx.run(), before);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert_eq!(fx.run().approved_by, None);
}
