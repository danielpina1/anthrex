//! Milestone 9.6 task M9.6.8: a brainstormer's launch. Its contract and first turn, its
//! exact Claude allowlist (and the document reviewer's, task 6's review b), a session
//! that cannot read the run's design folder (task 6's review a), and its machine's
//! texts, whose budget failures count as failed (DF §2.2).

use std::path::Path;

use proto::{AgentRole, Effort, Route, Runtime, Strength};
use serde_json::Value;

use super::*;
use crate::headless::SessionArg;
use crate::headless::argv::{CLI_CAPS, claude_args, mcp_args};
use crate::run::design::state::{DesignAgent, DesignAgentState, design_dir};
use crate::run::model::Run;
use crate::scout::machine::{ScoutEvent, ScoutLimits, step};
use crate::scout::spec::AREA_SCOUT_READ_TOOLS;

fn route(runtime: Runtime) -> Route {
    Route {
        runtime,
        model: "m".into(),
        strength: Strength::Frontier,
        effort: Effort::High,
    }
}

fn agent(label: &str, runtime: Runtime) -> DesignAgent {
    DesignAgent {
        label: label.into(),
        role: AgentRole::Brainstormer,
        route: route(runtime),
        session: 1,
        window_id: None,
        state: DesignAgentState::Running,
        calls: 0,
        tokens: 0,
        started: Some(1_000),
        listed: false,
    }
}

fn run() -> Run {
    let mut run = crate::run::orch::test_support::run_of(1);
    run.data_dir = format!("/tmp/data/runs/{}", run.id).into();
    run
}

/// The `--settings` JSON of a Claude session's argv.
fn settings(spec: &crate::headless::HeadlessSpec) -> Value {
    let args = claude_args(
        spec,
        &SessionArg::New { uuid: None },
        Path::new("/opt/anthrex"),
        7,
        Path::new("/tmp/sock"),
        &CLI_CAPS,
    );
    let at = args
        .iter()
        .position(|a| a == "--settings")
        .expect("settings");
    serde_json::from_str(&args[at + 1]).expect("json")
}

#[test]
fn the_brainstormer_contract_is_short_and_names_its_tool() {
    assert!(
        BRAINSTORMER_CONTRACT.len() < 1024,
        "{}",
        BRAINSTORMER_CONTRACT.len()
    );
    for needle in [
        "submit_doc",
        "brainstorm_draft",
        "file:line",
        "## Understanding",
        "## Questions for you",
        "12 KiB",
    ] {
        assert!(BRAINSTORMER_CONTRACT.contains(needle), "{needle}");
    }
    assert!(
        !BRAINSTORMER_CONTRACT.contains("skill"),
        "anthrex's own system"
    );
}

/// Task 6's review (b): each design role's Claude allowlist is exactly its MCP tools
/// (in `anthrex mcp`'s order) and an area scout's read tools; a brainstormer has no
/// `get_doc`, so it reads no draft.
#[test]
fn the_design_agents_allowlists_are_exact() {
    for (role, list) in [
        (AgentRole::Brainstormer, BRAINSTORMER_ALLOWED_TOOLS),
        (AgentRole::DocReviewer, DOC_REVIEWER_ALLOWED_TOOLS),
    ] {
        let mut want: Vec<String> = mcp::tools::tools_for(role)
            .iter()
            .map(|t| format!("mcp__anthrex__{}", t.name))
            .collect();
        want.extend(AREA_SCOUT_READ_TOOLS.iter().map(|t| t.to_string()));
        assert_eq!(list, want, "{role:?}");
    }
    assert_eq!(
        BRAINSTORMER_ALLOWED_TOOLS,
        ["mcp__anthrex__submit_doc", "Read", "Glob", "Grep"]
    );
    assert_eq!(
        DOC_REVIEWER_ALLOWED_TOOLS,
        [
            "mcp__anthrex__get_doc",
            "mcp__anthrex__submit_findings",
            "Read",
            "Glob",
            "Grep"
        ]
    );
}

/// Task 6's review (a): a brainstormer's session is a read-only scout's, named by its
/// label, and on Claude it cannot read anything under the run's design folder, where
/// the other brainstormer's draft is written: the settings deny it to the read tools,
/// and the sandbox to commands.
#[test]
fn a_brainstormers_session_cannot_read_the_design_folder() {
    let run = run();
    let spec = brainstormer_spec(&run, &agent("claude", Runtime::Claude));
    let headless = &spec.headless;
    assert_eq!(headless.allowed_tools, BRAINSTORMER_ALLOWED_TOOLS);
    assert_eq!(headless.instructions, BRAINSTORMER_CONTRACT);
    assert_eq!(headless.codex_sandbox, "read-only");
    assert!(headless.codex_writable_roots.is_empty());
    let mcp = headless.mcp.as_ref().expect("anthrex mcp");
    assert_eq!(
        (mcp.role, mcp.agent_label.as_deref()),
        (AgentRole::Brainstormer, Some("claude"))
    );
    let args = mcp_args(mcp, 7, Path::new("/tmp/sock")).unwrap();
    assert!(args.windows(2).any(|w| w == ["--agent-label", "claude"]));
    let sandbox = headless.claude_sandbox.as_ref().expect("a sandbox");
    assert!(sandbox.writable_roots.is_empty());
    let design = design_dir(&run);
    assert_eq!(sandbox.deny_read, std::slice::from_ref(&design));

    let settings = settings(headless);
    let rule = format!("Read(/{}/**)", design.display());
    assert!(rule.starts_with("Read(//tmp/data/runs/"), "{rule}");
    assert_eq!(settings["permissions"]["deny"], serde_json::json!([rule]));
    let denied = &settings["sandbox"]["filesystem"]["denyRead"];
    assert_eq!(denied, &serde_json::json!([design.display().to_string()]));
    // The other draft, and every design document, is under the denied folder.
    for other in [
        "brainstorm/draft-codex.md",
        "brainstorm-v1.md",
        "versions.json",
    ] {
        let path = design.join(other);
        assert!(
            sandbox.deny_read.iter().any(|d| path.starts_with(d)),
            "{other}"
        );
    }
    // The budget is `[orchestrator.design.budget] brainstormer`'s.
    let budget = run.limits.orch.design.brainstormer;
    assert_eq!(spec.max_tool_calls, budget.tool_calls);
    assert_eq!(spec.timeout_secs, u64::from(budget.minutes) * 60);
}

