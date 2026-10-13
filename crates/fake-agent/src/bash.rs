//! The `bash {cmd}` step's hooks and log (M8b decision 37): a Claude `Bash` tool call
//! whose command every matching `PreToolUse` group may rewrite through
//! `hookSpecificOutput.updatedInput`, as M8b.1 item 3 recorded the real CLI doing.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::runtime::Runtime;

/// The opt-in variable of milestone 9.14's tool ids and `PostToolUse` (Interfaces,
/// `fake-agent`). Without it the `bash` step is byte-for-byte milestone 8b's.
const TOOL_IDS_VAR: &str = "FAKE_AGENT_TOOL_IDS";

/// The last `fake-tu-<n>` handed out by this process.
static LAST_TOOL_ID: AtomicU32 = AtomicU32::new(0);

/// The next call's `tool_use_id`, `fake-tu-<n>` with `n` counting from 1 per process,
/// when `FAKE_AGENT_TOOL_IDS=1`; otherwise `None`.
pub fn next_tool_id() -> Option<String> {
    let on = std::env::var(TOOL_IDS_VAR).is_ok_and(|value| value == "1");
    on.then(|| {
        format!(
            "fake-tu-{}",
            LAST_TOOL_ID.fetch_add(1, Ordering::SeqCst) + 1
        )
    })
}

/// The command that runs: each `PreToolUse` group whose matcher is empty or matches
/// `Bash` runs its commands in order, each with the payload of the original command on
/// stdin (and `tool_use_id`, when there is one), and the last
/// `hookSpecificOutput.updatedInput.command` printed wins.
pub fn rewrite(runtime: &Runtime, cmd: &str, session: &str, id: Option<&str>) -> Result<String> {
    let mut command = cmd.to_owned();
    let groups = runtime.groups("PreToolUse").iter();
    for group in groups.filter(|group| group.matches("Bash")) {
        for hook in &group.hooks {
            let mut payload = json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": cmd},
                "session_id": session,
            });
            if let Some(id) = id {
                payload["tool_use_id"] = json!(id);
            }
            crate::fill_hook_payload(&mut payload, "PreToolUse")?;
            // Like the real CLI, a hook that fails (exits non-zero or times out) is a
            // non-blocking error: the tool runs as it was.
            let printed = match crate::run_hook_output(&hook.command, &payload, hook.timeout) {
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

/// After the command ran, with a `tool_use_id` only: every `PostToolUse` group that
/// matches `Bash` runs its commands with the call's id, original command, stdout and
/// exit code. What they print is dropped, and a failing one is only reported.
pub fn post_tool_use(
    runtime: &Runtime,
    (cmd, id, session): (&str, &str, &str),
    stdout: &str,
    exit_code: i32,
) -> Result<()> {
    let groups = runtime.groups("PostToolUse").iter();
    for group in groups.filter(|group| group.matches("Bash")) {
        for hook in &group.hooks {
            let mut payload = json!({
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "tool_use_id": id,
                "tool_input": {"command": cmd},
                "tool_response": {"stdout": stdout, "exit_code": exit_code},
                "session_id": session,
            });
            crate::fill_hook_payload(&mut payload, "PostToolUse")?;
            if let Err(error) = crate::run_hook_output(&hook.command, &payload, hook.timeout) {
                crate::diag::say!("fake-agent: PostToolUse hook failed: {error:#}");
            }
        }
    }
    Ok(())
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
