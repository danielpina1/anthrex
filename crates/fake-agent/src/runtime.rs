use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::Value;

#[derive(Debug, Default, PartialEq)]
pub struct Runtime {
    hooks: HashMap<String, String>,
    notify: Option<Vec<String>>,
}

impl Runtime {
    pub fn hook(&self, event: &str) -> Option<&str> {
        self.hooks.get(event).map(String::as_str)
    }

    pub fn notify(&self) -> Option<&[String]> {
        self.notify.as_deref()
    }
}

pub fn discover(args: &[String]) -> Result<Runtime> {
    let config_args = args
        .split(|arg| arg == "--")
        .next()
        .expect("split always yields one slice");
    if let Some(index) = config_args.iter().position(|arg| arg == "--settings") {
        let settings = config_args
            .get(index + 1)
            .context("--settings requires JSON")?;
        return claude(settings);
    }

    codex(config_args)
}

/// The anthrex MCP server a headless session was given: `--mcp-config` (Claude) or the
/// `-c mcp_servers.anthrex.command=` / `args=` pair (Codex), as
/// `daemon::headless::argv` writes them.
#[derive(Debug, Clone, PartialEq)]
pub struct McpServer {
    pub command: String,
    pub args: Vec<String>,
}

impl McpServer {
    /// The value after `flag` in the server's argv (`--role`, `--task`).
    pub fn flag(&self, flag: &str) -> Option<&str> {
        let index = self.args.iter().position(|arg| arg == flag)?;
        self.args.get(index + 1).map(String::as_str)
    }
}

pub fn mcp_server(args: &[String]) -> Result<Option<McpServer>> {
    let config_args = args
        .split(|arg| arg == "--")
        .next()
        .expect("split always yields one slice");
    if let Some(index) = config_args.iter().position(|arg| arg == "--mcp-config") {
        let config = config_args
            .get(index + 1)
            .context("--mcp-config requires JSON")?;
        let config: Value = serde_json::from_str(config).context("invalid --mcp-config JSON")?;
        let server = &config["mcpServers"]["anthrex"];
        if server.is_null() {
            return Ok(None);
        }
        let command = server["command"]
            .as_str()
            .context("the anthrex MCP server has no command")?;
        let args = serde_json::from_value(server["args"].clone())
            .context("the anthrex MCP server's args are not strings")?;
        return Ok(Some(McpServer {
            command: command.to_owned(),
            args,
        }));
    }

    let (mut command, mut server_args) = (None, None);
    for pair in config_args.windows(2) {
        if pair[0] != "-c" {
            continue;
        }
        if let Some(value) = pair[1].strip_prefix("mcp_servers.anthrex.command=") {
            command = Some(toml_value::<String>(value)?);
        } else if let Some(value) = pair[1].strip_prefix("mcp_servers.anthrex.args=") {
            server_args = Some(toml_value::<Vec<String>>(value)?);
        }
    }
    Ok(command.map(|command| McpServer {
        command,
        args: server_args.unwrap_or_default(),
    }))
}

/// One `-c key=<value>` value, parsed as TOML.
fn toml_value<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    let table: toml::Table =
        toml::from_str(&format!("v = {value}")).context("invalid Codex -c TOML value")?;
    let value = table.get("v").context("Codex -c value is missing")?.clone();
    value
        .try_into()
        .context("Codex -c value has the wrong type")
}

fn claude(settings: &str) -> Result<Runtime> {
    let settings: Value = serde_json::from_str(settings).context("invalid Claude settings JSON")?;
    let mut runtime = Runtime::default();
    let Some(hooks) = settings.get("hooks").and_then(Value::as_object) else {
        return Ok(runtime);
    };

    for (event, entries) in hooks {
        let command = entries
            .get(0)
            .and_then(|entry| entry.get("hooks"))
            .and_then(|commands| commands.get(0))
            .and_then(|hook| hook.get("command"))
            .and_then(Value::as_str);
        if let Some(command) = command {
            runtime.hooks.insert(event.clone(), command.to_owned());
        }
    }
    Ok(runtime)
}

fn codex(args: &[String]) -> Result<Runtime> {
    let mut runtime = Runtime::default();
    let mut index = 0;
    while index < args.len() {
        if args[index] != "-c" {
            index += 1;
            continue;
        }
        let Some(config) = args.get(index + 1) else {
            break;
        };
        if let Some(value) = config.strip_prefix("notify=") {
            runtime.notify =
                Some(serde_json::from_str(value).context("invalid Codex notify configuration")?);
        } else if let Some(rest) = config.strip_prefix("hooks.") {
            parse_codex_hook(rest, &mut runtime)?;
        }
        index += 2;
    }
    Ok(runtime)
}

