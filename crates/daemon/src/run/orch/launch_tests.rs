//! Milestone 9 task M9.8: a sub-planner's session (decision 31). Pure.

use std::path::Path;

use proto::{AgentRole, Runtime};

use super::*;
use crate::headless::SessionArg;
use crate::headless::argv::{CLI_CAPS, CliCaps, claude_args, codex_args, mcp_args};
use crate::run::orch::test_support::run_with;
use crate::run::orch::{EpicRecord, PlannerPhase};
use crate::run::role_launch::{
    REVIEWER_CODEX_SANDBOX, REVIEWER_DISALLOWED_TOOLS, REVIEWER_PERMISSION_MODE,
    protected_write_denials,
};
use crate::run::test_support::task_toml;
use crate::scout::planner::planner_names;

fn legacy_caps() -> CliCaps {
    CliCaps {
        codex_sandbox: crate::headless::codex_sandbox::DialectChoice::Fixed(
            crate::headless::codex_sandbox::CodexSandboxDialect::Legacy,
        ),
        ..CLI_CAPS
    }
}

fn mail(runtime: Runtime) -> (crate::run::model::Run, EpicRecord) {
    let run = run_with(&[task_toml("t1", "S", "[\"crates/auth/**\"]", "")]);
    let mut epic = EpicRecord::new("mail", PlannerPhase::Queued);
    epic.route.runtime = runtime;
    epic.request = "Plan the mail module.".into();
    epic.scout_refs = vec!["onboarding".into()];
    (run, epic)
}

#[test]
fn planner_spec_is_read_only_and_names_its_epic() {
    let (run, epic) = mail(Runtime::Claude);
    let spec = planner_spec(&run, &epic, 2);
    assert_eq!(
        (spec.run_id.as_str(), spec.epic.as_str(), spec.session),
        (run.id.as_str(), "mail", 2)
    );
    let h = &spec.headless;
    assert_eq!(
        h.cwd, run.root,
        "it reads the user's checkout (decision 20a)"
    );
    assert_eq!(spec.cwd, run.root);
    assert_eq!(spec.project, run.project);
    assert_eq!(h.instructions, crate::run::orch::contract::PLANNER_CONTRACT);
    assert_eq!(
        h.allowed_tools,
        [
            "mcp__anthrex__get_context",
            "mcp__anthrex__submit_epic",
            "Read",
            "Glob",
            "Grep"
        ]
    );
    // Every headless role runs with an explicit permission mode.
    assert_eq!(
        h.claude_permission_mode.as_deref(),
        Some(REVIEWER_PERMISSION_MODE)
    );
    assert_eq!(h.claude_disallowed_tools, REVIEWER_DISALLOWED_TOOLS);
    assert_eq!(h.output_filter, None);
    let run_ref = h.run_ref.as_ref().unwrap();
    assert_eq!(
        (run_ref.role, run_ref.task_id.as_deref(), run_ref.session),
        (AgentRole::Planner, None, 2)
    );
    let target = h.mcp.as_ref().unwrap();
    assert_eq!(target.role, AgentRole::Planner);
    assert_eq!(target.epic.as_deref(), Some("mail"));
    let args = mcp_args(target, 9, Path::new("/tmp/s.sock")).unwrap();
    let at = args.iter().position(|a| a == "--epic").unwrap();
    assert_eq!(args[at + 1], "mail");
    assert_eq!(args[at + 2], "--window");
    // Its window is `<h4>/plan-<e>.p<n>`, its internal id `<h4>-plan-<e>-<n>`.
    assert_eq!(
        planner_names(&run.id, "mail", 2),
        (
            format!("{}-plan-mail-2", run.short()),
            format!("{}/plan-mail.p2", run.short())
        )
    );
    // Its limits are `[orchestrator.planners]`, and its first turn the planner prompt
    // with the slot for the epic's scout extract.
    let limits = &run.limits.orch.planners;
    assert_eq!(
        (spec.max_tool_calls, spec.timeout_secs),
        (limits.max_tool_calls, limits.timeout_secs)
    );
    assert_eq!(
        spec.first_turn,
        crate::run::orch::contract::planner_prompt(&run, &epic, "")
    );
    let slot = spec.extract.as_ref().unwrap();
    assert_eq!(slot.refs, vec!["onboarding".to_string()]);
    // The Claude argv carries the mode and the MCP server with `--epic`.
    let argv = claude_args(
        h,
        &SessionArg::New { uuid: None },
        Path::new("/bin/anthrex"),
        9,
        Path::new("/tmp/s.sock"),
        &CLI_CAPS,
    );
    let mode = argv.iter().position(|a| a == "--permission-mode").unwrap();
    assert_eq!(argv[mode + 1], REVIEWER_PERMISSION_MODE);
    assert!(
        argv.iter().any(|a| a.contains("\"--epic\",\"mail\"")),
        "{argv:?}"
    );
}

