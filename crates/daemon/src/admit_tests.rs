//! Milestone 9.14 tasks M9.14.7–M9.14.9: the hook group, release, the hold book and
//! the interactive opt-in (decisions 17, 20, 22 and 23; pure). The matcher's tests are in
//! `admit_matcher_tests.rs`.

use super::*;
use proto::HookSource;
use serde_json::json;

#[test]
fn the_hook_command_and_group_are_exact() {
    let exe = Path::new("/opt/anthrex/bin/anthrex");
    let hook = AdmitHook {
        wait_secs: 300,
        slots: Some(2),
        prefixes: vec!["make test".into()],
    };
    let command = hook_command(exe, 7, &hook);
    assert_eq!(
        command,
        "'/opt/anthrex/bin/anthrex' admit --window 7 --wait-secs 300 --slots 2 --prefix 'make test'"
    );
    let half = AdmitHook {
        wait_secs: 120,
        slots: None,
        prefixes: Vec::new(),
    };
    assert_eq!(
        hook_command(exe, 3, &half),
        "'/opt/anthrex/bin/anthrex' admit --window 3 --wait-secs 120"
    );

    let filter = json!({"matcher": "Bash", "hooks": [{"type": "command", "command": "f"}]});
    let before = json!({
        "permissions": {"allow": ["Bash"]},
        "hooks": {
            "PreToolUse": [{"matcher": "*", "hooks": []}, filter],
            "Stop": [{"hooks": []}],
        },
    });
    let mut settings = before.clone();
    add_hook(&mut settings, exe, 7, Some(&hook));
    let mut expected = before.clone();
    expected["hooks"]["PreToolUse"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "matcher": "Bash",
            "hooks": [{"type": "command", "command": command, "timeout": 330}],
        }));
    assert_eq!(settings, expected);

    let mut untouched = before.clone();
    add_hook(&mut untouched, exe, 7, None);
    assert_eq!(untouched, before);
}

fn parsed(payload: serde_json::Value) -> ParsedHook {
    crate::hooks::parse(HookSource::Claude, &payload).expect("a hook payload")
}

fn event(name: &str, id: Option<&str>) -> ParsedHook {
    let mut payload = json!({"hook_event_name": name, "session_id": "s"});
    if let Some(id) = id {
        payload["tool_use_id"] = json!(id);
    }
    parsed(payload)
}

#[test]
fn release_for_each_event() {
    assert_eq!(
        release_for(&event("PostToolUse", Some("x"))),
        Release::One("x".into())
    );
    assert_eq!(release_for(&event("PostToolUse", None)), Release::Nothing);
    assert_eq!(
        release_for(&event("PreToolUse", Some("y"))),
        Release::AllBut("y".into())
    );
    assert_eq!(release_for(&event("PreToolUse", None)), Release::Nothing);
    for name in ["Stop", "StopFailure", "UserPromptSubmit", "SessionEnd"] {
        assert_eq!(release_for(&event(name, None)), Release::Window, "{name}");
    }
    assert_eq!(release_for(&event("Notification", None)), Release::Nothing);
}

/// A book with `holds`, each granted `at`; a hold's grant is its tool use id.
fn book_of(holds: &[(u32, &str)], at: Instant) -> HoldBook<String> {
    let mut book = HoldBook::default();
    for (window, id) in holds {
        book.insert(*window, id.to_string(), id.to_string(), at);
    }
    book
}

#[test]
fn a_newer_pre_tool_use_releases_the_older_holds() {
    let at = Instant::now();
    let mut book = book_of(&[(1, "a"), (1, "b"), (2, "c")], at);
    assert_eq!(book.release(1, &Release::AllBut("b".into())), ["a"]);
    assert!(book.get(1, "b").is_some());
    assert!(book.get(2, "c").is_some());
    assert_eq!(book.release(1, &Release::AllBut("z".into())), ["b"]);
    assert!(book.get(2, "c").is_some());
}

#[test]
fn a_post_tool_use_releases_the_hold() {
    let at = Instant::now();
    let mut book: HoldBook<u32> = HoldBook::default();
    book.insert(1, "a".into(), 10, at);
    book.insert(1, "b".into(), 20, at);
    assert_eq!(book.release(1, &Release::One("a".into())), [10]);
    assert_eq!(book.get(1, "a"), None);
    assert_eq!(book.get(1, "b"), Some(&20));
    assert_eq!(
        book.release(2, &Release::One("b".into())),
        Vec::<u32>::new()
    );
    assert_eq!(book.release(1, &Release::Nothing), Vec::<u32>::new());
}

