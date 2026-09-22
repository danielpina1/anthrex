use super::{LaunchContext, hook_command};
use proto::{HookSource, WindowSpec};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write;

pub fn args(spec: &WindowSpec, ctx: &LaunchContext<'_>) -> Vec<String> {
    let id = ctx.window_id.to_string();
    let exe = ctx.exe.to_string_lossy();
    let notify = [
        exe.as_ref(),
        "hook",
        "--window",
        &id,
        "--source",
        "codex-notify",
    ]
    .map(toml_string)
    .join(",");
    let mut args = vec!["-C".into(), spec.cwd.display().to_string()];
    for value in [
        format!("notify=[{notify}]"),
        format!("tui.terminal_title=[{}]", toml_string("status")),
        format!("tui.notifications=[{}]", toml_string("approval-requested")),
        format!("tui.notification_method={}", toml_string("bel")),
        format!("tui.notification_condition={}", toml_string("always")),
    ] {
        args.extend(["-c".into(), value]);
    }
    if ctx.codex_bypass_hook_trust {
        // Config decision 4: `--dangerously-bypass-hook-trust` right after the five `-c`
        // flags above, then the eight `hooks.<Event>` flags with no `hooks.state` trust
        // hashes — Codex is told to run the hooks without needing them trusted, so the
        // hashes `codex_hook_source` would otherwise add serve no purpose. This applies
        // whether or not `codex_hook_source` is set: the bypass turns lifecycle hooks on
        // by itself.
        args.push("--dangerously-bypass-hook-trust".into());
        let command = hook_command(ctx.exe, ctx.window_id, HookSource::CodexHook);
        for (event, _label) in HOOK_EVENTS {
            args.extend([
                "-c".into(),
                format!(
                    "hooks.{event}=[{{hooks=[{{type={},command={}}}]}}]",
                    toml_string("command"),
                    toml_string(&command)
                ),
            ]);
        }
    } else if let Some(source) = ctx.codex_hook_source {
        let command = hook_command(ctx.exe, ctx.window_id, HookSource::CodexHook);
        let mut trust_entries = Vec::new();
        for (event, label) in HOOK_EVENTS {
            // Codex replaces repeated CLI keys within this config layer. Each
            // adjacent trust flag must retain the earlier generated entries.
            trust_entries.push(format!(
                "{}={{trusted_hash={}}}",
                toml_string(&trust_key(source, label, 0, 0)),
                toml_string(&trust_hash(label, &command))
            ));
            args.extend([
                "-c".into(),
                format!(
                    "hooks.{event}=[{{hooks=[{{type={},command={}}}]}}]",
                    toml_string("command"),
                    toml_string(&command)
                ),
                "-c".into(),
                format!("hooks.state={{{}}}", trust_entries.join(",")),
            ]);
        }
    }
    if let Some(model) = &spec.model {
        // Design decision 16: kept on resume too. `codex resume --help` on codex-cli
        // 0.155.0 still lists `-m, --model <MODEL>`, and `codex -C /tmp -m gpt-x -c
        // 'tui.notification_method="bel"' resume --help` exits 0, confirming these root
        // options are accepted ahead of `resume`.
        args.extend(["-m".into(), model.clone()]);
    }
    if let Some(id) = ctx.resume {
        // Design decision 15: the initial prompt is never sent on resume.
        args.extend(["resume".into(), id.to_string()]);
    } else if let Some(prompt) = &spec.initial_prompt {
        args.extend(["--".into(), prompt.clone()]);
    }
    args
}

pub const HOOK_EVENTS: [(&str, &str); 8] = [
    ("SessionStart", "session_start"),
    ("UserPromptSubmit", "user_prompt_submit"),
    ("PreToolUse", "pre_tool_use"),
    ("PermissionRequest", "permission_request"),
    ("PostToolUse", "post_tool_use"),
    ("SubagentStart", "subagent_start"),
    ("SubagentStop", "subagent_stop"),
    ("Stop", "stop"),
];

pub const MIN_CODEX_VERSION: (u64, u64, u64) = (0, 135, 0);

pub fn toml_string(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() + 2);
    escaped.push('"');
    for ch in s.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{0008}' => escaped.push_str("\\b"),
            '\u{000C}' => escaped.push_str("\\f"),
            '\u{0000}'..='\u{001F}' | '\u{007F}' => {
                write!(escaped, "\\u{:04X}", ch as u32).expect("writing to a String cannot fail");
            }
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

pub fn canonical_json(value: &Value) -> String {
    let mut canonical = String::new();
    write_canonical_json(value, &mut canonical);
    canonical
}

fn write_canonical_json(value: &Value, output: &mut String) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => output.push_str(
            &serde_json::to_string(value).expect("serializing a JSON string cannot fail"),
        ),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                write_canonical_json(value, output);
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut keys: Vec<_> = values.keys().collect();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).expect("serializing a JSON key cannot fail"),
                );
                output.push(':');
                write_canonical_json(&values[key], output);
            }
            output.push('}');
        }
    }
}

pub fn trust_key(source: &str, label: &str, group: usize, handler: usize) -> String {
    format!("{source}:{label}:{group}:{handler}")
}

pub fn trust_hash(label: &str, command: &str) -> String {
    let normalized = serde_json::json!({
        "event_name": label,
        "hooks": [{
            "async": false,
            "command": command,
            "timeout": 600,
            "type": "command",
        }],
    });
    let digest = Sha256::digest(canonical_json(&normalized).as_bytes());
    format!("sha256:{digest:x}")
}

/// Returns Codex's synthetic path for configuration supplied through `-c`.
pub fn default_hook_source() -> Option<String> {
    Some("/<session-flags>/config.toml".to_owned())
}

