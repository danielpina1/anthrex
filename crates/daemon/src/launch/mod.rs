//! Builds the command line and environment for each runtime. See spec section 3.2.

pub mod claude;
pub mod codex;

use proto::{HookSource, Runtime, WindowSpec};
use std::path::{Path, PathBuf};

/// Everything needed to spawn a window's child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

/// Per-window facts the launcher needs that are not in the spec.
pub struct LaunchContext<'a> {
    pub window_id: u32,
    pub name: &'a str,
    pub socket_path: &'a Path,
    /// The user's login shell, e.g. `/bin/zsh`.
    pub shell: &'a str,
    pub exe: &'a Path,
    pub claude_bin: &'a str,
    pub codex_bin: &'a str,
    pub codex_hook_source: Option<&'a str>,
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn hook_command(exe: &Path, window_id: u32, source: HookSource) -> String {
    let label = match source {
        HookSource::Claude => "claude",
        HookSource::CodexNotify => "codex-notify",
        HookSource::CodexHook => "codex-hook",
    };
    format!(
        "{} hook --window {window_id} --source {label}",
        shell_quote(&exe.display().to_string())
    )
}

pub fn plan(spec: &WindowSpec, ctx: &LaunchContext<'_>) -> LaunchPlan {
    let env = vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
        ("ANTHREX_WINDOW_ID".to_string(), ctx.window_id.to_string()),
        (
            "ANTHREX_SOCKET".to_string(),
            ctx.socket_path.display().to_string(),
        ),
    ];
    let (program, args) = match spec.runtime {
        Runtime::Shell => (ctx.shell.to_string(), vec!["-l".to_string()]),
        Runtime::Claude => {
            let settings = serde_json::to_string(&claude::settings(ctx.exe, ctx.window_id))
                .expect("Claude hook settings are serializable");
            let mut args = vec![
                "--name".to_string(),
                ctx.name.to_string(),
                "--settings".to_string(),
                settings,
            ];
            if let Some(model) = &spec.model {
                args.push("--model".to_string());
                args.push(model.clone());
            }
            if let Some(prompt) = &spec.initial_prompt {
                // `--` first: a prompt that starts with a dash is a prompt, not an option.
                // Verified that `claude --prompt --version` printed the version and exited.
                args.push("--".to_string());
                args.push(prompt.clone());
            }
            (ctx.claude_bin.to_string(), args)
        }
        Runtime::Codex => (ctx.codex_bin.to_string(), codex::args(spec, ctx)),
    };
    LaunchPlan {
        program,
        args,
        cwd: spec.cwd.clone(),
        env,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{HookSource, Runtime, WindowSpec};
    use std::path::Path;

    fn spec(runtime: Runtime) -> WindowSpec {
        WindowSpec {
            name: Some("api".into()),
            runtime,
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
        }
    }

    #[test]
    fn shell_quote_wraps_and_escapes_single_quotes() {
        assert_eq!(shell_quote("/a b/x"), "'/a b/x'");
        assert_eq!(shell_quote("/it's/x"), "'/it'\\''s/x'");
    }

    #[test]
    fn hook_command_quotes_the_exe() {
        assert_eq!(
            hook_command(Path::new("/opt/anthrex/bin/anthrex"), 4, HookSource::Claude),
            "'/opt/anthrex/bin/anthrex' hook --window 4 --source claude"
        );
    }

    #[test]
    fn shell_runs_login_shell_in_cwd_with_anthrex_env() {
        let p = plan(&spec(Runtime::Shell), &ctx());
        assert_eq!(p.program, "/bin/zsh");
        assert_eq!(p.args, vec!["-l".to_string()]);
        assert_eq!(p.cwd, Path::new("/tmp/repo"));
        let env: std::collections::HashMap<_, _> = p.env.iter().cloned().collect();
        assert_eq!(env["TERM"], "xterm-256color");
        assert_eq!(env["COLORTERM"], "truecolor");
        assert_eq!(env["ANTHREX_WINDOW_ID"], "4");
        assert_eq!(env["ANTHREX_SOCKET"], "/tmp/a.sock");
    }

    #[test]
    fn claude_gets_name_settings_model_and_prompt_in_order() {
        let mut s = spec(Runtime::Claude);
        s.model = Some("opus".into());
        s.initial_prompt = Some("fix the tests".into());
        let p = plan(&s, &ctx());
        let settings =
            serde_json::to_string(&claude::settings(Path::new("/opt/anthrex/bin/anthrex"), 4))
                .unwrap();
        assert_eq!(p.program, "/opt/agents/claude");
        assert_eq!(
            p.args,
            vec![
                "--name",
                "api",
                "--settings",
                &settings,
                "--model",
                "opus",
                "--",
                "fix the tests"
            ]
        );
    }

    #[test]
    fn programs_come_from_the_context() {
        assert_eq!(
            plan(&spec(Runtime::Claude), &ctx()).program,
            "/opt/agents/claude"
        );
        assert_eq!(
            plan(&spec(Runtime::Codex), &ctx()).program,
            "/opt/agents/codex"
        );
    }

    /// M1: a prompt beginning with a dash must reach the agent as a prompt. Verified that
    /// `claude --prompt "--version"` otherwise printed claude's version and exited.
    #[test]
    fn a_prompt_starting_with_a_dash_is_separated_by_a_double_dash() {
        for runtime in [Runtime::Claude, Runtime::Codex] {
            let mut s = spec(runtime);
            s.initial_prompt = Some("--version".into());
            let p = plan(&s, &ctx());
            let dashdash = p.args.iter().position(|a| a == "--").expect("-- present");
            assert_eq!(p.args[dashdash + 1], "--version");
            assert_eq!(
                dashdash + 2,
                p.args.len(),
                "the prompt is last: {:?}",
                p.args
            );
        }
    }

    #[test]
    fn codex_args_without_hooks_are_exact() {
        let mut s = spec(Runtime::Codex);
        s.model = Some("gpt-5-codex".into());
        s.initial_prompt = Some("hello".into());
        let mut context = ctx();
        context.window_id = 7;
        let p = plan(&s, &context);
        assert_eq!(p.program, "/opt/agents/codex");
        assert_eq!(
            p.args,
            vec![
                "-C",
                "/tmp/repo",
                "-c",
                r#"notify=["/opt/anthrex/bin/anthrex","hook","--window","7","--source","codex-notify"]"#,
                "-c",
                r#"tui.terminal_title=["status"]"#,
                "-c",
                r#"tui.notifications=["approval-requested"]"#,
                "-c",
                r#"tui.notification_method="bel""#,
                "-c",
                r#"tui.notification_condition="always""#,
                "-m",
                "gpt-5-codex",
                "--",
                "hello"
            ]
        );
    }

    #[test]
    fn codex_args_with_hooks_are_exact() {
        let mut s = spec(Runtime::Codex);
        s.model = Some("gpt-5-codex".into());
        s.initial_prompt = Some("hello".into());
        let mut context = ctx();
        context.window_id = 7;
        context.codex_hook_source = Some("/tmp/codex-home/config.toml");
        let args = plan(&s, &context).args;
        assert_eq!(args.len(), 48);
        assert_eq!(args[12], "-c");
        assert_eq!(
            args[13],
            r#"hooks.SessionStart=[{hooks=[{type="command",command="'/opt/anthrex/bin/anthrex' hook --window 7 --source codex-hook"}]}]"#
        );
        assert_eq!(args[14], "-c");
        assert_eq!(
            args[15],
            r#"hooks.state={"/tmp/codex-home/config.toml:session_start:0:0"={trusted_hash="sha256:69de55a0df8e7abb3454e8d67f19af0d5797d208837a23f45af6e2617fb3668e"}}"#
        );
        let mut expected_trust_entries = Vec::new();
        for (index, (event, label, hash)) in [
            (
                "SessionStart",
                "session_start",
                "69de55a0df8e7abb3454e8d67f19af0d5797d208837a23f45af6e2617fb3668e",
            ),
            (
                "UserPromptSubmit",
                "user_prompt_submit",
                "ce28fe1882eab2637afb9213a27be7ac6a67155af428b111b3af6f4b616bd062",
            ),
            (
                "PreToolUse",
                "pre_tool_use",
                "cb3a96262c960578050675ee2bf22680d8aaf909f7941553bae039b36074ad7e",
            ),
            (
                "PermissionRequest",
                "permission_request",
                "bd8e3f7788f89e22bee74e8013c66f372634c2ebe116dff5b4b5ebc575a3227b",
            ),
            (
                "PostToolUse",
                "post_tool_use",
                "f636075c28818926b2811587ea0eb0f6bca8c33246d06da515f215f72e2273bc",
            ),
            (
                "SubagentStart",
                "subagent_start",
                "e78a75f4a757c49a69d9f05868989d0e87b84d2f16b330d712a73b353882718b",
            ),
            (
                "SubagentStop",
                "subagent_stop",
                "d19a438eb5d993e8fc89de50a36ff0a032cd82e4c8d8884306a45d0ca485f0d3",
            ),
            (
                "Stop",
                "stop",
                "38977630548e48639d47d2af5b14143f24886115cfab74dcd4aae3da28f1d203",
            ),
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(args[12 + index * 4], "-c");
            assert_eq!(
                args[13 + index * 4],
                format!(
                    "hooks.{event}=[{{hooks=[{{type=\"command\",command=\"'/opt/anthrex/bin/anthrex' hook --window 7 --source codex-hook\"}}]}}]"
                )
            );
            assert_eq!(args[14 + index * 4], "-c");
            expected_trust_entries.push(format!(
                "\"/tmp/codex-home/config.toml:{label}:0:0\"={{trusted_hash=\"sha256:{hash}\"}}"
            ));
            assert_eq!(
                args[15 + index * 4],
                format!("hooks.state={{{}}}", expected_trust_entries.join(","))
            );
        }
        assert_eq!(&args[44..], ["-m", "gpt-5-codex", "--", "hello"]);
        assert!(!args.iter().any(|arg| arg.contains("dangerously-bypass")));
    }

    #[test]
    fn codex_final_cli_trust_override_retains_every_hook() {
        let mut context = ctx();
        context.codex_hook_source = Some("/tmp/codex-home/config.toml");
        let args = plan(&spec(Runtime::Codex), &context).args;
        // Codex inserts each -c value by key: later values replace earlier ones.
        let effective: std::collections::HashMap<_, _> = args
            .windows(2)
            .filter(|pair| pair[0] == "-c")
            .map(|pair| pair[1].split_once('=').unwrap())
            .collect();
        let trust = effective["hooks.state"];
        for label in [
            "session_start",
            "user_prompt_submit",
            "pre_tool_use",
            "permission_request",
            "post_tool_use",
            "subagent_start",
            "subagent_stop",
            "stop",
        ] {
            assert!(
                trust.contains(&format!("\"/tmp/codex-home/config.toml:{label}:0:0\"=")),
                "missing {label} from effective trust: {trust}"
            );
        }
        assert_eq!(trust.matches("trusted_hash=").count(), 8);
    }
}
