//! Milestone 9.6 task M9.6.8: two independent brainstormers (DF §3, decisions 9 to 11).
//! `start_brainstorm` launches them in reader slots; each submits one checked draft
//! from its own live window (task 6's review c); both drafts in, or one draft and one
//! failure, wake the orchestrator; both failed halt the run, and `run resume` relaunches
//! them; a restart relaunches a running one fresh. Their sessions are stubbed by their
//! ops' results and their ended events.

use proto::{AgentRole, Runtime, TokenUsage, ToolCall};
use serde_json::{Value, json};

use super::design_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use crate::run::design::state::{DesignAgent, DesignAgentState};
use crate::run::engine::{Effect, EventKind, OpKind, OpResult, OrchEvent, ScoutEnd};
use crate::scout::design_spec::{DesignAgentKind, DesignAgentSpec, LENS_A, LENS_B};

/// The brainstormers' windows in these tests.
pub(super) const CLAUDE: u32 = 801;
pub(super) const CODEX: u32 = 802;

pub(super) const DRAFT: &str = "\
## Understanding
Users reset their password by mail.

## Assumptions
- [assumed] mail works.

## Constraints found
- crates/auth/src/lib.rs:10 keeps the tokens.

## Approaches
### Stored tokens
A table. Size M.

## Recommendation
Stored tokens.

## Questions for you
- How long should a link live?
";

/// Every `StartDesignAgent` op so far, oldest first, with its spec.
pub(super) fn launches(fx: &Fixture) -> Vec<(crate::run::model::OpId, DesignAgentSpec)> {
    (fx.ops("StartDesignAgent").into_iter())
        .map(|(op, kind)| match kind {
            OpKind::StartDesignAgent { spec } => (op, *spec),
            other => panic!("{other:?}"),
        })
        .collect()
}

/// `label`'s latest launch answered with its window.
pub(super) fn started(fx: &mut Fixture, label: &str, window: u32) -> Vec<Effect> {
    let (op, _) = (launches(fx).into_iter().rev())
        .find(|(_, s)| s.kind.label() == label)
        .unwrap_or_else(|| panic!("no launch of {label}"));
    fx.done(op, OpResult::DesignAgentStarted { window_id: window })
}

/// A brainstormer's `submit_doc` from `window`.
pub(super) fn submit_draft(fx: &mut Fixture, window: u32, text: &str) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Orch(OrchEvent::Tool {
        reply,
        call: ToolCall {
            run_id: RUN_ID.into(),
            task_id: None,
            role: AgentRole::Brainstormer,
            window_id: window,
            tool: "submit_doc".into(),
            args: json!({"kind": "brainstorm_draft", "text": text}),
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
        },
        refusals: Vec::new(),
    }))
}

/// [`submit_draft`]'s one answer: `(ok, value)`.
pub(super) fn answered(effects: &[Effect]) -> (bool, Value) {
    match &replies(effects)[..] {
        [Ok(text)] => (true, serde_json::from_str(text).unwrap()),
        [Err(text)] => (false, serde_json::from_str(text).unwrap()),
        other => panic!("{other:?}"),
    }
}

/// `label`'s session `session` ended.
pub(super) fn ended(fx: &mut Fixture, label: &str, session: u32, outcome: ScoutEnd) -> Vec<Effect> {
    fx.next(EventKind::Orch(OrchEvent::DesignAgentEnded {
        run_id: RUN_ID.into(),
        role: AgentRole::Brainstormer,
        label: label.into(),
        session,
        outcome,
        usage: TokenUsage {
            input: 100,
            output: 20,
            ..TokenUsage::default()
        },
        calls: 7,
    }))
}

pub(super) fn agents(fx: &Fixture) -> Vec<DesignAgent> {
    fx.run().orch.design.as_ref().unwrap().brainstormers.clone()
}

pub(super) fn states(fx: &Fixture) -> Vec<DesignAgentState> {
    agents(fx).into_iter().map(|a| a.state).collect()
}

/// A design run whose brainstorm started: both brainstormers launched and live.
pub(super) fn brainstorming() -> Fixture {
    let mut fx = design_launched(false);
    start_brainstorm(&mut fx);
    started(&mut fx, "claude", CLAUDE);
    started(&mut fx, "codex", CODEX);
    fx
}