pub fn parse_version(output: &str) -> Option<(u64, u64, u64)> {
    output.split_whitespace().rev().find_map(|token| {
        let mut parts = token.split('.');
        let version = (
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        );
        parts.next().is_none().then_some(version)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::Runtime;
    use serde_json::json;
    use std::path::Path;

    fn spec() -> WindowSpec {
        WindowSpec {
            name: Some("api".into()),
            runtime: Runtime::Codex,
            cwd: "/tmp/repo".into(),
            worktree_branch: None,
            model: None,
            initial_prompt: None,
        }
    }

    fn ctx() -> LaunchContext<'static> {
        LaunchContext {
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
        }
    }

    /// Config decision 4: with `codex_bypass_hook_trust: true` and no
    /// `codex_hook_source`, the bypass flag and the eight hook flags appear with no trust
    /// hashes at all; with it `false`, neither appears, exactly as in milestone 3.
    #[test]
    fn bypass_hook_trust_adds_the_flag_and_drops_trust_hashes() {
        let mut context = ctx();
        context.codex_bypass_hook_trust = true;
        let bypassed = args(&spec(), &context);
        // The five `-c` flags of milestone 3's decision 19 occupy indices 2..=11 (`-C`,
        // cwd, then five `-c`/value pairs); the bypass flag comes right after them.
        assert_eq!(&bypassed[..2], ["-C", "/tmp/repo"]);
        assert_eq!(bypassed[12], "--dangerously-bypass-hook-trust");
        let command = hook_command(context.exe, context.window_id, HookSource::CodexHook);
        for (index, (event, _label)) in HOOK_EVENTS.iter().enumerate() {
            assert_eq!(bypassed[13 + index * 2], "-c");
            assert_eq!(
                bypassed[14 + index * 2],
                format!("hooks.{event}=[{{hooks=[{{type=\"command\",command=\"{command}\"}}]}}]")
            );
        }
        assert_eq!(bypassed.len(), 13 + HOOK_EVENTS.len() * 2);
        assert!(!bypassed.iter().any(|a| a.starts_with("hooks.state")));

        context.codex_bypass_hook_trust = false;
        let plain = args(&spec(), &context);
        assert!(!plain.iter().any(|a| a == "--dangerously-bypass-hook-trust"));
        assert!(!plain.iter().any(|a| a.starts_with("hooks.")));
    }

    #[test]
    fn canonical_json_sorts_keys_recursively_and_is_compact() {
        let value = json!({
            "z": [{"b": 2, "a": 1}],
            "a": {"d": 4, "c": 3},
        });

        assert_eq!(
            canonical_json(&value),
            r#"{"a":{"c":3,"d":4},"z":[{"a":1,"b":2}]}"#
        );
    }

    #[test]
    fn trust_hash_matches_the_test_vectors() {
        let command = "'/opt/anthrex/bin/anthrex' hook --window 7 --source codex-hook";
        let normalized = json!({
            "event_name": "pre_tool_use",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": 600,
                "async": false,
            }],
        });
        assert_eq!(
            canonical_json(&normalized),
            r#"{"event_name":"pre_tool_use","hooks":[{"async":false,"command":"'/opt/anthrex/bin/anthrex' hook --window 7 --source codex-hook","timeout":600,"type":"command"}]}"#
        );
        assert_eq!(
            trust_hash("pre_tool_use", command),
            "sha256:cb3a96262c960578050675ee2bf22680d8aaf909f7941553bae039b36074ad7e"
        );
        assert_eq!(
            trust_hash("session_start", command),
            "sha256:69de55a0df8e7abb3454e8d67f19af0d5797d208837a23f45af6e2617fb3668e"
        );
        assert_eq!(
            trust_hash("pre_tool_use", "/tmp/anthrex-m3-codex-hook-log.py"),
            "sha256:d58938d41a83039d93b170819dc549e06a9b2f646de44905e017131bfdb30177"
        );
    }

    #[test]
    fn trust_key_joins_its_parts() {
        assert_eq!(
            trust_key("/h/config.toml", "pre_tool_use", 0, 0),
            "/h/config.toml:pre_tool_use:0:0"
        );
    }

    #[test]
    fn toml_string_escapes_quotes_backslashes_and_controls() {
        assert_eq!(
            toml_string("\"\\\n\r\t\u{0008}\u{000C}\u{0001}\u{007F}é"),
            "\"\\\"\\\\\\n\\r\\t\\b\\f\\u0001\\u007Fé\""
        );
    }

    #[test]
    fn parse_version_reads_codex_cli_output() {
        assert_eq!(parse_version("codex-cli 0.155.0\n"), Some((0, 155, 0)));
        assert_eq!(parse_version("garbage"), None);
        assert!((0, 134, 9) < MIN_CODEX_VERSION);
    }

    #[test]
    fn hook_events_match_the_supported_lifecycle_order() {
        assert_eq!(
            HOOK_EVENTS,
            [
                ("SessionStart", "session_start"),
                ("UserPromptSubmit", "user_prompt_submit"),
                ("PreToolUse", "pre_tool_use"),
                ("PermissionRequest", "permission_request"),
                ("PostToolUse", "post_tool_use"),
                ("SubagentStart", "subagent_start"),
                ("SubagentStop", "subagent_stop"),
                ("Stop", "stop"),
            ]
        );
    }

    #[test]
    fn default_hook_source_uses_the_session_flags_layer() {
        assert_eq!(
            default_hook_source(),
            Some("/<session-flags>/config.toml".to_owned())
        );
    }
}
