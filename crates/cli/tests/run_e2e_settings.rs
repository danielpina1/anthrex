//! Milestone 8a, task 22: decision 53 end to end. Headless sessions load only the
//! user's settings, or `run start` refuses the project settings they would run unasked
//! until `--trust-project` accepts them; each branch of `CLI_CAPS` is exercised through
//! the daemon's debug-build overrides.

mod support;

use serde_json::json;
use support::run_harness::{RUN_WAIT, RunHarness, init_repo};
use support::run_plans::*;

const HOOKED_SETTINGS: &str = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"true"}]}]}}"#;

fn settings_refusal(who: &str, paths: &str) -> String {
    format!(
        "this repository has project settings that headless {who} sessions would run without asking: {paths}; review them, then start again with --trust-project"
    )
}

#[test]
fn e2e_project_settings_are_refused_without_trust_project() {
    let h = RunHarness::with_repo(
        "",
        &[("ANTHREX_TEST_NO_SETTING_SOURCES", "1")],
        true,
        &[(".claude/settings.json", HOOKED_SETTINGS)],
    );
    let claude = plan("", &[task("t1", &["a.txt"], "")]);
    let message = refused(h.start_reply(&h.repo, &claude, true, false));
    assert_eq!(message, settings_refusal("Claude", ".claude/settings.json"));
    assert!(no_run_branches(&h.repo));

    let other = h.dir.path().join("repo2");
    init_repo(&other, &[(".mcp.json", r#"{"mcpServers":{}}"#)]);
    let message = refused(h.start_reply(&other, &claude, true, false));
    assert_eq!(message, settings_refusal("Claude", ".mcp.json"));
    assert!(no_run_branches(&other));

    // Ruling T22-I1: a Codex worker's reviewer runs Claude, so a Codex-only plan is
    // refused too.
    let codex = plan("", &[task("t1", &["a.txt"], CODEX)]);
    let message = refused(h.start_reply(&h.repo, &codex, true, false));
    assert_eq!(message, settings_refusal("Claude", ".claude/settings.json"));
    assert!(no_run_branches(&h.repo));
}

/// Ruling T22-I1: decision 53 covers every runtime a run launches. A Codex worker's
/// reviewer is a headless Claude session, so a repository whose base commit tracks a
/// hooked `.claude/settings.json` needs `--trust-project` even for a Codex-only plan;
/// with it, the run starts and records the file.
#[test]
fn e2e_a_codex_workers_claude_reviewer_needs_trust_project() {
    let h = RunHarness::with_repo(
        "",
        &[("ANTHREX_TEST_NO_SETTING_SOURCES", "1")],
        true,
        &[(".claude/settings.json", HOOKED_SETTINGS)],
    );
    let codex = plan("", &[task("t1", &["a.txt"], CODEX)]);
    let message = refused(h.start_reply(&h.repo, &codex, true, false));
    assert_eq!(message, settings_refusal("Claude", ".claude/settings.json"));
    assert!(no_run_branches(&h.repo));
    let id = h.start_in(&h.repo, &codex, false, true);
    let run = h.run(&id).unwrap();
    assert_eq!(
        run.trusted_project,
        vec![".claude/settings.json".to_string()]
    );
}

/// Final fix batch F2 (C-I3), decision 50's recorded ruling: `auth = "api_key"` is
/// refused at config load (whether anthrex's hooks and MCP server apply under `--bare`
/// is unverified), so `login` is kept. Before F2 the run was refused for want of a key
/// (ruling T22-I1's refusal, which stays for the day `api_key` is verified); now it
/// starts, and no Claude session is launched with `--bare`.
#[test]
fn e2e_api_key_auth_is_refused_at_config_load_and_login_kept() {
    let h = RunHarness::new("\n[orchestrator.claude]\nauth = \"api_key\"");
    green_scripts(&h.repo);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    h.wait_run(&id, complete, RUN_WAIT);
    for name in ["worker-t1-1", "reviewer-t1-1"] {
        let argv: Vec<String> = serde_json::from_str(&h.io_lines(name, "args")[0]).unwrap();
        assert!(!argv.iter().any(|a| a == "--bare"), "{name}: {argv:?}");
    }
}

#[test]
fn e2e_trust_project_is_accepted_and_reported() {
    let h = RunHarness::with_repo(
        "",
        &[("ANTHREX_TEST_NO_SETTING_SOURCES", "1")],
        true,
        &[(".claude/settings.json", HOOKED_SETTINGS)],
    );
    green_scripts(&h.repo);
    let id = h.start_in(
        &h.repo,
        &plan("", &[task("t1", &["a.txt"], "")]),
        true,
        true,
    );
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(
        run.trusted_project,
        vec![".claude/settings.json".to_string()]
    );
    // The report is written after `complete` is published: wait for it with a deadline
    // (it read empty once under load, final fix batch F1c round 2).
    report_with(
        &run,
        "project settings trusted by --trust-project: .claude/settings.json",
    );
}

const CODEX_CONFIG: &[(&str, &str)] = &[(".codex/config.toml", "model = \"x\"\n")];
const EXCLUDE_FLAG: &str = "--anthrex-test-exclude-project-config";

/// The branch where Codex loads project config and cannot be told not to.
fn codex_refuses(h: &RunHarness) {
    let codex = plan("", &[task("t1", &["a.txt"], CODEX)]);
    let message = refused(h.start_reply(&h.repo, &codex, true, false));
    assert_eq!(message, settings_refusal("Codex", ".codex/config.toml"));
    assert!(no_run_branches(&h.repo));
}

/// The branch where Codex is told not to load project config by `flags`: the plan
/// starts, and both the `exec` and the `exec resume` argv carry them.
/// `line`: the report's decision-53 Codex line (ruling T23-C1).
fn codex_excludes(h: &RunHarness, flags: &[&str], line: &str) {
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            json!({"read_message": {"expect": "no task_done"}}),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], CODEX)]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    until("the report's codex line", RUN_WAIT, || {
        report(&run).contains(line).then_some(())
    });
    let argvs: Vec<Vec<String>> = h
        .io_lines("worker-t1-1", "args")
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(argvs.len(), 2, "{argvs:?}");
    assert_eq!(argvs[0][0], "exec");
    assert_eq!(argvs[1][..2], ["exec".to_string(), "resume".to_string()]);
    for argv in &argvs {
        for flag in flags {
            assert!(
                argv.iter().any(|a| a == flag),
                "{flag} missing from {argv:?}"
            );
        }
    }
}