/// Decision 10 and DF §3.1: with two runtimes both get the same neutral prompt; with
/// one, the lenses' exact lines.
#[test]
fn the_first_turn_carries_a_lens_only_for_a_and_b() {
    let run = run();
    let claude = brainstormer_spec(&run, &agent("claude", Runtime::Claude));
    let codex = brainstormer_spec(&run, &agent("codex", Runtime::Codex));
    assert_eq!(claude.first_turn, codex.first_turn);
    assert_eq!(claude.headless.instructions, codex.headless.instructions);
    assert!(!claude.first_turn.contains("Your angle"));
    let a = brainstormer_spec(&run, &agent("A", Runtime::Claude));
    let b = brainstormer_spec(&run, &agent("B", Runtime::Claude));
    assert!(a.first_turn.contains(LENS_A), "{}", a.first_turn);
    assert!(b.first_turn.contains(LENS_B), "{}", b.first_turn);
    assert_eq!(
        LENS_A,
        "Your angle: the smallest change that fully meets the goal."
    );
    assert_eq!(LENS_B, "Your angle: the most robust, long-lived design.");
    assert_eq!(
        (a.kind.label(), b.kind.label()),
        ("A".to_string(), "B".to_string())
    );
}

/// DF §2.2: a brainstormer over its budget, in calls or in minutes, is stopped and
/// fails, in its own words.
#[test]
fn a_brainstormer_over_budget_fails() {
    let limits = ScoutLimits {
        timeout_secs: 900,
        max_tool_calls: 40,
        send_mid_turn: true,
        texts: BRAINSTORMER_TEXTS,
    };
    let (mut machine, _) = step(Default::default(), ScoutEvent::Start { now: 0 }, &limits);
    let mut last = Vec::new();
    for _ in 0..60 {
        let (next, effects) = step(machine, ScoutEvent::ToolUse, &limits);
        machine = next;
        last = effects;
    }
    assert_eq!(
        machine.failure.as_deref(),
        Some("the brainstormer used 60 tool calls without an accepted draft")
    );
    assert!(last.contains(&crate::scout::machine::ScoutEffect::Kill));
    let (machine, _) = step(Default::default(), ScoutEvent::Start { now: 0 }, &limits);
    let (machine, _) = step(machine, ScoutEvent::Tick { now: 900 }, &limits);
    assert_eq!(
        machine.failure.as_deref(),
        Some("the brainstormer ran longer than 900 s")
    );
    assert_eq!((BRAINSTORMER_TEXTS.wrap_up)(40), brainstormer_wrap_up(40));
    assert_eq!(BRAINSTORMER_TEXTS.submit_tool, "mcp__anthrex__submit_doc");
}

/// A design agent's window whose events come before its start's own bind is found by
/// its `anthrex mcp` target (`ScoutService::scout_of`): its run, role and label, never a
/// sub-planner's epic of the same name.
#[test]
fn an_unbound_design_session_is_named_by_its_role_and_label() {
    let run = run();
    let spec = brainstormer_spec(&run, &agent("codex", Runtime::Codex));
    let target = spec.headless.mcp.clone().unwrap();
    let limits = ScoutLimits {
        timeout_secs: 1,
        max_tool_calls: 1,
        send_mid_turn: false,
        texts: BRAINSTORMER_TEXTS,
    };
    let tag = |role, key: &str| crate::scout::planner::PlannerTag {
        run_id: run.id.clone(),
        role,
        epic: key.into(),
        session: 1,
        limits,
    };
    assert!(tag(AgentRole::Brainstormer, "codex").names(&target));
    assert!(!tag(AgentRole::Brainstormer, "claude").names(&target));
    assert!(!tag(AgentRole::Planner, "codex").names(&target));
    let mut planner = target.clone();
    (planner.role, planner.epic, planner.agent_label) =
        (AgentRole::Planner, Some("codex".into()), None);
    assert!(tag(AgentRole::Planner, "codex").names(&planner));
    assert!(!tag(AgentRole::Brainstormer, "codex").names(&planner));
}
