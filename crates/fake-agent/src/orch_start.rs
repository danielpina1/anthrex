//! Milestone 9.5 decision 31 (task M9.5.5a): how a PTY session launched with no prompt
//! after `--` starts, split out of `orch_steps.rs` to keep it under 600 lines.

use std::io;

use anyhow::Result;
use serde_json::json;

use super::{Pty, RawMode, text_of};
use crate::mcp;
use crate::roles;
use crate::script::Step;

/// Fix round 1 (m3): a session with no `--settings` before `--` is Codex.
pub(super) fn is_codex(args: &[String]) -> bool {
    let config = args.split(|arg| arg == "--").next().unwrap_or_default();
    !config.iter().any(|arg| arg == "--settings")
}

impl Pty {
    /// Milestone 9.5 decision 31: launched with no prompt (as the daemon launches an
    /// orchestrator since decision 38), the session lists the anthrex server's tools,
    /// shows it has started, and, unless it resumes, reads its first message: a prompt,
    /// recorded as `first_message` (not a `read_message` read) and kept as
    /// `FAKE_AGENT_RESULT` `{"first_message": <text>}` for `expect`. Claude starts with
    /// its `SessionStart` hook. Fix round 1 (m3): Codex, as the daemon launches it (hooks
    /// configured, the status title on), starts with its ready title, and runs
    /// `SessionStart` only with its first turn, just before `UserPromptSubmit` (M3.11).
    pub(super) fn start(&mut self, fresh: bool, codex: bool) -> Result<Option<i32>> {
        mcp::list_tools(&self.server)?;
        let source = if fresh { "startup" } else { "resume" };
        let hook = self.hooks.hook("SessionStart").map(str::to_owned);
        match &hook {
            Some(command) if !codex => self.session_start(command, source)?,
            _ => {
                let title = Step::Title("Ready".into());
                crate::run_step(title, &self.hooks, &mut self.input, &mut io::stdout())?;
            }
        }
        if !fresh {
            return Ok(None);
        }
        let raw_mode = RawMode::enter();
        let (raw, _) = match self.read_raw(None)? {
            Ok(read) => read,
            Err(_) => return Ok(Some(0)),
        };
        drop(raw_mode);
        let text = text_of(&raw);
        let line = json!({"at": crate::headless::timestamp(), "first_message": text});
        roles::record_stdin(&self.script.name, &line.to_string())?;
        if let Some(command) = hook.filter(|_| codex) {
            self.session_start(&command, source)?;
        }
        self.submitted(&text)?;
        self.vars.result = json!({"first_message": text}).to_string();
        self.script.save_vars(&self.vars)?;
        self.message = text;
        Ok(None)
    }

    fn session_start(&self, command: &str, source: &str) -> Result<()> {
        let mut payload = json!({"session_id": self.session, "source": source});
        crate::fill_hook_payload(&mut payload, "SessionStart")?;
        crate::run_hook(command, &payload)
    }
}