#[test]
fn stop_releases_every_hold_of_the_window() {
    let at = Instant::now();
    let mut book = book_of(&[(1, "a"), (1, "b"), (2, "c")], at);
    let mut released = book.release(1, &Release::Window);
    released.sort();
    assert_eq!(released, ["a", "b"]);
    assert!(book.get(2, "c").is_some());
}

#[test]
fn the_sweep_releases_old_holds_and_holds_of_gone_windows() {
    let start = Instant::now();
    let max_age = Duration::from_secs(630);
    let mut book = book_of(&[(1, "old")], start);
    let fresh = start + Duration::from_secs(100);
    book.insert(1, "fresh".into(), "fresh".into(), fresh);
    book.insert(2, "gone".into(), "gone".into(), fresh);
    let now = start + max_age + Duration::from_secs(1);
    let live = |window: u32| window == 1;
    let mut released = book.sweep(now, max_age, &live);
    released.sort();
    assert_eq!(released, ["gone", "old"]);
    assert!(book.get(1, "fresh").is_some());
    // Exactly the max age is not older than it.
    let mut edge = book_of(&[(1, "edge")], start);
    assert!(edge.sweep(start + max_age, max_age, &|_| true).is_empty());
}

#[test]
fn a_repeated_ask_is_the_same_hold() {
    let at = Instant::now();
    let mut book: HoldBook<u32> = HoldBook::default();
    assert_eq!(book.get(1, "a"), None);
    book.insert(1, "a".into(), 3, at);
    // The caller checks `get` before a second insert (decision 23): the held grant.
    assert_eq!(book.get(1, "a"), Some(&3));
    assert_eq!(book.get(2, "a"), None);
}

/// A Claude worker's spec, with an optional output filter and admission hook.
fn worker_spec(
    filter: Option<crate::output_filter::FilterHook>,
    admit: Option<AdmitHook>,
) -> crate::headless::HeadlessSpec {
    crate::headless::HeadlessSpec {
        runtime: proto::Runtime::Claude,
        model: "claude-sonnet-5".into(),
        effort: proto::Effort::HIGH,
        cwd: "/tmp/p/.anthrex/wt/t1".into(),
        instructions: "Do it.".into(),
        mcp: None,
        allowed_tools: vec!["Bash".into()],
        claude_permission_mode: Some("acceptEdits".into()),
        claude_disallowed_tools: Vec::new(),
        claude_sandbox: None,
        codex_sandbox: "workspace-write".into(),
        codex_writable_roots: Vec::new(),
        codex_read_only: Vec::new(),
        codex_grant_dialect: None,
        env: Vec::new(),
        claude_auth: config::ClaudeAuth::Login,
        api_key_helper: None,
        run_ref: None,
        codex_config_guard: None,
        output_filter: filter,
        admit,
    }
}

/// The `PreToolUse` groups of the `--settings` that `claude_args` passes for `spec`.
fn pre_tool_use_groups(spec: &crate::headless::HeadlessSpec) -> Vec<serde_json::Value> {
    let args = crate::headless::argv::claude_args(
        spec,
        &crate::headless::SessionArg::New { uuid: None },
        Path::new("/opt/anthrex/bin/anthrex"),
        7,
        Path::new("/tmp/a.sock"),
        &crate::headless::argv::CLI_CAPS,
    );
    let at = args.iter().position(|a| a == "--settings").unwrap();
    let settings: serde_json::Value = serde_json::from_str(&args[at + 1]).unwrap();
    settings["hooks"]["PreToolUse"].as_array().unwrap().clone()
}