fn parse_codex_hook(config: &str, runtime: &mut Runtime) -> Result<()> {
    let Some((event, value)) = config.split_once('=') else {
        return Ok(());
    };
    if event.contains('.') {
        return Ok(());
    }
    let Some((_, encoded_command)) = value.split_once("command=") else {
        return Ok(());
    };
    let command = serde_json::Deserializer::from_str(encoded_command)
        .into_iter::<String>()
        .next()
        .context("Codex hook command is missing")?
        .context("invalid Codex hook command string")?;
    runtime.hooks.insert(event.to_owned(), command);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{McpServer, discover, mcp_server};
    use serde_json::json;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn finds_claude_hook_commands_in_settings() {
        let settings = json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "",
                    "hooks": [{
                        "type": "command",
                        "command": "'/opt/anthrex/bin/anthrex' hook --window 4 --source claude"
                    }]
                }]
            },
            "preferredNotifChannel": "terminal_bell"
        })
        .to_string();
        let args = vec!["--name".into(), "x".into(), "--settings".into(), settings];

        let runtime = discover(&args).unwrap();

        assert_eq!(
            runtime.hook("PreToolUse"),
            Some("'/opt/anthrex/bin/anthrex' hook --window 4 --source claude")
        );
    }

    #[test]
    fn finds_codex_notify_and_hook_commands_in_config_overrides() {
        let command = "printf '%s' \"payload with a space\"";
        let hook = format!(
            "hooks.PreToolUse=[{{hooks=[{{type=\"command\",command={}}}]}}]",
            serde_json::to_string(command).unwrap()
        );
        let args = vec![
            "-C".into(),
            "/tmp/work".into(),
            "-c".into(),
            "notify=[\"/opt/anthrex/bin/anthrex\",\"hook\",\"--window\",\"4\",\"--source\",\"codex-notify\"]".into(),
            "-c".into(),
            hook,
            "-c".into(),
            "hooks.state.\"not-an-event\".trusted_hash=\"abc\"".into(),
        ];

        let runtime = discover(&args).unwrap();

        assert_eq!(
            runtime.notify(),
            Some(
                strings(&[
                    "/opt/anthrex/bin/anthrex",
                    "hook",
                    "--window",
                    "4",
                    "--source",
                    "codex-notify",
                ])
                .as_slice()
            )
        );
        assert_eq!(runtime.hook("PreToolUse"), Some(command));
        assert_eq!(runtime.hook("state"), None);
    }

    fn server_args() -> Vec<String> {
        strings(&[
            "mcp",
            "--role",
            "worker",
            "--run",
            "r1",
            "--task",
            "t1",
            "--window",
            "4",
            "--socket",
            "/tmp/d.sock",
        ])
    }

    #[test]
    fn finds_the_mcp_server_in_a_claude_mcp_config() {
        let config = json!({"mcpServers": {"anthrex": {
            "type": "stdio",
            "command": "/opt/anthrex/bin/anthrex",
            "args": server_args(),
        }}});
        let args = strings(&["-p", "--mcp-config", &config.to_string(), "--model", "x"]);

        let server = mcp_server(&args).unwrap().unwrap();

        assert_eq!(
            server,
            McpServer {
                command: "/opt/anthrex/bin/anthrex".into(),
                args: server_args(),
            }
        );
        assert_eq!(server.flag("--role"), Some("worker"));
        assert_eq!(server.flag("--task"), Some("t1"));
        assert_eq!(server.flag("--nope"), None);
    }

    #[test]
    fn finds_the_mcp_server_in_codex_config_overrides() {
        let list: Vec<String> = server_args()
            .iter()
            .map(|a| serde_json::to_string(a).unwrap())
            .collect();
        let args = vec![
            "exec".into(),
            "--json".into(),
            "-c".into(),
            "mcp_servers.anthrex.command=\"/opt/anthrex bin/anthrex\"".into(),
            "-c".into(),
            format!("mcp_servers.anthrex.args=[{}]", list.join(",")),
            "-c".into(),
            "mcp_servers.anthrex.tool_timeout_sec=120".into(),
            "--".into(),
            "-c".into(),
            "mcp_servers.anthrex.command=\"not-this\"".into(),
        ];

        let server = mcp_server(&args).unwrap().unwrap();

        assert_eq!(server.command, "/opt/anthrex bin/anthrex");
        assert_eq!(server.args, server_args());
    }

    #[test]
    fn no_mcp_server_without_its_flags() {
        let args = strings(&[
            "exec",
            "--json",
            "-c",
            "approval_policy=\"never\"",
            "--",
            "hi",
        ]);

        assert_eq!(mcp_server(&args).unwrap(), None);
    }

    #[test]
    fn unknown_flags_are_ignored() {
        let args = strings(&[
            "--name",
            "agent",
            "--model",
            "sonnet",
            "-m",
            "gpt-5",
            "-C",
            "/tmp/work",
            "--",
            "literal prompt",
            "--settings",
            "not-json",
            "-c",
            "notify=not-json",
        ]);

        let runtime = discover(&args).unwrap();

        assert_eq!(runtime.hook("PreToolUse"), None);
        assert_eq!(runtime.notify(), None);
    }
}
