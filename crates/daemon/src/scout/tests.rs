//! M8b.9: the scouts' pure parts (decisions 12 and 13; rulings R-T1-1, R-T8-1). The
//! machine's tests are in `tests_machine.rs`.

use std::path::{Path, PathBuf};

use proto::{AgentRole, Effort, RunRef, Runtime, ScoutKind, Strength};
use serde_json::{Value, json};

use super::report::{report_path, resolve_ref, validate};
use super::spec::{ScoutContext, ScoutSpec, headless_spec, route, valid_id};
use crate::headless::argv::{CLI_CAPS, CODEX_SANDBOX_PINS, CliCaps, claude_args, codex_args};
use crate::headless::codex_guard::{CodexConfigGuard, EntryKind, GuardEntry, ObjectFormat};
use crate::headless::{ClaudeSandbox, McpTarget, SessionArg};

const BASE: &str = "0123456789abcdef0123456789abcdef01234567";

fn ctx(runtime: Runtime) -> ScoutContext {
    ScoutContext {
        roster: config::default_roster().into(),
        default_runtime: runtime,
        scouts: config::Scouts::default(),
        claude: config::ClaudeHeadless::default(),
        caps: CLI_CAPS,
        data_dir: PathBuf::from("/data"),
    }
}

fn scout(kind: ScoutKind) -> ScoutSpec {
    let (id, run_id, cwd) = match kind {
        ScoutKind::Area => ("api-1", Some("r1".to_string()), "/wt/runs/r1/integration"),
        ScoutKind::Onboarding => ("onboarding-1", None, "/wt/runs/.onboarding"),
    };
    ScoutSpec {
        id: id.into(),
        kind,
        run_id,
        question: "How is it built?".into(),
        first_turn: "go".into(),
        cwd: cwd.into(),
        project: "/repo".into(),
        web: false,
        codex_config: vec![GuardEntry {
            path: ".codex/config.toml".into(),
            kind: EntryKind::File,
            oid: "aa".repeat(20),
        }],
        base_sha: BASE.into(),
        repo_paths: vec![
            "/data/repos/repo-1/tasks/.onboarding".into(),
            "/repo/.git/objects".into(),
        ],
    }
}

