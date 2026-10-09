//! Client requests (M8a.22): each becomes an engine event and waits for its reply.
//! Three read git before the engine sees them: `run start`
//! (decisions 15, 17, 50, 53 and 56), `run accept`/`discard`'s confirmation and moved
//! base (decision 20), and `run resume --rebaseline` (decision 21). Every git read runs
//! on `spawn_blocking`; the engine lock is taken only to read a run's fields.

use std::path::PathBuf;
use std::time::Duration;

use proto::run_wire::request;
use proto::{ActionKind, BaseMovedInfo, FinishAction, RunReply, RunRequest};

use super::RunService;
use super::adapt::BuildError;
use super::build::Shape;
use super::delivery::{DeliveryRequestOf, DeliveryStart};
use super::design_io::DocQuery;
use crate::run::design::{GoalOrigin, mode_for};
use crate::run::engine::actions::{self, ActionNode};
use crate::run::engine::{EventKind, OrchEvent};
use crate::run::git::{self, Git};
use crate::run::model::Run;
use crate::run::plan::parse_plan_file;
use crate::run::validate::EditScope;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|error| format!("a blocking step did not finish: {error}"))?
}

impl RunService {
    /// Answers every `RunRequest` but `Subscribe` and `Unsubscribe` (`server/run_api.rs`).
    pub async fn request(&self, req: RunRequest) -> RunReply {
        let answer = |label: &str, result: Result<String, String>| match result {
            Ok(message) => RunReply::done(label, message),
            Err(message) => RunReply::refused(label, message),
        };
        match req {
            RunRequest::Start {
                plan_toml,
                dir,
                yes,
                trust_project,
                unconfined_checks,
                // Milestone 9.2 decision 3: `--delivery`, resolved once, at the start.
                delivery,
                // Milestone 9.6 (DF §1): a plan file never runs the design flow.
                design,
            } => {
                // A plan file is off whatever the config says, so its default is enough.
                let config = &config::DesignConfig::default();
                if let Err(refusal) = mode_for(GoalOrigin::PlanFile, design, config) {
                    return RunReply::refused(request::START, refusal);
                }
                let flags = (yes, trust_project, unconfined_checks);
                self.start(plan_toml, dir, flags, delivery).await
            }
            RunRequest::Approve { run_id } => answer(
                request::APPROVE,
                self.ask(|reply| EventKind::Approve { reply, run_id }).await,
            ),
            RunRequest::Reject { run_id } => answer(
                request::REJECT,
                self.ask(|reply| EventKind::Reject { reply, run_id }).await,
            ),
            RunRequest::Edit {
                run_id,
                edits,
                submit,
            } => answer(request::EDIT, self.edit(run_id, edits, submit).await),
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
            RunRequest::Tool(call) => self.tool(call).await,
            // Milestone 8b: `run start --goal` (decision 22), `run promote` (25), `run
            // stats` (35) and `anthrex profile` (10); the logic is in `driver/adapt.rs`.
            RunRequest::StartGoal {
                goal,
                dir,
                yes,
                trust_project,
                unconfined_checks,
                orchestrator,
                delivery,
                continue_from: None,
                design,
            } => {
                let flags = (trust_project, unconfined_checks);
                let choices = (orchestrator, delivery, design);
                self.start_goal(goal, dir, flags, yes, choices).await
            }
            // Milestone 9.3 decision 22: a next goal on a chain's orchestrator, which
            // keeps its runtime and model (`orchestrator` is ignored).
            RunRequest::StartGoal {
                goal,
                dir,
                yes,
                trust_project,
                unconfined_checks,
                delivery,
                continue_from: Some(after),
                design,
                ..
            } => {
                let next = super::orch::Next {
                    goal,
                    dir,
                    trust_project,
                    unconfined_checks,
                    yes,
                    delivery,
                    design,
                };
                self.continue_request(&after, next).await
            }
            RunRequest::Promote {
                run_id,
                orchestrator,
            } => self.promote(run_id, orchestrator).await,
            // Milestone 9.5 decisions 11 and 48.
            RunRequest::Stats {
                dir,
                apply,
                dismiss,
                read_only,
            } => self.stats(dir, apply, dismiss, read_only).await,
            RunRequest::Profile(profile) => self.profile(profile).await,
            // Milestone 9 decision 28's approval holds: only the user's requests decide.
            RunRequest::ApproveHold { run_id, hold } => self.hold_verdict(run_id, hold, true).await,
            RunRequest::RejectHold { run_id, hold } => self.hold_verdict(run_id, hold, false).await,
            // Milestone 9.0.5 decision 7: answered from memory.
            RunRequest::TaskDetail { run_id, task_id } => self.task_detail(&run_id, &task_id),
            // Milestone 9.0.6 decision 25 (`driver/settings.rs`).
            RunRequest::Settings(request) => self.settings(request).await,
            // Milestone 9.2 decision 25 (`driver/delivery.rs`).
            RunRequest::Deliver { run_id, stage } => {
                let req = DeliveryRequestOf::Deliver { run_id, stage };
                self.delivery_request(req).await
            }
            RunRequest::Watch { run_id, on } => {
                let req = DeliveryRequestOf::Watch { run_id, on };
                self.delivery_request(req).await
            }
            // Milestone 9.3 decision 10: a round of a settled run.
            // Milestone 9.6 decision 28: with the round's design.
            RunRequest::Iterate { run, goal, design } => answer(
                request::ITERATE,
                self.ask(|reply| EventKind::Iterate {
                    reply,
                    run_id: run,
                    goal,
                    design,
                })
                .await,
            ),
            // Milestone 9.5 decision 38: only the run's orchestrator window, as the
            // manager has it, tells the engine; any other notice changes nothing.
            // Whole-branch review C, m-1: a chain's run is read as the chain's current
            // run, as a chain window's tool calls are (an adopted window's server keeps
            // the `--run` it was launched with).
            RunRequest::McpReady { run_id, window_id } => {
                let run_id = self.chain_current(&run_id).unwrap_or(run_id);
                let ours = self.manager.list().iter().any(|w| {
                    w.id == window_id
                        && w.run.as_ref().is_some_and(|r| {
                            r.run_id == run_id && r.role == proto::AgentRole::Orchestrator
                        })
                });
                if ours {
                    self.send(EventKind::Orch(OrchEvent::McpReady { run_id, window_id }));
                }
                RunReply::done(request::MCP_READY, "")
            }
            // Milestone 9.6 decision 7: an action at a document gate.
            RunRequest::DocGate { run, kind, action } => answer(
                request::DOC_GATE,
                (self.ask(|reply| EventKind::DocGate {
                    reply,
                    run_id: run,
                    kind,
                    action,
                }))
                .await,
            ),
            // Milestone 9.6 (DF §7): one document version, read off the engine.
            RunRequest::ShowDoc {
                run,
                kind,
                version,
                diff,
                findings,
            } => {
                let from = None;
                let query = DocQuery {
                    kind,
                    version,
                    from,
                    draft: None,
                    diff,
                    findings,
                    reviewer: false,
                };
                self.show_doc(run, query).await
            }
            RunRequest::Subscribe | RunRequest::Unsubscribe => {
                RunReply::refused("run", "subscriptions are answered by the connection")
            }
        }
    }