/// Decision 10 and DF §3.1: with Claude and Codex installed, `start_brainstorm`
/// launches two brainstormers at once, the strongest model of each runtime, with the
/// same contract and the same first turn; each is named by its label.
#[test]
fn two_brainstormers_launch_on_two_runtimes_with_the_same_prompt() {
    let mut fx = design_launched(false);
    assert!(launches(&fx).is_empty(), "nothing before start_brainstorm");
    start_brainstorm(&mut fx);
    let specs: Vec<DesignAgentSpec> = launches(&fx).into_iter().map(|(_, s)| s).collect();
    assert_eq!(specs.len(), 2, "both at once");
    let routes: Vec<(String, Runtime, String)> = (specs.iter())
        .map(|s| (s.kind.label(), s.route.runtime, s.route.model.clone()))
        .collect();
    let roster = &fx.run().roster;
    let strongest = |runtime| {
        crate::run::roster::strongest_of(roster, runtime)
            .unwrap()
            .model
            .clone()
    };
    assert_eq!(
        routes,
        [
            ("claude".into(), Runtime::Claude, strongest(Runtime::Claude)),
            ("codex".into(), Runtime::Codex, strongest(Runtime::Codex)),
        ]
    );
    assert_eq!(specs[0].first_turn, specs[1].first_turn);
    assert_eq!(
        specs[0].headless.instructions,
        specs[1].headless.instructions
    );
    for spec in &specs {
        let mcp = spec.headless.mcp.as_ref().unwrap();
        assert_eq!(mcp.role, AgentRole::Brainstormer);
        assert_eq!(mcp.agent_label, Some(spec.kind.label()));
        assert_eq!(spec.session, 1);
    }
    assert_eq!(
        states(&fx),
        [DesignAgentState::Running, DesignAgentState::Running]
    );
    // Each launch opens its routing record.
    let ids: Vec<String> = (fx.run().role_routing_decisions.iter())
        .filter(|d| d.role == AgentRole::Brainstormer)
        .map(|d| d.session_id.clone())
        .collect();
    assert_eq!(ids, ["claude/1", "codex/1"]);
    assert!(
        agents(&fx).iter().all(|a| !a.listed),
        "the roster's defaults"
    );
}

/// Decision 10: with one runtime installed, its strongest model runs twice, as `A` with
/// lens A's line and `B` with lens B's.
#[test]
fn with_one_runtime_the_strongest_model_runs_twice_with_lenses_a_and_b() {
    let mut fx = design_launched(false);
    fx.run_mut().orch.installed = [("codex".to_string(), false)].into();
    start_brainstorm(&mut fx);
    let specs: Vec<DesignAgentSpec> = launches(&fx).into_iter().map(|(_, s)| s).collect();
    let claude = crate::run::roster::strongest_of(&fx.run().roster, Runtime::Claude).unwrap();
    for (spec, (label, lens)) in specs.iter().zip([("A", LENS_A), ("B", LENS_B)]) {
        assert_eq!(
            spec.kind,
            DesignAgentKind::Brainstormer {
                label: label.into()
            }
        );
        assert_eq!(
            (spec.route.runtime, spec.route.model.as_str()),
            (Runtime::Claude, claude.model.as_str())
        );
        assert!(spec.first_turn.contains(lens), "{}", spec.first_turn);
    }
    assert_eq!(specs.len(), 2);
}

/// Decision 9: each brainstormer holds a reader slot; with one slot the second waits
/// until the first has ended.
#[test]
fn brainstormers_hold_reader_slots_and_wait_for_them() {
    let mut fx = design_launched(false);
    fx.run_mut().limits.max_readers = 1;
    start_brainstorm(&mut fx);
    assert_eq!(launches(&fx).len(), 1);
    assert_eq!(
        states(&fx),
        [DesignAgentState::Running, DesignAgentState::Queued]
    );
    assert_eq!(crate::run::engine::schedule::readers_busy(fx.run()), 1);
    started(&mut fx, "claude", CLAUDE);
    fx.tick();
    assert_eq!(launches(&fx).len(), 1, "still waiting for the slot");
    let (ok, _) = answered(&submit_draft(&mut fx, CLAUDE, DRAFT));
    assert!(ok);
    assert_eq!(
        launches(&fx).len(),
        2,
        "the slot is free once the draft is in"
    );
    assert_eq!(launches(&fx)[1].1.kind.label(), "codex");
}