/// Ruling R-T1-1: the protected paths, then the checkout, the project and every other
/// repository path the scout can see.
fn denials(cwd: &str) -> Vec<PathBuf> {
    let cwd = Path::new(cwd);
    let mut denied: Vec<PathBuf> = [".claude", ".codex", ".mcp.json", "CLAUDE.md", "AGENTS.md"]
        .iter()
        .map(|p| cwd.join(p))
        .collect();
    denied.extend([
        cwd.to_path_buf(),
        "/repo".into(),
        "/data/repos/repo-1/tasks/.onboarding".into(),
        "/repo/.git/objects".into(),
    ]);
    denied
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn area_scout_spec_is_read_only() {
    let spec = headless_spec(&scout(ScoutKind::Area), &ctx(Runtime::Claude));
    assert_eq!(spec.runtime, Runtime::Claude);
    assert_eq!(spec.model, "claude-haiku-4-5");
    assert_eq!(spec.effort, Effort::Low);
    assert_eq!(spec.cwd, PathBuf::from("/wt/runs/r1/integration"));
    assert_eq!(spec.instructions, super::contract::SCOUT_CONTRACT);
    assert_eq!(
        spec.mcp,
        Some(McpTarget {
            role: AgentRole::Scout,
            run_id: "r1".into(),
            task_id: None,
            scout_id: Some("api-1".into()),
            epic: None,
            chain: None,
            lane: None,
        })
    );
    assert_eq!(
        spec.allowed_tools,
        strings(&["mcp__anthrex__submit_scout_report", "Read", "Glob", "Grep"])
    );
    assert_eq!(spec.claude_permission_mode.as_deref(), Some("dontAsk"));
    assert_eq!(
        spec.claude_disallowed_tools,
        strings(&["Edit", "Write", "NotebookEdit"])
    );
    assert_eq!(
        spec.claude_sandbox,
        Some(ClaudeSandbox {
            writable_roots: Vec::new(),
            deny_write: denials("/wt/runs/r1/integration"),
        })
    );
    assert_eq!(spec.codex_sandbox, "read-only");
    assert!(spec.codex_writable_roots.is_empty());
    assert!(spec.env.is_empty());
    assert_eq!(spec.claude_auth, config::ClaudeAuth::Login);
    assert_eq!(spec.api_key_helper, None);
    assert_eq!(
        spec.run_ref,
        Some(RunRef {
            run_id: "r1".into(),
            task_id: None,
            role: AgentRole::Scout,
            session: 1,
            lane: None,
        })
    );
    assert_eq!(spec.codex_config_guard, None);
    // Ruling R-T8-1: no output filter for a scout.
    assert_eq!(spec.output_filter, None);

    let mut web = scout(ScoutKind::Area);
    web.web = true;
    let spec = headless_spec(&web, &ctx(Runtime::Claude));
    assert_eq!(
        spec.allowed_tools,
        strings(&[
            "mcp__anthrex__submit_scout_report",
            "Read",
            "Glob",
            "Grep",
            "WebFetch",
            "WebSearch"
        ])
    );
}

#[test]
fn onboarding_scout_spec_is_read_only_too() {
    let spec = headless_spec(&scout(ScoutKind::Onboarding), &ctx(Runtime::Claude));
    assert_eq!(spec.instructions, super::contract::ONBOARDING_CONTRACT);
    assert_eq!(
        spec.allowed_tools,
        strings(&[
            "mcp__anthrex__submit_scout_report",
            "Bash",
            "Read",
            "Glob",
            "Grep"
        ])
    );
    for write in ["Edit", "Write", "NotebookEdit"] {
        assert!(!spec.allowed_tools.iter().any(|t| t == write));
        assert!(spec.claude_disallowed_tools.iter().any(|t| t == write));
    }
    assert_eq!(spec.claude_permission_mode.as_deref(), Some("dontAsk"));
    assert_eq!(
        spec.claude_sandbox,
        Some(ClaudeSandbox {
            writable_roots: Vec::new(),
            deny_write: denials("/wt/runs/.onboarding"),
        })
    );
    // Ruling R-T1-1: the checkout root itself is denied.
    let sandbox = spec.claude_sandbox.as_ref().unwrap();
    assert!(
        sandbox
            .deny_write
            .contains(&PathBuf::from("/wt/runs/.onboarding"))
    );
    assert_eq!(spec.run_ref, None);
    assert_eq!(spec.codex_sandbox, "read-only");
    assert_eq!(
        spec.mcp,
        Some(McpTarget {
            role: AgentRole::Scout,
            run_id: String::new(),
            task_id: None,
            scout_id: Some("onboarding-1".into()),
            epic: None,
            chain: None,
            lane: None,
        })
    );
    assert_eq!(spec.output_filter, None);

    let codex = headless_spec(&scout(ScoutKind::Onboarding), &ctx(Runtime::Codex));
    assert_eq!(codex.runtime, Runtime::Codex);
    assert_eq!(codex.codex_sandbox, "read-only");
    assert!(codex.codex_writable_roots.is_empty());
    assert_eq!(codex.run_ref, None);
    assert_eq!(codex.output_filter, None);
}

#[test]
fn the_scout_sandbox_does_not_depend_on_worker_sandbox() {
    // `[orchestrator] worker_sandbox` is not an input of a scout's spec at all: a
    // context built from a config with it off gives the same spec.
    let off = config::Orchestrator {
        worker_sandbox: false,
        ..config::Orchestrator::default()
    };
    let context = ScoutContext {
        roster: off.models.clone().into(),
        default_runtime: off.default_runtime,
        scouts: off.scouts.clone(),
        claude: off.claude.clone(),
        caps: CLI_CAPS,
        data_dir: PathBuf::from("/data"),
    };
    for kind in [ScoutKind::Area, ScoutKind::Onboarding] {
        let spec = headless_spec(&scout(kind), &context);
        assert_eq!(spec, headless_spec(&scout(kind), &ctx(Runtime::Claude)));
        let sandbox = spec.claude_sandbox.expect("always sandboxed");
        assert!(sandbox.writable_roots.is_empty());
    }
}

fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let at = args.iter().position(|a| a == flag)?;
    args.get(at + 1).map(String::as_str)
}