/// M9.14.8: a worker's settings carry the status group, the filter group and the
/// admission group, in that order; without a filter, the admission group is second.
#[test]
fn the_worker_settings_carry_three_pre_tool_use_groups() {
    let filter = crate::output_filter::FilterHook {
        mode: proto::OutputFilter::FailuresOnly,
        prefixes: vec!["cargo test".into()],
        log_dir: "/tmp/ax/t1/anthrex-logs".into(),
    };
    let admit = AdmitHook {
        wait_secs: 300,
        slots: Some(2),
        prefixes: vec!["cargo test".into()],
    };
    let groups = pre_tool_use_groups(&worker_spec(Some(filter), Some(admit.clone())));
    assert_eq!(groups.len(), 3, "{groups:?}");
    assert_eq!(groups[0]["matcher"], "");
    assert!(
        groups[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains(" hook --window 7 --source claude"),
        "{groups:?}"
    );
    assert_eq!(groups[1]["matcher"], "Bash");
    assert!(
        groups[1]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains(" filter-hook "),
        "{groups:?}"
    );
    let admission = json!({
        "matcher": "Bash",
        "hooks": [{
            "type": "command",
            "command": "'/opt/anthrex/bin/anthrex' admit --window 7 --wait-secs 300 --slots 2 --prefix 'cargo test'",
            "timeout": 330,
        }],
    });
    assert_eq!(groups[2], admission);

    let groups = pre_tool_use_groups(&worker_spec(None, Some(admit)));
    assert_eq!(groups.len(), 2, "{groups:?}");
    assert_eq!(groups[0]["matcher"], "");
    assert_eq!(groups[1], admission);

    // Without admission, the settings are what they were.
    assert_eq!(pre_tool_use_groups(&worker_spec(None, None)).len(), 1);
}

/// The `--settings` that `launch::plan` gives an interactive Claude window (decision 17).
fn interactive_settings(
    role: Option<&crate::launch::role::RoleLaunch>,
    admit_interactive: bool,
    admit_wait_secs: u64,
) -> serde_json::Value {
    let spec = proto::WindowSpec {
        name: Some("api".into()),
        runtime: proto::Runtime::Claude,
        cwd: "/tmp/repo".into(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let ctx = crate::launch::LaunchContext {
        window_id: 4,
        name: "api",
        socket_path: Path::new("/tmp/a.sock"),
        shell: "/bin/zsh",
        exe: Path::new("/opt/anthrex/bin/anthrex"),
        claude_bin: "/opt/agents/claude",
        codex_bin: "/opt/agents/codex",
        codex_hook_source: None,
        codex_bypass_hook_trust: false,
        resume: None,
        caps: &crate::headless::argv::CLI_CAPS,
        role,
        admit_interactive,
        admit_wait_secs,
    };
    let args = crate::launch::plan(&spec, &ctx).args;
    let at = args.iter().position(|a| a == "--settings").unwrap();
    serde_json::from_str(&args[at + 1]).unwrap()
}

fn plain_settings() -> serde_json::Value {
    crate::launch::claude::settings(Path::new("/opt/anthrex/bin/anthrex"), 4)
}

/// Decision 17 (C7, C22): a user's Claude window gets the admission group only with
/// `[testing] admit_interactive = true` and a daemon-wide wait above 0; it asks
/// `Want::Half` (no `--slots`) and its `timeout` is W + `ADMIT_HOOK_SLACK_SECS`.
#[test]
fn an_interactive_window_gets_the_group_only_when_opted_in() {
    assert_eq!(interactive_settings(None, false, 300), plain_settings());
    assert_eq!(interactive_settings(None, true, 0), plain_settings());

    let settings = interactive_settings(None, true, 300);
    let groups = settings["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(groups.len(), 2, "{groups:?}");
    assert_eq!(groups[0], plain_settings()["hooks"]["PreToolUse"][0]);
    let admission = json!({
        "matcher": "Bash",
        "hooks": [{
            "type": "command",
            "command": "'/opt/anthrex/bin/anthrex' admit --window 4 --wait-secs 300",
            "timeout": 330,
        }],
    });
    assert_eq!(groups[1], admission);
    // Everything but the admission group is the plain settings.
    let mut without = settings.clone();
    without["hooks"]["PreToolUse"].as_array_mut().unwrap().pop();
    assert_eq!(without, plain_settings());

    // The daemon caps the wait at `ADMIT_WAIT_MAX_SECS`.
    let capped = interactive_settings(None, true, config::ADMIT_WAIT_MAX_SECS + 100);
    let command = capped["hooks"]["PreToolUse"][1]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_string();
    let max = config::ADMIT_WAIT_MAX_SECS;
    assert!(
        command.ends_with(&format!("--wait-secs {max}")),
        "{command}"
    );
}

/// Decision 17 (C7): `launch::plan` builds the orchestrator's settings as a user
/// window's, and the orchestrator never gets the admission group.
#[test]
fn the_orchestrator_never_gets_the_group() {
    let role = crate::launch::role::RoleLaunch {
        run_ref: proto::RunRef {
            run_id: "r-7a2c".into(),
            task_id: None,
            role: proto::AgentRole::Orchestrator,
            session: 1,
            lane: None,
        },
        mcp: crate::headless::McpTarget {
            role: proto::AgentRole::Orchestrator,
            run_id: "r-7a2c".into(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
            agent_label: None,
        },
        instructions: "THE CONTRACT".into(),
        effort: proto::Effort::HIGH,
        claude_allowed_tools: vec!["Read".into()],
        claude_disallowed_tools: vec![],
        env: vec![],
        remove_env: vec![],
    };
    assert_eq!(
        interactive_settings(Some(&role), true, 300),
        plain_settings()
    );
}