/// DF §3.3: a draft is checked against its template; a refused one gets the exact
/// reason and may be resubmitted within the budget; the accepted one is held, and its
/// session retired. Ruling T8-1: while the other brainstormer runs, the held draft is
/// neither written nor indexed, so no file of it exists and `get_doc` cannot find it.
#[test]
fn a_draft_is_checked_and_a_refused_draft_may_be_resubmitted() {
    let mut fx = brainstorming();
    let partial = DRAFT.replace("## Constraints found", "## Constraints");
    let (ok, value) = answered(&submit_draft(&mut fx, CODEX, &partial));
    assert!(!ok);
    assert_eq!(
        value["error"],
        "the brainstorm draft is missing the section \"## Constraints found\""
    );
    let big = format!("{DRAFT}{}", "x".repeat(12 * 1024));
    let (_, value) = answered(&submit_draft(&mut fx, CODEX, &big));
    assert_eq!(
        value["error"],
        "the brainstorm draft is over its 12 KiB cap"
    );
    assert_eq!(states(&fx)[1], DesignAgentState::Running);
    let effects = submit_draft(&mut fx, CODEX, DRAFT);
    let (ok, value) = answered(&effects);
    assert!(ok, "{value}");
    assert_eq!(value, json!({"accepted": true, "kind": "brainstorm_draft"}));
    assert!(
        !(effects.iter()).any(|e| matches!(e, Effect::WriteDoc { .. })),
        "held, not written: {effects:?}"
    );
    assert!(effects.contains(&Effect::PlannerAccepted { window_id: CODEX }));
    assert_eq!(states(&fx)[1], DesignAgentState::Submitted);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(
        design.versions.is_empty(),
        "not indexed: {:?}",
        design.versions
    );
    assert_eq!(design.draft_from("codex"), None);
    assert_eq!(design.held, [("codex".to_string(), DRAFT.to_string())]);
    // Its one draft is in: a second is refused, from a window no longer live.
    let (ok, value) = answered(&submit_draft(&mut fx, CODEX, DRAFT));
    assert!(!ok);
    assert_eq!(
        value["error"],
        format!("this window is not a brainstormer of run {RUN_ID}")
    );
}

/// Task 6's review (c): a draft is accepted only from a live brainstormer's window;
/// the orchestrator cannot submit one, nor a brainstormer another kind.
#[test]
fn a_draft_from_a_window_that_is_not_live_is_refused() {
    let mut fx = brainstorming();
    let (ok, value) = answered(&submit_draft(&mut fx, 999, DRAFT));
    assert!(!ok);
    assert_eq!(
        value["error"],
        format!("this window is not a brainstormer of run {RUN_ID}")
    );
    assert!(fx.run().orch.design.as_ref().unwrap().versions.is_empty());
    let args = json!({"kind": "brainstorm_draft", "text": DRAFT});
    let effects = orch_tool(&mut fx, ORCH, "submit_doc", args);
    assert!(replies(&effects)[0].is_err(), "{effects:?}");
}

/// Task 6's review (c): a draft that arrives before its launch's result binds the
/// window is held, as a sub-planner's `submit_epic` is, then applied once it binds.
#[test]
fn a_draft_before_its_window_is_bound_is_held_then_applied() {
    let mut fx = design_launched(false);
    start_brainstorm(&mut fx);
    let effects = submit_draft(&mut fx, CLAUDE, DRAFT);
    assert!(replies(&effects).is_empty(), "held: {effects:?}");
    assert_eq!(states(&fx)[0], DesignAgentState::Running);
    let effects = started(&mut fx, "claude", CLAUDE);
    let (ok, value) = answered(&effects);
    assert!(ok, "{value}");
    assert_eq!(states(&fx)[0], DesignAgentState::Submitted);
}

/// DF §3.2: neither brainstormer can read the other's draft: `get_doc` is not its tool,
/// in the MCP listing, the Claude allowlist or the daemon's parse (so a `from` from a
/// brainstormer is refused), and its session cannot read the design folder.
#[test]
fn neither_brainstormer_can_get_the_others_draft() {
    let fx = brainstorming();
    let names: Vec<String> = (mcp::tools::tools_for(AgentRole::Brainstormer).iter())
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(names, ["submit_doc"]);
    let spec = &launches(&fx)[0].1;
    assert!(
        !(spec.headless.allowed_tools.iter()).any(|t| t.contains("get_doc")),
        "{:?}",
        spec.headless.allowed_tools
    );
    let args = json!({"kind": "brainstorm_draft", "from": "codex"});
    let refused = crate::run::orch::tools::parse_call(AgentRole::Brainstormer, "get_doc", &args);
    assert_eq!(
        refused.err().as_deref(),
        Some("tool get_doc is not available to the brainstormer role")
    );
    let design = crate::run::design::state::design_dir(fx.run());
    let other = design.join("brainstorm/draft-codex.md");
    let sandbox = spec.headless.claude_sandbox.as_ref().unwrap();
    assert!(sandbox.deny_read.iter().any(|d| other.starts_with(d)));
}
