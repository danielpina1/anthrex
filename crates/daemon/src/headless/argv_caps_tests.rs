//! The argv branches that depend on `CliCaps`, auth and the TOML strings.

use super::*;
use crate::launch::codex::toml_string;

/// Flags that take a variable number of values, so the next argument must be a flag.
const VARIADIC: [&str; 4] = [
    "--mcp-config",
    "--allowedTools",
    "--disallowedTools",
    "--setting-sources",
];

fn sessions() -> [SessionArg; 3] {
    [
        new_session(),
        SessionArg::New { uuid: None },
        SessionArg::Resume {
            session_id: "sess-9".into(),
        },
    ]
}

/// Every Claude argv over role, model, effort flag, session argument and auth.
fn every_claude_argv(caps: &CliCaps) -> Vec<Vec<String>> {
    let mut all = Vec::new();
    for spec in [worker(Runtime::Claude), reviewer(Runtime::Claude)] {
        for model in ["", "claude-haiku-4-5"] {
            for effort_flag in [true, false] {
                for session in sessions() {
                    for auth in [config::ClaudeAuth::Login, config::ClaudeAuth::ApiKey] {
                        let spec = HeadlessSpec {
                            model: model.into(),
                            claude_auth: auth,
                            ..spec.clone()
                        };
                        let caps = CliCaps {
                            claude_effort_flag: effort_flag,
                            ..*caps
                        };
                        all.push(claude(&spec, &session, &caps));
                    }
                }
            }
        }
    }
    all
}

/// How many times `needle` occurs in `argv` as a contiguous run.
fn occurrences(argv: &[String], needle: &[&str]) -> usize {
    argv.windows(needle.len())
        .filter(|w| w.iter().zip(needle).all(|(a, b)| a == b))
        .count()
}

#[test]
fn claude_variadic_flags_are_followed_by_a_flag() {
    let mut checked = 0;
    for argv in every_claude_argv(&CLI_CAPS) {
        for (at, arg) in argv.iter().enumerate() {
            if VARIADIC.contains(&arg.as_str()) {
                let next = argv
                    .get(at + 2)
                    .unwrap_or_else(|| panic!("{arg} last: {argv:?}"));
                assert!(next.starts_with("--"), "{arg} then {next}: {argv:?}");
                checked += 1;
            }
        }
        // The prompt is never on the argv: every value follows its own flag.
        assert_eq!(argv[0], "-p");
    }
    assert!(checked > 48 * 2, "{checked}");
}

#[test]
fn api_key_auth_adds_bare() {
    let mut spec = worker(Runtime::Claude);
    spec.claude_auth = config::ClaudeAuth::ApiKey;
    spec.api_key_helper = Some("/usr/local/bin/key-helper".into());
    let argv = claude(&spec, &new_session(), &CLI_CAPS);
    assert_eq!(argv.last().map(String::as_str), Some("--bare"));
    assert_eq!(occurrences(&argv, &["--bare"]), 1);
    // Decision 50: the helper goes through `--settings`.
    assert_eq!(
        json_after(&argv, "--settings")["apiKeyHelper"],
        "/usr/local/bin/key-helper"
    );

    // `login` passes neither, even with a helper configured.
    spec.claude_auth = config::ClaudeAuth::Login;
    let argv = claude(&spec, &new_session(), &CLI_CAPS);
    assert_eq!(occurrences(&argv, &["--bare"]), 0);
    assert!(
        json_after(&argv, "--settings")
            .get("apiKeyHelper")
            .is_none()
    );
}

#[test]
fn login_adds_the_non_bare_flag_only_when_caps_say_so() {
    let opt_out = CliCaps {
        claude_non_bare_flag: Some("--no-bare"),
        ..CLI_CAPS
    };
    let mut spec = worker(Runtime::Claude);
    let argv = claude(&spec, &new_session(), &opt_out);
    assert_eq!(argv.last().map(String::as_str), Some("--no-bare"));
    assert_eq!(occurrences(&argv, &["--no-bare"]), 1);
    assert_eq!(occurrences(&argv, &["--bare"]), 0);

    spec.claude_auth = config::ClaudeAuth::ApiKey;
    let argv = claude(&spec, &new_session(), &opt_out);
    assert_eq!(occurrences(&argv, &["--no-bare"]), 0);
    assert_eq!(argv.last().map(String::as_str), Some("--bare"));

    // M8a.1 found no opt-out, so today `login` adds nothing.
    assert_eq!(CLI_CAPS.claude_non_bare_flag, None);
    spec.claude_auth = config::ClaudeAuth::Login;
    let argv = claude(&spec, &new_session(), &CLI_CAPS);
    assert!(!argv.iter().any(|a| a.contains("bare")));
}