/// Followups file, F1 N4: a read-only role runs under a read-only OS sandbox. A
/// research session's spec is task M9.9's (`research_spec`); it gets the same test.
#[test]
fn planner_and_research_specs_carry_the_read_only_sandbox() {
    let (run, epic) = mail(Runtime::Claude);
    let h = planner_spec(&run, &epic, 1).headless;
    let sandbox = h.claude_sandbox.expect("a Claude planner is sandboxed");
    assert!(sandbox.writable_roots.is_empty(), "{sandbox:?}");
    for denied in protected_write_denials(&run.root, &[]).iter().chain([
        &run.root,
        &run.project,
        &run.git_common_dir,
    ]) {
        assert!(
            sandbox.deny_write.contains(denied),
            "{denied:?}: {sandbox:?}"
        );
    }
    assert_eq!(h.codex_sandbox, REVIEWER_CODEX_SANDBOX);
    assert!(h.codex_writable_roots.is_empty());
    // On Codex: `-s read-only`, no writable root, and the run's config guard.
    let (run, epic) = mail(Runtime::Codex);
    let h = planner_spec(&run, &epic, 1).headless;
    assert_eq!(h.claude_sandbox, None);
    assert_eq!(h.claude_permission_mode, None);
    assert_eq!(h.codex_sandbox, "read-only");
    assert_eq!(
        h.codex_config_guard,
        crate::run::role_launch::codex_config_guard(&run, Runtime::Codex)
    );
    let argv = codex_args(
        &h,
        &SessionArg::New { uuid: None },
        "go",
        Path::new("/bin/anthrex"),
        9,
        Path::new("/tmp/s.sock"),
        &legacy_caps(),
    );
    let at = argv
        .iter()
        .position(|a| a == "-s" || a == "--sandbox")
        .unwrap();
    assert_eq!(argv[at + 1], "read-only", "{argv:?}");
}

/// Decision 35 (task M9.9): the research half of the F1 N4 rule. A research task runs
/// as an area scout bound to its task: read-only on both runtimes, the scout contract
/// and tools with the web ones, the task's route, in the user's checkout.
#[test]
fn research_spec_is_read_only_and_bound_to_its_task() {
    for runtime in [Runtime::Claude, Runtime::Codex] {
        let mut run = run_with(&[task_toml("r1", "S", "[]", "kind = \"research\"")]);
        run.tasks[0].route.runtime = runtime;
        run.tasks[0].session = 1;
        let task = run.tasks[0].clone();
        let h = research_spec(&run, &task);
        assert_eq!(h.runtime, runtime);
        assert_eq!(h.model, task.route.model);
        assert_eq!(h.cwd, run.root);
        assert_eq!(h.instructions, crate::scout::contract::SCOUT_CONTRACT);
        let mcp = h.mcp.clone().unwrap();
        assert_eq!(
            (mcp.role, mcp.task_id.as_deref(), mcp.scout_id, mcp.epic),
            (AgentRole::Scout, Some("r1"), None, None)
        );
        let run_ref = h.run_ref.clone().unwrap();
        assert_eq!(
            (run_ref.role, run_ref.task_id.as_deref(), run_ref.session),
            (AgentRole::Scout, Some("r1"), 1)
        );
        assert_eq!(
            h.allowed_tools,
            [
                "mcp__anthrex__submit_scout_report",
                "Read",
                "Glob",
                "Grep",
                "WebFetch",
                "WebSearch"
            ]
        );
        assert_eq!(h.codex_sandbox, REVIEWER_CODEX_SANDBOX);
        assert!(h.codex_writable_roots.is_empty());
        assert_eq!(h.output_filter, None);
        match runtime {
            Runtime::Claude => {
                let sandbox = h.claude_sandbox.clone().expect("sandboxed");
                assert!(sandbox.writable_roots.is_empty());
                for denied in [&run.root, &run.project, &run.git_common_dir] {
                    assert!(sandbox.deny_write.contains(denied), "{sandbox:?}");
                }
                assert_eq!(
                    h.claude_permission_mode.as_deref(),
                    Some(REVIEWER_PERMISSION_MODE)
                );
                assert_eq!(h.claude_disallowed_tools, REVIEWER_DISALLOWED_TOOLS);
            }
            _ => {
                assert_eq!(h.claude_sandbox, None);
                assert_eq!(
                    h.codex_config_guard,
                    crate::run::role_launch::codex_config_guard(&run, Runtime::Codex)
                );
            }
        }
    }
}