    /// The current run of `run_id`'s chain, when it is one of the chain's runs. The
    /// engine lock is taken only for the lookup.
    fn chain_current(&self, run_id: &str) -> Option<String> {
        let state = crate::lock(&self.state); // lookup
        let chain = state.runs.get(run_id)?.chain.as_deref()?;
        let chain = state.chains.get(chain)?;
        (chain.runs.iter().any(|r| r == run_id)).then(|| chain.current().to_string())
    }

    /// Milestone 9.0.5 decision 7: a task's brief, acceptance and worker summary, built
    /// by the pure `snapshot_detail::task_detail` under the engine lock, which is dropped
    /// before the reply is made. No file, git or process access, and no `.await`.
    pub(super) fn task_detail(&self, run_id: &str, task_id: &str) -> RunReply {
        let detail = crate::lock(&self.state)
            .runs
            .get(run_id)
            .and_then(|run| crate::run::snapshot_detail::task_detail(run, task_id));
        match detail {
            Some(detail) => RunReply::TaskDetail {
                detail: Box::new(detail),
                request_id: None,
            },
            None => RunReply::refused(
                request::TASK_DETAIL,
                format!("run {run_id} has no task {task_id}"),
            ),
        }
    }

    /// `run start`: preflight, the protected files, the id, the run, decision 53's and
    /// decision 50's checks, then `Start`.
    pub(super) async fn start(
        &self,
        plan_toml: String,
        dir: PathBuf,
        flags: (bool, bool, bool),
        delivery: Option<proto::DeliveryMode>,
    ) -> RunReply {
        let refused = |message: String| RunReply::refused(request::START, message);
        match self.build(plan_toml, dir, flags, delivery).await {
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
                        RunReply::Started {
                            run_id,
                            state,
                            request_id: None,
                        }
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
        flags: (bool, bool, bool),
        delivery: Option<proto::DeliveryMode>,
    ) -> Result<Run, String> {
        let plan = parse_plan_file(&plan_toml)?;
        let delivery = DeliveryStart::Resolve(delivery);
        let once = super::build::TuneOnce::new();
        self.build_delivered(plan, dir, flags, Shape::PlanFile, delivery, &once)
            .await
            .map_err(BuildError::text)
    }

    /// `run edit`, with its [`Self::runtime_refusals`].
    async fn edit(
        &self,
        run_id: String,
        edits: Vec<proto::PlanEdit>,
        submit: bool,
    ) -> Result<String, String> {
        // Milestone 9 decision 42e: a refresh's clean-tree check, before the engine.
        self.refresh_precheck(&run_id, &edits, !submit).await?;
        let refusals = self.runtime_refusals(&run_id, &edits).await?;
        self.ask(|reply| EventKind::Edit {
            reply,
            run_id,
            edits,
            scope: EditScope::Run,
            refusals,
            submit,
        })
        .await
    }

    /// `run resume`; with `rebaseline`, the base and run heads read first (decision 21).
    pub(super) async fn resume(&self, run_id: String, rebaseline: bool) -> Result<String, String> {
        let rebaseline = if rebaseline {
            let (root, base_branch, run_branch, timeout) = self.run_refs_of(&run_id)?;
            // Milestone 9.1 decision 47: a `Multi` run's every created stage too.
            let stages: Vec<(u16, String)> = crate::lock(&self.state)
                .runs
                .get(&run_id)
                .filter(|run| run.stage_layout == crate::run::model::StageLayout::Multi)
                .map(|run| run.stages.iter().map(|s| (s.n, s.branch.clone())).collect())
                .unwrap_or_default();
            // Controller ruling C-16: only a run the engine will resume (halted or
            // paused) has its refs written; any other only has them read.
            let (project, may_move) = crate::lock(&self.state)
                .runs
                .get(&run_id)
                .map(|run| {
                    let may_move =
                        matches!(run.state, proto::RunState::Halted | proto::RunState::Paused);
                    (run.project.clone(), may_move)
                })
                .unwrap_or_else(|| (root.clone(), false));
            let refs = super::stage_ops::RunRefs {
                run_id: run_id.clone(),
                root,
                project,
                base_branch,
                run_branch,
                stages,
                timeout,
                may_move,
                now: super::unix_now(),
            };
            let heads = super::stage_ops::rebaseline(self, refs).await?;
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

    /// `run accept` and `run discard` (decision 20): the engine's predicate, then the
    /// confirmation; for accept, the base branch read and classified before the engine
    /// sees the request.
    pub(super) async fn finish(
        &self,
        run_id: String,
        action: FinishAction,
        confirm: Option<String>,
    ) -> RunReply {
        let refused = |message: String| RunReply::refused(request::FINISH, message);
        let Ok((root, base_branch, run_branch, timeout)) = self.run_refs_of(&run_id) else {
            return refused(format!("unknown run {run_id}"));
        };
        // Milestone 9.0.6 decision 41: the engine's own predicate first, before any git
        // read or confirmation, with the handler's text.
        let kind = match action {
            FinishAction::Accept => ActionKind::Accept,
            FinishAction::Discard => ActionKind::Discard,
        };
        let (base_sha, run_head) = match crate::lock(&self.state).runs.get(&run_id) {
            Some(run) => match actions::check(run, &ActionNode::Run, &kind) {
                Ok(()) => (run.base_sha.clone(), run.run_head.clone()),
                Err(text) => return refused(text),
            },
            None => Default::default(),
        };
        if action == FinishAction::Discard {
            if confirm.as_deref() != Some(run_id.as_str()) {
                return RunReply::ConfirmNeeded {
                    prompt: format!(
                        "discard run {run_id}: remove its worktrees and delete its branches?"
                    ),
                    run_id,
                    base_moved: None,
                    request_id: None,
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
                request_id: None,
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
                request_id: None,
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
            Ok(message) => RunReply::done(request::FINISH, message),
            Err(message) => RunReply::refused(request::FINISH, message),
        }
    }
}

#[cfg(test)]
#[path = "requests_tests.rs"]
mod tests;
