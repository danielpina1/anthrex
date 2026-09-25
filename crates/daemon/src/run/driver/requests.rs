//! Client requests (M8a.22): each becomes an engine event and waits for its reply.
//! Three read git before the engine sees them: `run start`
//! (decisions 15, 17, 50, 53 and 56), `run accept`/`discard`'s confirmation and moved
//! base (decision 20), and `run resume --rebaseline` (decision 21). Every git read runs
//! on `spawn_blocking`; the engine lock is taken only to read a run's fields.

use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::run_wire::request;
use proto::{BaseMovedInfo, FinishAction, RunReply, RunRequest, Runtime};

use super::{RunService, unix_now};
use crate::run::confine;
use crate::run::engine::EventKind;
use crate::run::git::{self, Git, os};
use crate::run::globs::ProtectedMatcher;
use crate::run::journal::runs_dir;
use crate::run::model::{ClaudeAuth, Run};
use crate::run::plan::{
    BuildContext, parse_plan, random_suffix, resolve_profile, run_id_taken, slug,
};
use crate::run::reach::{edits_may_widen, reachable_runtimes};
use crate::run::validate::EditScope;
use crate::worktree::repo_worktrees_dir;

/// Decision 15: an id is redrawn at most this many times.
const ID_DRAWS: usize = 5;

/// Decision 50's refusal.
const API_KEY_NEEDED: &str = "[orchestrator.claude] auth = \"api_key\" needs ANTHROPIC_API_KEY in the daemon's environment or orchestrator.claude.api_key_helper";

/// Decision 53's refusal for the project settings a headless `who` session would run.
fn settings_refusal(who: &str, paths: &[String]) -> String {
    format!(
        "this repository has project settings that headless {who} sessions would run without asking: {}; review them, then start again with --trust-project",
        paths.join(", ")
    )
}

/// Decision 53's refusal for an edit that would reach `who` (T22-P1, F4): the run's
/// start trusted only the settings it checked, and `run edit` has no `--trust-project`.
fn edit_settings_refusal(who: &str, paths: &[String]) -> String {
    format!(
        "this edit would start headless {who} sessions, and this repository has project settings they would run without asking: {}; a run trusts only what its start checked, so review them and start a new run with --trust-project",
        paths.join(", ")
    )
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|error| format!("a blocking step did not finish: {error}"))?
}

/// A random 64-bit nonce from the standard library's per-process random keys.
fn random_nonce() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(unix_now());
    hasher.finish().max(1)
}

/// What [`RunService::check_runtimes`] found: decision 50's refusal, and each runtime's
/// project settings (decision 53) with the name its refusal gives it.
#[derive(Default)]
struct RuntimeChecks {
    api_key: Vec<(Runtime, String)>,
    settings: Vec<(Runtime, &'static str, Vec<String>)>,
}

/// Every branch under `refs/heads/anthrex/` (decision 15's redraw test).
fn run_refs(git: &OsString, root: &Path, timeout: Duration) -> Result<Vec<String>, String> {
    let listing = Git::new(git, timeout).ok(
        root,
        &[
            os("for-each-ref"),
            os("--format=%(refname)"),
            os("refs/heads/anthrex/"),
        ],
    )?;
    Ok(listing.lines().map(str::to_string).collect())
}

impl RunService {
    /// Answers every `RunRequest` but `Subscribe` and `Unsubscribe` (`server/run_api.rs`).
    pub async fn request(&self, req: RunRequest) -> RunReply {
        let answer = |label: &str, result: Result<String, String>| match result {
            Ok(message) => RunReply::Done {
                request: label.to_string(),
                message,
            },
            Err(message) => RunReply::Refused {
                request: label.to_string(),
                message,
            },
        };
        match req {
            RunRequest::Start {
                plan_toml,
                dir,
                yes,
                trust_project,
                unconfined_checks,
            } => {
                self.start(plan_toml, dir, yes, trust_project, unconfined_checks)
                    .await
            }
            RunRequest::Approve { run_id } => answer(
                request::APPROVE,
                self.ask(|reply| EventKind::Approve { reply, run_id }).await,
            ),
            RunRequest::Reject { run_id } => answer(
                request::REJECT,
                self.ask(|reply| EventKind::Reject { reply, run_id }).await,
            ),
            RunRequest::Edit { run_id, edits } => {
                answer(request::EDIT, self.edit(run_id, edits).await)
            }
            RunRequest::Retry { run_id, task_id } => answer(
                request::RETRY,
                self.ask(|reply| EventKind::Retry {
                    reply,
                    run_id,
                    task_id,
                })
                .await,
            ),
            RunRequest::Override {
                run_id,
                task_id,
                reason,
            } => answer(
                request::OVERRIDE,
                self.ask(|reply| EventKind::Override {
                    reply,
                    run_id,
                    task_id,
                    reason,
                })
                .await,
            ),
            RunRequest::Cancel { run_id } => answer(
                request::CANCEL,
                self.ask(|reply| EventKind::Cancel { reply, run_id }).await,
            ),
            RunRequest::Resume { run_id, rebaseline } => {
                answer(request::RESUME, self.resume(run_id, rebaseline).await)
            }
            RunRequest::Finish {
                run_id,
                action,
                confirm,
            } => self.finish(run_id, action, confirm).await,
            RunRequest::List => RunReply::Snapshot(self.current()),
            RunRequest::Tool(call) => {
                // Every text here reaches an agent verbatim (M8a.19): the engine's own
                // tool answers, worded for an agent.
                match self.ask(|reply| EventKind::Tool { reply, call }).await {
                    Ok(text) => RunReply::ToolResult { ok: true, text },
                    Err(text) => RunReply::ToolResult { ok: false, text },
                }
            }
            RunRequest::Subscribe | RunRequest::Unsubscribe => RunReply::Refused {
                request: "run".to_string(),
                message: "subscriptions are answered by the connection".to_string(),
            },
        }
    }