#[test]
fn scouts_pass_the_user_settings_flags_and_codex_pins() {
    let (exe, socket) = (Path::new("/bin/anthrex"), Path::new("/tmp/a.sock"));
    let session = SessionArg::New { uuid: None };
    let spec = headless_spec(&scout(ScoutKind::Onboarding), &ctx(Runtime::Claude));
    let args = claude_args(&spec, &session, exe, 7, socket, &CLI_CAPS);
    let flags = CLI_CAPS.claude_user_settings_only.unwrap();
    assert!(
        args.windows(flags.len()).any(|w| w == flags),
        "{args:?} lacks {flags:?}"
    );
    let settings: Value = serde_json::from_str(after(&args, "--settings").unwrap()).unwrap();
    let sandbox = &settings["sandbox"];
    assert_eq!(sandbox["enabled"], json!(true));
    assert_eq!(sandbox["filesystem"]["allowWrite"], json!([]));
    assert_eq!(
        sandbox["filesystem"]["denyWrite"],
        json!(
            denials("/wt/runs/.onboarding")
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
        )
    );
    assert_eq!(sandbox["network"]["allowedDomains"], json!([]));
    assert_eq!(sandbox["network"]["allowLocalBinding"], json!(false));
    assert_eq!(after(&args, "--permission-mode"), Some("dontAsk"));
    assert_eq!(
        after(&args, "--disallowedTools"),
        Some("Edit,Write,NotebookEdit")
    );
    // Ruling R-T8-1: one `PreToolUse` group only, M3's.
    assert_eq!(settings["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);

    let codex = headless_spec(&scout(ScoutKind::Onboarding), &ctx(Runtime::Codex));
    for flag in [Some(&["--flag"][..]), None] {
        let caps = CliCaps {
            codex_user_config_only: flag,
            ..CLI_CAPS
        };
        let args = codex_args(&codex, &session, "go", exe, 7, socket, &caps);
        assert_eq!(after(&args, "-s"), Some("read-only"));
        for pin in CODEX_SANDBOX_PINS {
            assert!(args.iter().any(|a| a == pin), "{args:?} lacks {pin}");
        }
        let decider = crate::decider::argv::codex_decider_args(
            &decider_ctx(caps),
            &crate::decider::argv::DECIDER_CAPS,
            Path::new("/s.json"),
            "p",
        );
        let has = |args: &[String]| args.iter().any(|a| a == "--flag");
        assert_eq!(has(&args), flag.is_some(), "scout {args:?}");
        assert_eq!(has(&decider), flag.is_some(), "decider {decider:?}");
    }
}

fn decider_ctx(caps: CliCaps) -> crate::decider::DeciderContext {
    crate::decider::DeciderContext {
        mode: proto::DeciderMode::Codex,
        program: "codex".into(),
        route: proto::Route {
            runtime: Runtime::Codex,
            model: String::new(),
            strength: Strength::Standard,
            effort: Effort::Low,
        },
        timeout: std::time::Duration::from_secs(90),
        cwd: PathBuf::from("/data/deciders/cwd"),
        schema_dir: PathBuf::from("/data/deciders/schemas"),
        caps,
    }
}

#[test]
fn a_codex_scout_carries_the_codex_config_guard_when_codex_loads_project_config() {
    let loaded = headless_spec(&scout(ScoutKind::Onboarding), &ctx(Runtime::Codex));
    assert_eq!(
        loaded.codex_config_guard,
        Some(CodexConfigGuard {
            format: ObjectFormat::Sha1,
            entries: scout(ScoutKind::Onboarding).codex_config,
        })
    );
    let mut excluded = ctx(Runtime::Codex);
    excluded.caps.codex_user_config_only = Some(&["--flag"]);
    let spec = headless_spec(&scout(ScoutKind::Onboarding), &excluded);
    assert_eq!(spec.codex_config_guard, None);
    let mut not_loaded = ctx(Runtime::Codex);
    not_loaded.caps.codex_loads_project_config = false;
    let spec = headless_spec(&scout(ScoutKind::Onboarding), &not_loaded);
    assert_eq!(spec.codex_config_guard, None);
    let claude = headless_spec(&scout(ScoutKind::Onboarding), &ctx(Runtime::Claude));
    assert_eq!(claude.codex_config_guard, None);
}

#[test]
fn route_picks_the_lowest_strength_at_or_above() {
    let roster = config::default_roster();
    let fast = route(&roster, Runtime::Claude, Strength::Fast, Effort::Low);
    assert_eq!(
        (
            fast.runtime,
            fast.model.as_str(),
            fast.strength,
            fast.effort
        ),
        (
            Runtime::Claude,
            "claude-haiku-4-5",
            Strength::Fast,
            Effort::Low
        )
    );
    let standard = route(&roster, Runtime::Claude, Strength::Standard, Effort::High);
    assert_eq!(standard.model, "claude-sonnet-5");
    assert_eq!(standard.effort, Effort::High);
    let codex = route(&roster, Runtime::Codex, Strength::Fast, Effort::Low);
    assert_eq!(
        (codex.runtime, codex.model.as_str(), codex.strength),
        (Runtime::Codex, "", Strength::Standard)
    );
    // Nothing on Codex at or above frontier: the peer's frontier entry.
    let peer = route(&roster, Runtime::Codex, Strength::Frontier, Effort::Low);
    assert_eq!(
        (peer.runtime, peer.model.as_str()),
        (Runtime::Claude, "claude-opus-5-5")
    );
    // Nothing at or above on either runtime: the runtime's first entry.
    let only_fast = vec![roster[0].clone(), roster[3].clone()];
    let first = route(&only_fast, Runtime::Claude, Strength::Frontier, Effort::Low);
    assert_eq!(
        (first.runtime, first.model.as_str()),
        (Runtime::Claude, "claude-haiku-4-5")
    );
    // An empty roster: the runtime with no model.
    let none = route(&[], Runtime::Codex, Strength::Fast, Effort::Low);
    assert_eq!(
        (none.runtime, none.model.as_str(), none.strength),
        (Runtime::Codex, "", Strength::Fast)
    );
}

#[test]
fn valid_ids() {
    for id in ["onboarding-1695000000", "a", "0", "api-2", &"a".repeat(48)] {
        assert!(valid_id(id), "{id}");
    }
    for id in ["", "-a", "A", "api_2", "a b", "a/b", "é", &"a".repeat(49)] {
        assert!(!valid_id(id), "{id}");
    }
}

fn report(extra: Value) -> Value {
    let mut base = json!({
        "summary": "a Rust workspace",
        "files": [{"path": "Cargo.toml", "why": "the workspace"}],
    });
    for (k, v) in extra.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

#[test]
fn report_validation_cases() {
    let profile = json!({"languages": ["rust"], "check": "cargo test"});
    let files: Vec<Value> = (0..61)
        .map(|i| json!({"path": format!("f{i}"), "why": "w"}))
        .collect();
    let cases = [
        (
            json!({"files": []}),
            ScoutKind::Area,
            "invalid arguments: summary: missing",
        ),
        (
            report(json!({"files": files})),
            ScoutKind::Area,
            "invalid arguments: files: must have at most 60 items",
        ),
        (
            report(json!({"profile": profile})),
            ScoutKind::Area,
            "invalid arguments: profile: only the onboarding scout reports a profile",
        ),
        (
            report(json!({})),
            ScoutKind::Onboarding,
            "invalid arguments: profile: required for the onboarding scout",
        ),
        (
            report(json!({"profile": {"env": {"1BAD": "x"}}})),
            ScoutKind::Onboarding,
            "invalid arguments: profile.env: key 1BAD must match ^[A-Za-z_][A-Za-z0-9_]*$",
        ),
        (
            report(json!({"profile": {"env": {"K".repeat(65): "x"}}})),
            ScoutKind::Onboarding,
            "invalid arguments: profile.env: a key is longer than 64 characters",
        ),
        (
            report(json!({"profile": {"env": {"V": "x".repeat(1001)}}})),
            ScoutKind::Onboarding,
            "invalid arguments: profile.env.V: must be at most 1000 characters",
        ),
        (
            report(
                json!({"profile": {"env": (0..21).map(|i| (format!("K{i}"), json!("v"))).collect::<serde_json::Map<_, _>>()}}),
            ),
            ScoutKind::Onboarding,
            "invalid arguments: profile.env: must have at most 20 properties",
        ),
        (
            report(json!({"profile": {"cache_dirs": ["/tmp"]}})),
            ScoutKind::Onboarding,
            "invalid arguments: profile.cache_dirs: not in the schema",
        ),
        (
            report(json!({"profile": {"confined_network": ["example.com"]}})),
            ScoutKind::Onboarding,
            "invalid arguments: profile.confined_network: not in the schema",
        ),
        (
            report(json!({"summary": ""})),
            ScoutKind::Area,
            "invalid arguments: summary: must not be empty",
        ),
        (
            report(json!({"summary": "x".repeat(8001)})),
            ScoutKind::Area,
            "invalid arguments: summary: must be at most 8000 characters",
        ),
        (
            report(json!({"files": [{"path": "a"}]})),
            ScoutKind::Area,
            "invalid arguments: files[0].why: missing",
        ),
        (
            report(json!({"extra": 1})),
            ScoutKind::Area,
            "invalid arguments: extra: not in the schema",
        ),
        (
            report(json!({"profile": {"check_timeout_secs": 5}})),
            ScoutKind::Onboarding,
            "invalid arguments: profile.check_timeout_secs: must be between 10 and 14400",
        ),
        (
            report(json!({"profile": {"output_filter": "all"}})),
            ScoutKind::Onboarding,
            "invalid arguments: profile.output_filter: expected failures-only, tail or none",
        ),
    ];
    for (args, kind, message) in cases {
        assert_eq!(validate(&args, kind), Err(message.to_string()), "{args}");
    }

    let ok = validate(
        &report(json!({
            "modules": ["crates/*"],
            "risks": ["no CI"],
            "profile": {
                "languages": ["rust"], "check": "cargo test", "check_timeout_secs": 600,
                "output_filter": "tail", "env": {"RUST_LOG": "", "_X": "{worktree}", "K".repeat(64): "x".repeat(1000)},
            },
        })),
        ScoutKind::Onboarding,
    )
    .unwrap();
    assert_eq!(ok.summary, "a Rust workspace");
    assert_eq!(ok.files[0].path, "Cargo.toml");
    assert_eq!(ok.modules, ["crates/*"]);
    assert_eq!(ok.risks, ["no CI"]);
    let profile = ok.profile.unwrap();
    assert_eq!(profile.check.as_deref(), Some("cargo test"));
    assert_eq!(profile.check_timeout_secs, Some(600));
    assert_eq!(profile.output_filter, proto::OutputFilter::Tail);
    assert_eq!(profile.env["_X"], "{worktree}");
    let area = validate(&report(json!({})), ScoutKind::Area).unwrap();
    assert_eq!(area.profile, None);
}

#[test]
fn reports_are_stored_by_scope_and_the_onboarding_alias_resolves() {
    let (repo, run) = (Path::new("/data/repos/r-1"), Path::new("/data/runs/r1"));
    assert_eq!(
        report_path(repo, None, "onboarding-1"),
        PathBuf::from("/data/repos/r-1/scouts/onboarding-1.json")
    );
    assert_eq!(
        report_path(repo, Some(run), "api-1"),
        PathBuf::from("/data/runs/r1/scouts/api-1.json")
    );
    assert_eq!(
        resolve_ref("onboarding", run, repo, Some("onboarding-9")),
        PathBuf::from("/data/repos/r-1/scouts/onboarding-9.json")
    );
    assert_eq!(
        resolve_ref("api-1", run, repo, Some("onboarding-9")),
        PathBuf::from("/data/runs/r1/scouts/api-1.json")
    );
}

#[test]
fn contract_texts() {
    use super::contract::{onboarding_first_turn, scout_wrap_up};
    let names: Vec<String> = (0..70).map(|i| format!("e{i}")).collect();
    let turn = onboarding_first_turn(Path::new("/repo"), Path::new("/wt/.onboarding"), 12, &names);
    let listed: Vec<String> = names[..60].to_vec();
    assert_eq!(
        turn,
        format!(
            "[anthrex] Work out this repository's profile. Repository: /repo. Disposable copy: /wt/.onboarding. It tracks 12 files; the top-level entries are: {}.",
            listed.join(", ")
        )
    );
    assert_eq!(
        scout_wrap_up(11),
        "[anthrex] You have used 11 tool calls. Stop exploring and call submit_scout_report now with what you found."
    );
    assert_eq!(
        super::contract::SCOUT_NUDGE,
        "[anthrex] Your turn ended without a report. Call submit_scout_report now with what you found, then stop."
    );
}

/// The Claude tool-search fix (2026-09-27): a run's area scout and the onboarding scout
/// get `ENABLE_TOOL_SEARCH=false` exactly once on Claude, on top of their spec's `env`
/// (the variables `HeadlessHandle::spawn` sets), and not on Codex.
#[test]
fn claude_scouts_get_tool_search_off_exactly_once() {
    for kind in [ScoutKind::Area, ScoutKind::Onboarding] {
        for (runtime, expected) in [(Runtime::Claude, 1), (Runtime::Codex, 0)] {
            let spec = headless_spec(&scout(kind), &ctx(runtime));
            assert_eq!(spec.runtime, runtime);
            let vars = crate::headless::session_vars(spec.runtime, &spec.env);
            let pins: Vec<_> = vars
                .iter()
                .filter(|(key, _)| key == "ENABLE_TOOL_SEARCH")
                .collect();
            assert_eq!(pins.len(), expected, "{kind:?} on {runtime:?}: {vars:?}");
            assert!(pins.iter().all(|(_, value)| value == "false"), "{vars:?}");
        }
    }
}

/// The Claude tool-search fix (2026-09-27): both scout contracts name the report tool by
/// the full id a Claude session sees, marked as Claude's (Codex names it differently).
#[test]
fn scout_contracts_name_the_report_tool_by_its_claude_id() {
    let named = "submit_scout_report (in Claude: mcp__anthrex__submit_scout_report)";
    for contract in [
        super::contract::SCOUT_CONTRACT,
        super::contract::ONBOARDING_CONTRACT,
    ] {
        assert!(contract.contains(named), "{contract}");
    }
}