#[test]
fn e2e_codex_project_config_follows_cli_caps() {
    // `load`: refused, naming the file; accepted with --trust-project and reported; a
    // Claude-only plan is refused for it too, since its reviewer runs Codex (ruling
    // T22-I1).
    let h = RunHarness::with_repo(
        "",
        &[("ANTHREX_TEST_CODEX_PROJECT_CONFIG", "load")],
        true,
        CODEX_CONFIG,
    );
    codex_refuses(&h);
    h.script("worker-t1-1", &[commit("a.txt", "a\n"), done("added a")]);
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start_in(
        &h.repo,
        &plan("", &[task("t1", &["a.txt"], CODEX)]),
        true,
        true,
    );
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(run.trusted_project, vec![".codex/config.toml".to_string()]);
    until("the report's trusted and codex lines", RUN_WAIT, || {
        let text = report(&run);
        (text.contains("project settings trusted by --trust-project: .codex/config.toml")
            && text.contains("codex project config: loaded (this Codex CLI cannot exclude it)\n"))
        .then_some(())
    });
    let claude = plan("", &[task("t9", &["z.txt"], "")]);
    let message = refused(h.start_reply(&h.repo, &claude, false, false));
    assert_eq!(message, settings_refusal("Codex", ".codex/config.toml"));
    drop(h);

    // `exclude`: the placeholder flag on both turns' argv.
    let h = RunHarness::with_repo(
        "",
        &[("ANTHREX_TEST_CODEX_PROJECT_CONFIG", "exclude")],
        true,
        CODEX_CONFIG,
    );
    codex_excludes(&h, &[EXCLUDE_FLAG], "codex project config: excluded\n");
    drop(h);

    // Unset: whichever branch the real CLI_CAPS names.
    let caps = daemon::headless::argv::CLI_CAPS;
    let h = RunHarness::with_repo("", &[], true, CODEX_CONFIG);
    match (caps.codex_loads_project_config, caps.codex_user_config_only) {
        (true, None) => codex_refuses(&h),
        (true, Some(flags)) => codex_excludes(&h, flags, "codex project config: excluded\n"),
        (false, _) => codex_excludes(&h, &[], "codex project config: not loaded by this CLI\n"),
    }
}

