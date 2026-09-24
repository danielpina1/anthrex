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

    let codex = plan("", &[task("t1", &["a.txt"], CODEX)]);
    h.start_in(&h.repo, &codex, false, false);
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
    let text = report(&run);
    assert!(
        text.contains("project settings trusted by --trust-project: .claude/settings.json"),
        "{text}"
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
fn codex_excludes(h: &RunHarness, flags: &[&str]) {
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
    h.wait_run(&id, complete, RUN_WAIT);
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
    // Claude-only plan is not refused for it.
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
    assert!(
        report(&run).contains("project settings trusted by --trust-project: .codex/config.toml")
    );
    h.start(&plan("", &[task("t9", &["z.txt"], "")]), false);
    drop(h);

    // `exclude`: the placeholder flag on both turns' argv.
    let h = RunHarness::with_repo(
        "",
        &[("ANTHREX_TEST_CODEX_PROJECT_CONFIG", "exclude")],
        true,
        CODEX_CONFIG,
    );
    codex_excludes(&h, &[EXCLUDE_FLAG]);
    drop(h);

    // Unset: whichever branch the real CLI_CAPS names.
    let caps = daemon::headless::argv::CLI_CAPS;
    let h = RunHarness::with_repo("", &[], true, CODEX_CONFIG);
    match (caps.codex_loads_project_config, caps.codex_user_config_only) {
        (true, None) => codex_refuses(&h),
        (true, Some(flags)) => codex_excludes(&h, flags),
        (false, _) => codex_excludes(&h, &[]),
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
