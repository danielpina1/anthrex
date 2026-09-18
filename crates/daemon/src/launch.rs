//! Builds the command line and environment for each runtime. See spec section 3.2.

use proto::{Runtime, WindowSpec};
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
}

pub fn plan(spec: &WindowSpec, ctx: &LaunchContext<'_>) -> LaunchPlan {
    let env = vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
        ("ANTHREX_WINDOW_ID".to_string(), ctx.window_id.to_string()),
        ("ANTHREX_SOCKET".to_string(), ctx.socket_path.display().to_string()),
    ];
    let (program, args) = match spec.runtime {
        Runtime::Shell => (ctx.shell.to_string(), vec!["-l".to_string()]),
        Runtime::Claude => {
            let mut args = vec!["--name".to_string(), ctx.name.to_string()];
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
            ("claude".to_string(), args)
        }
        Runtime::Codex => {
            let mut args = vec!["-C".to_string(), spec.cwd.display().to_string()];
            if let Some(model) = &spec.model {
                args.push("-m".to_string());
                args.push(model.clone());
            }
            if let Some(prompt) = &spec.initial_prompt {
                // See the `claude` arm: `--` keeps a leading-dash prompt out of the parser.
                args.push("--".to_string());
                args.push(prompt.clone());
            }
            ("codex".to_string(), args)
        }
    };
    LaunchPlan { program, args, cwd: spec.cwd.clone(), env }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{Runtime, WindowSpec};
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
        LaunchContext { window_id: 4, name: "api", socket_path: Path::new("/tmp/a.sock"), shell: "/bin/zsh" }
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
    fn claude_gets_name_model_and_prompt() {
        let mut s = spec(Runtime::Claude);
        s.model = Some("opus".into());
        s.initial_prompt = Some("fix the tests".into());
        let p = plan(&s, &ctx());
        assert_eq!(p.program, "claude");
        assert_eq!(p.args, vec!["--name", "api", "--model", "opus", "--", "fix the tests"]);
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
            assert_eq!(dashdash + 2, p.args.len(), "the prompt is last: {:?}", p.args);
        }
    }

    #[test]
    fn codex_gets_cwd_flag_model_and_prompt() {
        let mut s = spec(Runtime::Codex);
        s.model = Some("gpt-5-codex".into());
        s.initial_prompt = Some("hello".into());
        let p = plan(&s, &ctx());
        assert_eq!(p.program, "codex");
        assert_eq!(p.args, vec!["-C", "/tmp/repo", "-m", "gpt-5-codex", "--", "hello"]);
    }
}