    /// `run start`: preflight, the protected files, the id, the run, decision 53's and
    /// decision 50's checks, then `Start`.
    pub(super) async fn start(
        &self,
        plan_toml: String,
        dir: PathBuf,
        yes: bool,
        trust_project: bool,
        unconfined_checks: bool,
    ) -> RunReply {
        let refused = |message: String| RunReply::Refused {
            request: request::START.to_string(),
            message,
        };
        match self
            .build(plan_toml, dir, yes, trust_project, unconfined_checks)
            .await
        {
            Ok(run) => {
                let run_id = run.id.clone();
                match self
                    .ask(|reply| EventKind::Start {
                        reply,
                        run: Box::new(run),
                    })
                    .await
                {
                    Ok(id) => {
                        let state = crate::lock(&self.state)
                            .runs
                            .get(&id)
                            .map_or(proto::RunState::AwaitingApproval, |run| run.state);
                        RunReply::Started { run_id, state }
                    }
                    Err(message) => refused(message),
                }
            }
            Err(message) => refused(message),
        }
    }

    async fn build(
        &self,
        plan_toml: String,
        dir: PathBuf,
        yes: bool,
        trust_project: bool,
        unconfined_checks: bool,
    ) -> Result<Run, String> {
        let config = self.ctx.orchestrator.clone();
        // Final fix batch F1c round 2: never run worker-written code unconfined unless
        // the user said so, on the command line or in their own config.
        let available = confine::available();
        let allowed = unconfined_checks || config.unconfined_checks;
        if let Some(refusal) = confine::start_refusal(config.worker_sandbox, available, allowed) {
            return Err(refusal);
        }
        let plan = parse_plan(&plan_toml)?;
        let timeout = Duration::from_secs(config.git_timeout_secs);
        let git = self.ctx.git.clone();
        let g = git.clone();
        let mut pre = blocking(move || git::preflight(&g, &dir, timeout)).await?;

        // Decision 17 (carry): the protected files, by the resolved profile's matcher,
        // before `build_run` turns them into the plan's warnings (decision 56).
        let profile = resolve_profile(&plan.profile, &config.profile);
        let matcher = ProtectedMatcher::new(&profile.protected)?;
        let (g, root, base) = (git.clone(), pre.root.clone(), pre.base_sha.clone());
        pre.protected_files =
            blocking(move || git::protected_files(&g, &root, &base, &matcher, timeout)).await?;

        let (g, root) = (git.clone(), pre.root.clone());
        let refs = blocking(move || run_refs(&g, &root, timeout)).await?;
        let id = self.pick_id(&plan.goal, &refs)?;
        let wt_dir = repo_worktrees_dir(&self.ctx.worktrees_root, &pre.project);
        let ctx = BuildContext {
            id: id.clone(),
            wt_dir,
            data_dir: runs_dir(&self.ctx.data_dir).join(&id),
            config: &config,
            now: unix_now(),
            yes,
        };
        let mut run = crate::run::plan::build_run(plan, pre, ctx).map_err(|errors| {
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        })?;
        run.limits.unconfined_checks = run.limits.worker_sandbox && !available;
        run.session_nonce = random_nonce();
        run.codex_project_config = Some(self.ctx.cli_caps.codex_project_config());
        // Final fix batch F2 (C-I1): the base's `.codex`, which every Codex session's
        // checkout must still hold when its process starts.
        let (g, root, base) = (git.clone(), run.root.clone(), run.base_sha.clone());
        run.codex_config_base =
            blocking(move || git::codex_config_tree(&g, &root, &base, timeout)).await?;

        let runtimes = reachable_runtimes(&run);
        let checks = self.check_runtimes(&run, &runtimes, timeout).await?;
        if let Some((_, text)) = checks.api_key.first() {
            return Err(text.clone());
        }
        if !trust_project {
            let refusals: Vec<String> = checks
                .settings
                .iter()
                .filter(|(_, _, paths)| !paths.is_empty())
                .map(|(_, who, paths)| settings_refusal(who, paths))
                .collect();
            if !refusals.is_empty() {
                return Err(refusals.join("\n"));
            }
        }
        run.trusted_project = checks
            .settings
            .into_iter()
            .flat_map(|(_, _, paths)| paths)
            .collect();
        run.trusted_project.sort();
        run.trusted_project.dedup();
        Ok(run)
    }