#[test]
fn e2e_claude_sessions_load_only_user_settings() {
    let tried = ["--setting-sources", "user", "--strict-mcp-config"];
    match daemon::headless::argv::CLI_CAPS.claude_user_settings_only {
        Some(flags) => {
            let h = RunHarness::new("");
            green_scripts(&h.repo);
            let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
            h.wait_run(&id, complete, RUN_WAIT);
            let argv: Vec<String> =
                serde_json::from_str(&h.io_lines("worker-t1-1", "args")[0]).unwrap();
            for flag in flags {
                let n = argv.iter().filter(|a| a == flag).count();
                assert_eq!(n, 1, "{flag} in {argv:?}");
            }
        }
        None => {
            let h =
                RunHarness::with_repo("", &[], true, &[(".claude/settings.json", HOOKED_SETTINGS)]);
            let message = refused(h.start_reply(
                &h.repo,
                &plan("", &[task("t1", &["a.txt"], "")]),
                true,
                false,
            ));
            assert_eq!(message, settings_refusal("Claude", ".claude/settings.json"));
            let h = RunHarness::new("");
            green_scripts(&h.repo);
            let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
            h.wait_run(&id, complete, RUN_WAIT);
            let argv: Vec<String> =
                serde_json::from_str(&h.io_lines("worker-t1-1", "args")[0]).unwrap();
            for flag in [tried[0], tried[2]] {
                assert!(!argv.iter().any(|a| a == flag), "{flag} in {argv:?}");
            }
        }
    }
}

/// A roster that keeps a Claude plan on Claude through every rung (ruling T22-I1b): no
/// Codex entry at a Claude route's strength, and the only one too weak for any reviewer
/// an unreviewed `S` task can get once raised.
const CLAUDE_BOUND: &str = "builtin_models = false\n\n[orchestrator.review]\nsmall = \"off\"\n\n[[orchestrator.models]]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"\nstrength = \"standard\"\n\n[[orchestrator.models]]\nruntime = \"claude\"\nmodel = \"claude-opus-5\"\nstrength = \"frontier\"\n\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-5-codex-mini\"\nstrength = \"fast\"\n";

/// A roster with Codex entries only, so a Codex plan never reaches Claude.
const CODEX_ONLY: &str = "builtin_models = false\n\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-5-codex\"\nstrength = \"standard\"\n";

/// The brief's case, where the run really stays on Claude: a Claude plan is not refused
/// for `.codex/config.toml`. An edit that adds a Codex task would reach Codex, whose
/// project config was never checked, and is refused with decision 53's text.
#[test]
fn e2e_a_claude_bound_plan_starts_and_an_edit_onto_codex_is_refused() {
    let h = RunHarness::with_repo(
        CLAUDE_BOUND,
        &[("ANTHREX_TEST_CODEX_PROJECT_CONFIG", "load")],
        true,
        CODEX_CONFIG,
    );
    let claude = plan(
        "",
        &[task(
            "t1",
            &["a.txt"],
            "route = { runtime = \"claude\", model = \"claude-sonnet-5\" }",
        )],
    );
    let id = h.start_in(&h.repo, &claude, false, false);
    assert!(h.run(&id).unwrap().trusted_project.is_empty());

    let on_codex: proto::PlanTask = serde_json::from_value(json!({
        "id": "t2", "title": "Task t2", "size": "S", "test_mode": "check",
        "test_mode_reason": "smoke", "owns": ["b.txt"], "brief": "Do t2",
        "acceptance": ["t2 is done"],
        "route": {"runtime": "codex", "model": "gpt-5-codex-mini"},
    }))
    .unwrap();
    let reply = h.request(proto::RunRequest::Edit {
        run_id: id.clone(),
        edits: vec![proto::PlanEdit::AddTask { task: on_codex }],
    });
    match reply {
        proto::RunReply::Refused { message, .. } => {
            assert_eq!(message, settings_refusal("Codex", ".codex/config.toml"));
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(h.run(&id).unwrap().tasks.len(), 1);
}

/// The brief's other case: a Codex plan on a Codex-only roster starts in a repository
/// tracking a hooked `.claude/settings.json`, since no session of it runs Claude.
#[test]
fn e2e_a_codex_bound_plan_starts_beside_claude_settings() {
    let h = RunHarness::with_repo(
        CODEX_ONLY,
        &[("ANTHREX_TEST_NO_SETTING_SOURCES", "1")],
        true,
        &[(".claude/settings.json", HOOKED_SETTINGS)],
    );
    let codex = plan(
        "",
        &[task(
            "t1",
            &["a.txt"],
            "route = { runtime = \"codex\", model = \"gpt-5-codex\" }",
        )],
    );
    let id = h.start_in(&h.repo, &codex, false, false);
    assert!(h.run(&id).unwrap().trusted_project.is_empty());
}
