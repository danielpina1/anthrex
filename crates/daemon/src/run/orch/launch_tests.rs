//! Milestone 9 task M9.8: a sub-planner's session (decision 31). Pure.

use std::path::Path;

use proto::{AgentRole, Runtime};

use super::*;
use crate::headless::SessionArg;
use crate::headless::argv::{CLI_CAPS, claude_args, codex_args, mcp_args};
use crate::run::orch::test_support::run_with;
use crate::run::orch::{EpicRecord, PlannerPhase};
use crate::run::role_launch::{
    REVIEWER_CODEX_SANDBOX, REVIEWER_DISALLOWED_TOOLS, REVIEWER_PERMISSION_MODE,
    protected_write_denials,
};
use crate::run::test_support::task_toml;
use crate::scout::planner::planner_names;

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
        &CLI_CAPS,
    );
    let at = argv
        .iter()
        .position(|a| a == "-s" || a == "--sandbox")
        .unwrap();
    assert_eq!(argv[at + 1], "read-only", "{argv:?}");
}