    /// `run edit` (ruling T22-I1b): decisions 50 and 53 for each runtime the run cannot
    /// reach yet, so the engine refuses an edit that would reach one whose checks fail
    /// with that check's text. Project settings already trusted at `run start` pass.
    /// Only a batch that adds, splits or amends a task is probed (T22-P2, F4).
    async fn edit(&self, run_id: String, edits: Vec<proto::PlanEdit>) -> Result<String, String> {
        let run = crate::lock(&self.state).runs.get(&run_id).cloned();
        let mut refusals = Vec::new();
        if let Some(run) = run.filter(|_| edits_may_widen(&edits)) {
            let reachable = reachable_runtimes(&run);
            let unreached: Vec<Runtime> = [Runtime::Claude, Runtime::Codex]
                .into_iter()
                .filter(|r| !reachable.contains(r))
                .collect();
            let timeout = Duration::from_secs(self.ctx.orchestrator.git_timeout_secs);
            let checks = self.check_runtimes(&run, &unreached, timeout).await?;
            refusals = checks.api_key;
            for (runtime, who, paths) in checks.settings {
                let trusted = paths.iter().all(|p| run.trusted_project.contains(p));
                if !trusted && !refusals.iter().any(|(r, _)| *r == runtime) {
                    refusals.push((runtime, edit_settings_refusal(who, &paths)));
                }
            }
        }
        self.ask(|reply| EventKind::Edit {
            reply,
            run_id,
            edits,
            scope: EditScope::Run,
            refusals,
        })
        .await
    }

