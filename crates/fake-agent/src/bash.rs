//! The `bash {cmd}` step's hooks and log (M8b decision 37): a Claude `Bash` tool call
//! whose command every matching `PreToolUse` group may rewrite through
//! `hookSpecificOutput.updatedInput`, as M8b.1 item 3 recorded the real CLI doing.

use std::fs::OpenOptions;
use std::io::Write;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::runtime::Runtime;

/// The command that runs: each `PreToolUse` group whose matcher is empty or matches
/// `Bash` runs its commands in order, each with the payload of the original command on
/// stdin, and the last `hookSpecificOutput.updatedInput.command` printed wins.
pub fn rewrite(runtime: &Runtime, cmd: &str, session: &str) -> Result<String> {
    let mut command = cmd.to_owned();
    let groups = runtime.groups("PreToolUse").iter();
    for group in groups.filter(|group| group.matches("Bash")) {
        for hook in &group.commands {
            let mut payload = json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": cmd},
                "session_id": session,
            });
            crate::fill_hook_payload(&mut payload, "PreToolUse")?;
            // Like the real CLI, a hook that fails (exits non-zero or times out) is a
            // non-blocking error: the tool runs as it was.
            let printed = match crate::run_hook_output(hook, &payload) {
                Ok(printed) => printed,
                Err(error) => {
                    crate::diag::say!("fake-agent: PreToolUse hook failed: {error:#}");
                    continue;
                }
            };
            if let Some(updated) = updated_command(&printed) {
                command = updated;
            }
        }
    }
    Ok(command)
}

/// `hookSpecificOutput.updatedInput.command` in a hook's stdout: the whole output as
/// one JSON object, or else its last line that is one.
fn updated_command(printed: &str) -> Option<String> {
    let whole = serde_json::from_str::<Value>(printed.trim()).ok();
    let value = whole.or_else(|| {
        printed
            .lines()
            .rev()
            .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
    })?;
    let command = &value["hookSpecificOutput"]["updatedInput"]["command"];
    command.as_str().map(str::to_owned)
}

/// Appends `{"original","ran","exit","output_lines"}` to `$FAKE_AGENT_BASH_LOG`, when
/// it is set.
pub fn log(original: &str, ran: &str, exit: i32, output: &str) -> Result<()> {
    let Some(path) = std::env::var_os("FAKE_AGENT_BASH_LOG") else {
        return Ok(());
    };
    let entry = json!({
        "original": original,
        "ran": ran,
        "exit": exit,
        "output_lines": output.lines().collect::<Vec<_>>(),
    });
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.to_string_lossy()))?;
    file.write_all(format!("{entry}\n").as_bytes())
        .context("append to the bash log")
}

#[cfg(test)]
mod tests {
    use super::updated_command;

    #[test]
    fn finds_the_updated_command_in_a_hooks_output() {
        let object = r#"{"hookSpecificOutput":{"updatedInput":{"command":"b"}}}"#;
        assert_eq!(updated_command(object), Some("b".into()));
        assert_eq!(
            updated_command(&format!("noise\n{object}\n")),
            Some("b".into())
        );
        assert_eq!(updated_command(""), None);
        assert_eq!(updated_command(r#"{"decision":"approve"}"#), None);
    }
}
