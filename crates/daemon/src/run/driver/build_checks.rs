//! `run start`'s runtime checks and id draw, moved out of `build.rs` (task M9.15.6's
//! preparatory move, ruling C5; no behaviour change): decisions 50 and 53's
//! [`RunService::check_runtimes`] and decision 15's [`RunService::pick_id`].

use std::time::Duration;

use proto::Runtime;

use super::super::RunService;
use super::super::ops::blocking;
use crate::run::chain::suffix_taken;
use crate::run::git;
use crate::run::journal::runs_dir;
use crate::run::model::{ClaudeAuth, Run};
use crate::run::plan::{random_suffix, run_id_taken, slug};

/// Decision 15: an id is redrawn at most this many times.
const ID_DRAWS: usize = 5;

/// Decision 50's refusal.
const API_KEY_NEEDED: &str = "[orchestrator.claude] auth = \"api_key\" needs ANTHROPIC_API_KEY in the daemon's environment or orchestrator.claude.api_key_helper";

/// What [`RunService::check_runtimes`] found: decision 50's refusal, and each runtime's
/// project settings (decision 53) with the name its refusal gives it.
#[derive(Default)]
pub(super) struct RuntimeChecks {
    pub(super) api_key: Vec<(Runtime, String)>,
    pub(super) settings: Vec<(Runtime, &'static str, Vec<String>)>,
}

impl RunService {
    /// Decisions 50 and 53 for `runtimes`: decision 50's refusal when Claude is among
    /// them and has no key, and the project settings each runtime's sessions would load
    /// unasked, by the caps the sessions are launched with. Claude's only when it cannot
    /// exclude them; Codex's only when it loads project config with no exclusion.
    pub(super) async fn check_runtimes(
        &self,
        run: &Run,
        runtimes: &[Runtime],
        timeout: Duration,
    ) -> Result<RuntimeChecks, String> {
        let mut checks = RuntimeChecks::default();
        let claude = runtimes.contains(&Runtime::Claude);
        if claude
            && run.limits.claude_auth == ClaudeAuth::ApiKey
            && std::env::var_os("ANTHROPIC_API_KEY").is_none()
            && run.limits.api_key_helper.is_none()
        {
            checks
                .api_key
                .push((Runtime::Claude, API_KEY_NEEDED.to_string()));
        }
        let caps = self.ctx.cli_caps;
        let codex = runtimes.contains(&Runtime::Codex)
            && caps.codex_loads_project_config
            && caps.codex_user_config_only.is_none();
        let (root, base) = (run.root.clone(), run.base_sha.clone());
        if claude && caps.claude_user_settings_only.is_none() {
            let (g, r, b) = (self.ctx.git.clone(), root.clone(), base.clone());
            let paths =
                blocking(move || git::project_settings(&g, &r, &b, true, None, timeout)).await?;
            checks.settings.push((Runtime::Claude, "Claude", paths));
        }
        if codex {
            let (g, paths) = (self.ctx.git.clone(), caps.codex_project_config_paths);
            let paths = blocking(move || {
                git::project_settings(&g, &root, &base, false, Some(paths), timeout)
            })
            .await?;
            checks.settings.push((Runtime::Codex, "Codex", paths));
        }
        Ok(checks)
    }

    /// Decision 15: the slug of `head` and a random suffix, redrawn while the id's
    /// branches, its data directory or an engine run already take it, or (milestone
    /// 9.3) a chain already has its suffix (`chain::suffix_taken`). `head` is the goal,
    /// or the `run_name` decider's slug (the run title change, `RunName::id_head`).
    pub(in crate::run::driver) fn pick_id(
        &self,
        head: &str,
        refs: &[String],
    ) -> Result<String, String> {
        for _ in 0..ID_DRAWS {
            let id = slug(head, random_suffix());
            let taken =
                run_id_taken(&id, refs) || runs_dir(&self.ctx.data_dir).join(&id).exists() || {
                    let state = crate::lock(&self.state);
                    state.runs.contains_key(&id) || suffix_taken(&state.chains, &state.runs, &id)
                };
            if !taken {
                return Ok(id);
            }
        }
        Err("could not pick a free run id".to_string())
    }
}