    /// Decisions 50 and 53 for `runtimes`: decision 50's refusal when Claude is among
    /// them and has no key, and the project settings each runtime's sessions would load
    /// unasked, by the caps the sessions are launched with. Claude's only when it cannot
    /// exclude them; Codex's only when it loads project config with no exclusion.
    async fn check_runtimes(
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

    /// Decision 15: the slug and a random suffix, redrawn while the id's branches, its
    /// data directory or an engine run already take it.
    fn pick_id(&self, goal: &str, refs: &[String]) -> Result<String, String> {
        for _ in 0..ID_DRAWS {
            let id = slug(goal, random_suffix());
            let taken = run_id_taken(&id, refs)
                || runs_dir(&self.ctx.data_dir).join(&id).exists()
                || crate::lock(&self.state).runs.contains_key(&id);
            if !taken {
                return Ok(id);
            }
        }
        Err("could not pick a free run id".to_string())
    }

    /// `run resume`; with `rebaseline`, the base and run heads read first (decision 21).
    pub(super) async fn resume(&self, run_id: String, rebaseline: bool) -> Result<String, String> {
        let rebaseline = if rebaseline {
            let (root, base_branch, run_branch, timeout) = self.run_refs_of(&run_id)?;
            let git = self.ctx.git.clone();
            let heads = blocking(move || {
                let base_ref = format!("refs/heads/{base_branch}");
                let run_ref = format!("refs/heads/{run_branch}");
                let base = git::read_ref(&git, &root, &base_ref, timeout)?
                    .ok_or_else(|| format!("{base_ref} does not exist"))?;
                let head = git::read_ref(&git, &root, &run_ref, timeout)?
                    .ok_or_else(|| format!("{run_ref} does not exist"))?;
                Ok((base, head))
            })
            .await?;
            Some(heads)
        } else {
            None
        };
        self.ask(|reply| EventKind::Resume {
            reply,
            run_id,
            rebaseline,
        })
        .await
    }

    fn run_refs_of(&self, run_id: &str) -> Result<(PathBuf, String, String, Duration), String> {
        let state = crate::lock(&self.state);
        let run = state
            .runs
            .get(run_id)
            .ok_or_else(|| format!("unknown run {run_id}"))?;
        Ok((
            run.root.clone(),
            run.base_branch.clone(),
            run.run_branch(),
            Duration::from_secs(run.limits.git_timeout_secs),
        ))
    }

    /// `run accept` and `run discard` (decision 20): the confirmation first; for accept,
    /// the base branch read and classified before the engine sees the request.
    pub(super) async fn finish(
        &self,
        run_id: String,
        action: FinishAction,
        confirm: Option<String>,
    ) -> RunReply {
        let refused = |message: String| RunReply::Refused {
            request: request::FINISH.to_string(),
            message,
        };
        let Ok((root, base_branch, run_branch, timeout)) = self.run_refs_of(&run_id) else {
            return refused(format!("unknown run {run_id}"));
        };
        let (base_sha, run_head) = crate::lock(&self.state)
            .runs
            .get(&run_id)
            .map(|run| (run.base_sha.clone(), run.run_head.clone()))
            .unwrap_or_default();
        if action == FinishAction::Discard {
            if confirm.as_deref() != Some(run_id.as_str()) {
                return RunReply::ConfirmNeeded {
                    prompt: format!(
                        "discard run {run_id}: remove its worktrees and delete its branches?"
                    ),
                    run_id,
                    base_moved: None,
                };
            }
            return self.finish_now(run_id, action).await;
        }
        let git = self.ctx.git.clone();
        let (from, r, id) = (base_sha.clone(), root.clone(), run_id.clone());
        let base_ref = format!("refs/heads/{base_branch}");
        let classified = blocking(move || {
            let Some(to) = git::read_ref(&git, &r, &base_ref, timeout)? else {
                return Err(format!("{base_ref} does not exist"));
            };
            if to == from {
                return Ok(None);
            }
            if !git::is_ancestor(Git::new(&git, timeout), &r, &from, &to)? {
                return Err(format!(
                    "{base_ref} was rewritten since the run started ({} is not an ancestor of {}); merge {run_branch} by hand, or discard the run",
                    git::short(&from),
                    git::short(&to)
                ));
            }
            // Final fix batch F1, finding D-2: run work on the base that no accept put
            // there (only the run head, merged whole by the user, is theirs to confirm).
            let run_work =
                git::run_work_on_base(&git, &r, &id, &from, &to, Some(&run_head), timeout)?;
            if run_work > 0 {
                return Err(format!(
                    "{base_ref} contains unaccepted run work ({run_work} {}) that no accept merged; move {base_ref} off it, or discard the run",
                    if run_work == 1 { "commit" } else { "commits" }
                ));
            }
            let (commits, total) =
                git::commits_since(&git, &r, &from, &to, git::ACCEPT_LIST_MAX, timeout)?;
            Ok(Some(BaseMovedInfo {
                from,
                to,
                commits,
                total,
            }))
        })
        .await;
        let moved = match classified {
            Ok(moved) => moved,
            Err(message) => return refused(message),
        };
        let prompt = format!(
            "merge anthrex/{run_id}/integration into {base_branch} in {}?",
            root.display()
        );
        match moved {
            None if confirm.as_deref() == Some(run_id.as_str()) => {
                // Final fix batch F1, finding D-4: a base that advanced during the run
                // and came back is at `base_sha` again; the engine drops its record, so
                // accept expects `base_sha`, not a head the base no longer has.
                self.send(EventKind::BaseAdvanced {
                    run_id: run_id.clone(),
                    to: base_sha,
                    commits: 0,
                });
                self.finish_now(run_id, action).await
            }
            None => RunReply::ConfirmNeeded {
                run_id,
                prompt,
                base_moved: None,
            },
            Some(info) if confirm.as_deref() == Some(format!("{run_id}@{}", info.to).as_str()) => {
                self.send(EventKind::BaseAdvanced {
                    run_id: run_id.clone(),
                    to: info.to,
                    commits: info.total,
                });
                self.finish_now(run_id, action).await
            }
            Some(info) => RunReply::ConfirmNeeded {
                run_id,
                prompt,
                base_moved: Some(info),
            },
        }
    }

    async fn finish_now(&self, run_id: String, action: FinishAction) -> RunReply {
        match self
            .ask(|reply| EventKind::Finish {
                reply,
                run_id,
                action,
            })
            .await
        {
            Ok(message) => RunReply::Done {
                request: request::FINISH.to_string(),
                message,
            },
            Err(message) => RunReply::Refused {
                request: request::FINISH.to_string(),
                message,
            },
        }
    }
}