fn row(model: &str, effort: Option<&str>) -> proto::models::RoleChoice {
    proto::models::RoleChoice {
        model: proto::models::ModelRef::parse(model).expect("a model"),
        effort: effort.map(str::to_string),
        fallback: None,
    }
}

fn model_of(route: &Route) -> (Runtime, &str, &str) {
    (route.runtime, route.model.as_str(), route.effort.as_str())
}

/// Milestone 9.8 (MR §3.1): the orchestrator takes its row (source `role_table`)
/// unless the goal form chose (`explicit_choice`); the choice's effort, else the row's
/// when it chose the row's own model, else the model's default. The candidates are the
/// chosen route alone.
#[test]
fn the_orchestrator_takes_its_row_unless_the_goal_form_chose() {
    use proto::models::Role;
    let mut run = run_with(&[task_toml("t1", "S", "[\"a/**\"]", "")]);
    let sol = "codex:gpt-6.1-sol";
    crate::run::test_support::set_row(&mut run, Role::Orchestrator, sol, Some("high"), None);
    let models = run.limits.models();
    let only = |r: &Resolved| {
        let one = proto::RoutingCandidate {
            route: r.route.clone(),
            skipped_reason: None,
        };
        assert_eq!(r.candidates, [one]);
    };
    let row = orchestrator_route(None, models);
    assert_eq!(
        model_of(&row.route),
        (Runtime::Codex, "gpt-6.1-sol", "high")
    );
    assert_eq!(row.source, "role_table");
    only(&row);
    let choice = |runtime, model: &str, effort: Option<&str>| OrchestratorChoice {
        runtime,
        model: Some(model.to_string()),
        effort: effort.map(str::to_string),
    };
    let opus = choice(Runtime::Claude, "claude-opus-5-5", Some("max"));
    let chosen = orchestrator_route(Some(&opus), models);
    assert_eq!(
        model_of(&chosen.route),
        (Runtime::Claude, "claude-opus-5-5", "max")
    );
    assert_eq!(chosen.source, "explicit_choice");
    only(&chosen);
    let same = orchestrator_route(Some(&choice(Runtime::Codex, "gpt-6.1-sol", None)), models);
    assert_eq!(
        model_of(&same.route),
        (Runtime::Codex, "gpt-6.1-sol", "high")
    );
    let other = orchestrator_route(Some(&choice(Runtime::Codex, "gpt-6-luna", None)), models);
    assert_eq!(other.route.effort, proto::Effort::DEFAULT);
    assert_eq!(other.route.model, "gpt-6-luna");
}

/// Milestone 9.8 (MR §3.1): a sub-planner takes the `planner` row, once the run has
/// an orchestrator.
#[test]
fn planner_takes_the_planner_row() {
    use proto::models::Role;
    let mut run = run_with(&[task_toml("t1", "S", "[\"a/**\"]", "")]);
    let luna = "codex:gpt-6-luna";
    crate::run::test_support::set_row(&mut run, Role::Planner, luna, Some("medium"), None);
    assert_eq!(planner_route(&run), None, "no orchestrator yet");
    run.orch.orchestrator = Some(crate::run::orch::test_support::orchestrator());
    let route = planner_route(&run).expect("a planner route");
    assert_eq!(model_of(&route), (Runtime::Codex, "gpt-6-luna", "medium"));
}

/// Milestone 9.8 (MR §3.1): the run scouts and a research task take the `research` row.
#[test]
fn scouts_and_research_tasks_take_the_research_row() {
    use proto::models::Role;
    let mut config = config::Orchestrator::default();
    let sonnet = row("claude:claude-sonnet-5", Some("medium"));
    config.roles.rows.insert(Role::Research, sonnet);
    let plan = crate::run::test_support::plan_with(
        crate::run::test_support::PROFILE,
        &[task_toml("r1", "S", "[]", "kind = \"research\"")],
    );
    let run = crate::run::test_support::build_with(&plan, &config)
        .unwrap_or_else(|e| panic!("{}", crate::run::test_support::show(&e)));
    let want = (Runtime::Claude, "claude-sonnet-5", "medium");
    assert_eq!(model_of(&run.tasks[0].route), want);
    assert_eq!(model_of(&scout_route(&run)), want);
}