#[test]
fn claude_launches_load_only_user_settings() {
    let flags = CLI_CAPS
        .claude_user_settings_only
        .expect("M8a.1 item 4a found the flags");
    assert_eq!(flags, ["--setting-sources", "user", "--strict-mcp-config"]);
    for argv in every_claude_argv(&CLI_CAPS) {
        assert_eq!(occurrences(&argv, flags), 1, "{argv:?}");
        let at = argv
            .windows(flags.len())
            .position(|w| w.iter().zip(flags).all(|(a, b)| a == b))
            .unwrap();
        let settings = argv.iter().position(|a| a == "--settings").unwrap();
        assert!(at < settings, "{argv:?}");
    }
    let without = CliCaps {
        claude_user_settings_only: None,
        ..CLI_CAPS
    };
    for argv in every_claude_argv(&without) {
        for flag in flags {
            assert!(!argv.contains(&flag.to_string()), "{flag} in {argv:?}");
        }
    }
}

#[test]
fn codex_launches_exclude_project_config_when_caps_say_so() {
    let flags: &[&str] = &["--anthrex-test-exclude-project-config", "-c", "x=1"];
    let with = CliCaps {
        codex_user_config_only: Some(&["--anthrex-test-exclude-project-config", "-c", "x=1"]),
        ..CLI_CAPS
    };
    for spec in [worker(Runtime::Codex), reviewer(Runtime::Codex)] {
        for session in sessions() {
            let argv = codex(&spec, &session, "go", &with);
            assert_eq!(occurrences(&argv, flags), 1, "{argv:?}");
            let at = argv
                .windows(flags.len())
                .position(|w| w.iter().zip(flags).all(|(a, b)| a == b))
                .unwrap();
            let dashes = argv.iter().position(|a| a == "--").unwrap();
            assert!(at < dashes, "{argv:?}");
            assert_eq!(argv.last().map(String::as_str), Some("go"));

            // M8a.1 item 7a: nothing on the command line excludes it today.
            let argv = codex(&spec, &session, "go", &CLI_CAPS);
            assert_eq!(occurrences(&argv, flags), 0, "{argv:?}");
            assert!(!argv.iter().any(|a| a.contains("exclude-project-config")));
        }
    }
    assert_eq!(CLI_CAPS.codex_user_config_only, None);
    assert_eq!(
        CLI_CAPS.codex_project_config_paths,
        [".codex/config.toml", ".codex/hooks.json"]
    );
    let loads = CLI_CAPS.codex_loads_project_config;
    assert!(loads, "M8a.1 item 7a: Codex loads project config");
}

#[test]
fn codex_resume_takes_the_sandbox_flag_when_caps_say_so() {
    let takes = CliCaps {
        codex_resume_takes_sandbox: true,
        ..CLI_CAPS
    };
    let resume = SessionArg::Resume {
        session_id: "th-1".into(),
    };
    let argv = codex(&worker(Runtime::Codex), &resume, "go", &takes);
    assert_eq!(occurrences(&argv, &["-s", "workspace-write"]), 1);
    assert!(!argv.iter().any(|a| a.starts_with("sandbox_mode=")));
    let argv = codex(&worker(Runtime::Codex), &resume, "go", &CLI_CAPS);
    assert_eq!(occurrences(&argv, &["-s", "workspace-write"]), 0);
    assert_eq!(
        occurrences(&argv, &["-c", "sandbox_mode=\"workspace-write\""]),
        1
    );
}

