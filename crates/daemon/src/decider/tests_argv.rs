//! Decision 16's argv and credential scrub for deciders.

use super::argv::{DECIDER_CAPS, claude_decider_args, codex_decider_args, schema_file_name};
use super::schema::schema;
use super::*;
use crate::headless::argv::{CLI_CAPS, CODEX_SANDBOX_PINS, CliCaps};
use crate::headless::credential_scrub_for;
use proto::{DeciderMode, Effort, Route, Runtime, Strength};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn ctx(runtime: Runtime, model: &str, caps: CliCaps) -> DeciderContext {
    DeciderContext {
        mode: DeciderMode::Claude,
        program: "claude".into(),
        route: Route {
            runtime,
            model: model.into(),
            strength: Strength::Fast,
            effort: Effort::Low,
        },
        timeout: Duration::from_secs(90),
        cwd: PathBuf::from("/data/deciders/cwd"),
        schema_dir: PathBuf::from("/data/deciders/schemas"),
        caps,
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn claude_decider_args_exact() {
    let s = schema(DeciderKind::BlockedReason);
    let args = claude_decider_args(
        &ctx(Runtime::Claude, "claude-haiku-4-5", CLI_CAPS),
        &DECIDER_CAPS,
        &s,
    );
    let tools = "Bash,Edit,Write,NotebookEdit,Agent,WebFetch,WebSearch,Read,Glob,Grep";
    let schema_json = s.to_string();
    assert_eq!(
        args,
        strings(&[
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-prompts",
            "none",
            "--setting-sources",
            "user",
            "--strict-mcp-config",
            "--tools",
            "",
            "--permission-mode",
            "dontAsk",
            "--disallowedTools",
            tools,
            "--max-turns",
            "3",
            "--json-schema",
            &schema_json,
            "--no-session-persistence",
            "--model",
            "claude-haiku-4-5",
            "--effort",
            "low",
        ])
    );

    // Older caps: no --verbose, --permission-prompts, user-settings flags or --effort;
    // decider caps without --max-turns, --json-schema or --no-session-persistence; no
    // model named.
    let old = CliCaps {
        claude_verbose: false,
        claude_permission_prompts: false,
        claude_effort_flag: false,
        claude_user_settings_only: None,
        ..CLI_CAPS
    };
    let dcaps = DeciderCaps {
        claude_json_schema: false,
        claude_max_turns: false,
        claude_no_session_persistence: false,
        ..DECIDER_CAPS
    };
    let args = claude_decider_args(&ctx(Runtime::Claude, "", old), &dcaps, &s);
    assert_eq!(
        args,
        strings(&[
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--tools",
            "",
            "--permission-mode",
            "dontAsk",
            "--disallowedTools",
            tools,
        ])
    );
    for a in [
        claude_decider_args(&ctx(Runtime::Claude, "m", CLI_CAPS), &DECIDER_CAPS, &s),
        args,
    ] {
        for banned in [
            "--settings",
            "--bare",
            "--mcp-config",
            "--dangerously-skip-permissions",
        ] {
            assert!(!a.iter().any(|x| x == banned), "{banned} in {a:?}");
        }
    }
}

#[test]
fn codex_decider_args_exact() {
    let file = Path::new("/data/deciders/schemas/triage-0123456789abcdef.json");
    let args = codex_decider_args(
        &ctx(Runtime::Codex, "gpt-5-mini", CLI_CAPS),
        &DECIDER_CAPS,
        file,
        "PROMPT",
    );
    let mut expected = strings(&[
        "exec",
        "--json",
        "--skip-git-repo-check",
        "--ephemeral",
        "-s",
        "read-only",
    ]);
    for pin in CODEX_SANDBOX_PINS {
        expected.extend(strings(&["-c", pin]));
    }
    expected.extend(strings(&[
        "-c",
        "approval_policy=\"never\"",
        "-c",
        "model_reasoning_effort=\"low\"",
        "--output-schema",
        "/data/deciders/schemas/triage-0123456789abcdef.json",
        "-m",
        "gpt-5-mini",
        "--",
        "PROMPT",
    ]));
    assert_eq!(args, expected);

    // A CLI with a project-config exclusion passes it after --ephemeral; without
    // --ephemeral, --output-schema or a model they are omitted.
    let caps = CliCaps {
        codex_user_config_only: Some(&["--ignore-project-config"]),
        ..CLI_CAPS
    };
    let dcaps = DeciderCaps {
        codex_ephemeral: false,
        codex_output_schema: false,
        ..DECIDER_CAPS
    };
    let args = codex_decider_args(&ctx(Runtime::Codex, "", caps), &dcaps, file, "-p");
    let mut expected = strings(&[
        "exec",
        "--json",
        "--skip-git-repo-check",
        "--ignore-project-config",
        "-s",
        "read-only",
    ]);
    for pin in CODEX_SANDBOX_PINS {
        expected.extend(strings(&["-c", pin]));
    }
    expected.extend(strings(&[
        "-c",
        "approval_policy=\"never\"",
        "-c",
        "model_reasoning_effort=\"low\"",
        "--",
        "-p",
    ]));
    assert_eq!(args, expected);
}

#[test]
fn schema_file_name_is_stable() {
    for kind in DeciderKind::ALL {
        let s = schema(kind);
        let name = schema_file_name(kind, &s);
        assert_eq!(name, schema_file_name(kind, &schema(kind)));
        let (label, rest) = name.rsplit_once('-').unwrap();
        assert_eq!(label, kind.label());
        let hex = rest.strip_suffix(".json").unwrap();
        assert_eq!(hex.len(), 16);
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }
    // The name is FNV-1a 64 of the schema's compact JSON: pinned for one kind, and a
    // changed schema gets a new file.
    assert_eq!(
        schema_file_name(DeciderKind::BlockedReason, &json!({})),
        format!("blocked_reason-{:016x}", fnv_of(b"{}")) + ".json"
    );
    assert_ne!(
        schema_file_name(DeciderKind::Triage, &json!({"a":1})),
        schema_file_name(DeciderKind::Triage, &json!({"a":2}))
    );
}

fn fnv_of(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[test]
fn decider_credential_scrub_removes_every_api_credential() {
    let all = [
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "CODEX_ACCESS_TOKEN",
        "OPENAI_BASE_URL",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
    ];
    for runtime in [Runtime::Claude, Runtime::Codex] {
        let scrub = credential_scrub_for(runtime, config::ClaudeAuth::Login);
        assert_eq!(scrub, all, "{runtime:?}");
        assert!(scrub.contains(&"ANTHROPIC_API_KEY") && scrub.contains(&"OPENAI_API_KEY"));
    }
    // What `credential_scrub(spec)` gave M8a's specs: only a Claude api_key session
    // keeps the Anthropic credentials.
    assert_eq!(
        credential_scrub_for(Runtime::Claude, config::ClaudeAuth::ApiKey),
        [
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
            "CODEX_ACCESS_TOKEN",
            "OPENAI_BASE_URL"
        ]
    );
    assert_eq!(
        credential_scrub_for(Runtime::Codex, config::ClaudeAuth::ApiKey),
        all
    );
}