#[test]
fn a_session_without_mcp_or_tools_omits_their_flags() {
    let mut spec = worker(Runtime::Claude);
    spec.mcp = None;
    spec.allowed_tools.clear();
    spec.claude_permission_mode = None;
    let argv = claude(&spec, &new_session(), &CLI_CAPS);
    for flag in ["--mcp-config", "--allowedTools", "--permission-mode"] {
        assert!(!argv.contains(&flag.to_string()), "{flag}");
    }
    let mut spec = worker(Runtime::Codex);
    spec.mcp = None;
    let argv = codex(&spec, &SessionArg::New { uuid: None }, "go", &CLI_CAPS);
    assert!(!argv.iter().any(|a| a.starts_with("mcp_servers.")));
}

/// The refreshed M8 brief's twenty strings, restated: every control character, U+007F,
/// quotes, backslashes and a multi-line text.
fn twenty_strings() -> Vec<String> {
    let controls: String = (0u8..0x20).map(char::from).collect();
    vec![
        String::new(),
        "plain".into(),
        controls.clone(),
        controls[..16].to_string(),
        controls[16..].to_string(),
        "\u{7f}".into(),
        "a\u{7f}b\u{0}c".into(),
        "\"".into(),
        "say \"hi\"".into(),
        "\\".into(),
        "C:\\path\\to\\file".into(),
        "\\\"".into(),
        "\"\\\"\\".into(),
        "line one\nline two\r\nline three\n".into(),
        "tab\there\u{8}back\u{c}feed".into(),
        "é世🦀".into(),
        "'single' and `back`".into(),
        "\"\"\"triple\"\"\"".into(),
        "trailing backslash\\".into(),
        format!("Every control: {controls}\u{7f} \"quoted\" \\slashed\\\nnext line"),
    ]
}

#[test]
fn toml_string_round_trips_through_the_toml_crate() {
    let strings = twenty_strings();
    assert_eq!(strings.len(), 20);
    for s in &strings {
        let document = format!("x = {}", toml_string(s));
        let table: toml::Table = toml::from_str(&document)
            .unwrap_or_else(|e| panic!("{document:?} does not parse: {e}"));
        assert_eq!(table["x"].as_str(), Some(s.as_str()), "{document:?}");
    }
    // The argv's own TOML values parse too: the instructions and the MCP args array.
    let argv = codex(
        &worker(Runtime::Codex),
        &SessionArg::New { uuid: None },
        "go",
        &CLI_CAPS,
    );
    for value in argv
        .iter()
        .filter(|a| a.contains('=') && !a.starts_with('-'))
    {
        let parsed: Result<toml::Table, _> = toml::from_str(value);
        assert!(parsed.is_ok(), "{value}");
    }
}

/// Decision 53's test overrides: `ANTHREX_TEST_NO_SETTING_SOURCES=1` acts as if the
/// CLI could not exclude project settings; `ANTHREX_TEST_CODEX_PROJECT_CONFIG=load` as
/// if Codex loads project config with no way out, `=exclude` as if the placeholder flag
/// excluded it. Anything else leaves the caps alone.
#[test]
fn test_overrides_follow_decision_53() {
    let var = |pairs: &'static [(&'static str, &'static str)]| {
        move |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        }
    };
    assert_eq!(caps_with_test_overrides(CLI_CAPS, var(&[])), CLI_CAPS);
    let none = caps_with_test_overrides(CLI_CAPS, var(&[("ANTHREX_TEST_NO_SETTING_SOURCES", "1")]));
    assert_eq!(none.claude_user_settings_only, None);
    let load = caps_with_test_overrides(
        CLI_CAPS,
        var(&[("ANTHREX_TEST_CODEX_PROJECT_CONFIG", "load")]),
    );
    assert!(load.codex_loads_project_config);
    assert_eq!(load.codex_user_config_only, None);
    let exclude = caps_with_test_overrides(
        CLI_CAPS,
        var(&[("ANTHREX_TEST_CODEX_PROJECT_CONFIG", "exclude")]),
    );
    assert!(exclude.codex_loads_project_config);
    assert_eq!(
        exclude.codex_user_config_only,
        Some(&[TEST_EXCLUDE_PROJECT_CONFIG][..])
    );
    let other =
        caps_with_test_overrides(CLI_CAPS, var(&[("ANTHREX_TEST_NO_SETTING_SOURCES", "0")]));
    assert_eq!(other, CLI_CAPS);
}
