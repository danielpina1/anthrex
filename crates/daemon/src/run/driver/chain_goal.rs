//! Milestone 9.3 decision 22 (KG §3.3, §3.5, §3.6): a next goal on a chain's
//! orchestrator, from `RunRequest::StartGoal { continue_from: Some(run) }` or the
//! orchestrator's `start_goal` tool. The chain is looked up and refused first, then
//! the goal's directory is checked against the chain's project, then `run start
//! --goal`'s steps 1 and 2 and the delivery's preflight run unchanged
//! (`RunService::goal_ready`), triage is skipped, and the run is built as a planned
//! run on the chain's runtime and model, carrying its chain. The engine then adopts the
//! chain's window or, once the chain has ended, launches a fresh session with the
//! handoff prompt (`engine/chains.rs::join`).
//!
//! I/O: git, the profile, the delivery's preflight and the history read run off every
//! lock, on `spawn_blocking` and bounded; the engine lock is taken only to look the
//! chain up (`// lookup`).

use std::path::PathBuf;
use std::time::Duration;

use proto::run_wire::request;
use proto::{DeliveryMode, OrchestratorChoice, Plan, RunReply, RunState};

use super::super::RunService;
use super::super::adapt::GoalReady;
use super::super::build::{Planned, Shape};
use super::super::delivery::DeliveryStart;
use crate::run::chain::continuable;
use crate::run::contract::sha7;
use crate::run::engine::{EventKind, HISTORY_FILE};
use crate::run::model::{LogEntry, Run};
use crate::run::orch::contract::orchestrator_first_prompt;
use crate::run::orch::contract_rounds::{goal_started, handoff_prompt, other_project};

/// Decision 24: how many of the chain's history lines a fresh session's first prompt
/// carries.
pub const HANDOFF_LINES: usize = 10;

/// What a continued goal is started with: the request's own fields, or, from the
/// orchestrator's tool, its previous run's (decision 22, step 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::run::driver) struct Next {
    pub goal: String,
    pub dir: PathBuf,
    pub trust_project: bool,
    pub unconfined_checks: bool,
    pub yes: bool,
    pub delivery: Option<DeliveryMode>,
}

impl Next {
    /// Decision 22, step 4, from the tool: the previous run's delivery mode, trust and
    /// unconfined checks (which its user allowed at its start), in its project; never
    /// "approve at once", so the plan stops at the gate (KG §3.3, §8).
    pub(in crate::run::driver) fn inherited(prev: &Run, goal: String) -> Next {
        Next {
            goal,
            dir: prev.project.clone(),
            trust_project: prev.trust_project,
            unconfined_checks: prev.limits.unconfined_checks,
            yes: false,
            delivery: Some(prev.delivery.mode),
        }
    }
}

/// The chain a continued goal joins, as the lookup copied it out.
struct Joined {
    chain: String,
    project: PathBuf,
    choice: OrchestratorChoice,
    handoff: Handoff,
    /// The lookup found the chain's window to adopt (fix round 1, m5: no history read).
    adopt: bool,
}

/// Decision 24: what a fresh session's first prompt says of its chain, copied out of
/// the engine state: the chain, its previous run's short id, outcome (KG §3.3's
/// `accepted`, `discarded` or, D17, `delivered`) and summary, and the history file
/// with the chain's runs.
pub(in crate::run::driver) struct Handoff {
    chain: String,
    prev_h4: String,
    outcome: &'static str,
    summary: Option<String>,
    history: PathBuf,
    runs: Vec<String>,
}

impl Handoff {
    /// The handoff of `chain` (with runs `runs`) after its run `prev`.
    pub(in crate::run::driver) fn of(chain: &str, runs: Vec<String>, prev: &Run) -> Handoff {
        Handoff {
            chain: chain.to_string(),
            prev_h4: prev.short().to_string(),
            outcome: crate::run::chain::outcome(prev),
            summary: prev
                .orch
                .orchestrator
                .as_ref()
                .and_then(|o| o.summary.clone()),
            history: prev.repo_dir.join(HISTORY_FILE),
            runs,
        }
    }

    /// The prompt after `first` (the run's own first prompt), with the chain's last
    /// history lines read within `read` ([`history_lines`]); none read when `read` is
    /// `None`.
    pub(in crate::run::driver) async fn prompt(
        &self,
        first: &str,
        read: Option<Duration>,
    ) -> String {
        let lines = match read {
            Some(bound) => history_lines(self.history.clone(), self.runs.clone(), bound).await,
            None => None,
        };
        let ended = (self.chain.as_str(), self.prev_h4.as_str(), self.outcome);
        handoff_prompt(first, ended, self.summary.as_deref(), lines.as_deref())
    }
}

impl RunService {
    /// `RunRequest::StartGoal { continue_from: Some(after) }`: `Started` in `planning`
    /// (D11: a continued goal is not triaged), or the refusal.
    pub(in crate::run::driver) async fn continue_request(
        &self,
        after: &str,
        next: Next,
    ) -> RunReply {
        match self.continue_goal(after, next).await {
            Ok(run_id) => RunReply::Started {
                run_id,
                state: RunState::Planning,
                request_id: None,
            },
            Err(message) => RunReply::refused(request::START_GOAL, message),
        }
    }

    /// The `start_goal` tool of run `last`'s orchestrator, its call resolved and its
    /// caller checked: the goal on [`Next::inherited`] options; `run <id> started`.
    pub(in crate::run::driver) async fn start_goal_tool(
        &self,
        last: &str,
        goal: String,
    ) -> Result<String, String> {
        let next = {
            let state = crate::lock(&self.state); // lookup
            let prev = state
                .runs
                .get(last)
                .ok_or_else(|| format!("unknown run {last}"))?;
            Next::inherited(prev, goal)
        };
        let run_id = self.continue_goal(last, next).await?;
        Ok(goal_started(&run_id))
    }

    /// Decision 22's steps 1 to 6 for a goal continuing from run `after`; the new run's
    /// id.
    async fn continue_goal(&self, after: &str, next: Next) -> Result<String, String> {
        let joined = self.joined(after)?;
        let timeout =
            Duration::from_secs(self.ctx.settings.current().orchestrator.git_timeout_secs);
        let (g, d) = (self.ctx.git.clone(), next.dir.clone());
        let roots =
            tokio::task::spawn_blocking(move || crate::project::detect_roots_with(&g, &d, timeout))
                .await
                .map_err(|error| format!("a blocking step did not finish: {error}"))?;
        // A directory that is no repository is preflight's to refuse, in its words.
        if roots.worktree.is_some() && roots.project != joined.project {
            return Err(other_project(&joined.chain, &joined.project));
        }
        let flags = (next.trust_project, next.unconfined_checks);
        let GoalReady {
            profile, frozen, ..
        } = self
            .goal_ready(&next.goal, &next.dir, flags, next.delivery)
            .await?;
        let plan = Plan {
            goal: next.goal.clone(),
            max_writers: None,
            max_readers: None,
            max_bounces: None,
            profile: profile.spec(),
            tasks: Vec::new(),
        };
        // Triage is skipped (D14); the chain's runtime and model hold.
        let shape = Shape::Planned(Box::new(Planned {
            triage: None,
            usage: None,
            yes: next.yes,
            choice: Some(joined.choice.clone()),
        }));
        let all = (false, next.trust_project, next.unconfined_checks);
        let done = DeliveryStart::Done(frozen);
        let mut run = self
            .build_delivered(plan, next.dir, all, shape, done)
            .await
            .map_err(|error| error.text())?;
        run.chain = Some(joined.chain.clone());
        let based = format!("based on {} at {}", run.base_branch, sha7(&run.base_sha));
        run.log.insert(
            0,
            LogEntry {
                at: run.created_at,
                text: based,
            },
        );
        // Decision 24: the first prompt of a fresh session, should the chain have
        // ended; the engine keeps the adopted session's own when it adopts. Fix round
        // 1 (m5): when the lookup found a window to adopt, the history is not read; a
        // chain that ended before the step launches with the summary alone.
        let bound = Duration::from_secs(run.limits.git_timeout_secs);
        let read = (!joined.adopt).then_some(bound);
        let prompt = joined
            .handoff
            .prompt(&orchestrator_first_prompt(&run), read)
            .await;
        if let Some(o) = run.orch.orchestrator.as_mut() {
            o.first_prompt = prompt;
        }
        let run_id = run.id.clone();
        self.ask(|reply| EventKind::Start {
            reply,
            run: Box::new(run),
        })
        .await?;
        Ok(run_id)
    }

    /// Decision 22, step 1: the chain run `after` continues, copied out of the engine
    /// state, or its refusal (`chain::continuable`).
    fn joined(&self, after: &str) -> Result<Joined, String> {
        let state = crate::lock(&self.state); // lookup
        let (chain, prev) = continuable(&state.chains, &state.runs, after)?;
        Ok(Joined {
            chain: chain.id.clone(),
            project: chain.project.clone(),
            choice: OrchestratorChoice {
                runtime: chain.runtime,
                model: (!chain.model.is_empty()).then(|| chain.model.clone()),
            },
            handoff: Handoff::of(&chain.id, chain.runs.clone(), prev),
            adopt: !chain.ended && chain.window_id != 0,
        })
    }
}

/// Decision 24: the last [`HANDOFF_LINES`] lines of the history file at `path` whose
/// `run_id` is one of `runs`, each as written (a JSON line), read on `spawn_blocking`
/// within `bound` (the run's `git_timeout_secs`). `None` when the read fails, finds no
/// line, or does not answer in time; a read that never answers is left to end on its
/// own, holding no lock.
pub(in crate::run::driver) async fn history_lines(
    path: PathBuf,
    runs: Vec<String>,
    bound: Duration,
) -> Option<String> {
    #[cfg(test)]
    crate::lock(&HISTORY_READS).push(path.clone());
    let read = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        let text = std::fs::read_to_string(&path)?;
        let ours = |line: &&str| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v.get("run_id")?.as_str().map(str::to_string))
                .is_some_and(|id| runs.contains(&id))
        };
        let mut kept: Vec<&str> = text.lines().filter(ours).collect();
        let tail = kept.split_off(kept.len().saturating_sub(HANDOFF_LINES));
        Ok(tail.join("\n"))
    });
    match tokio::time::timeout(bound, read).await {
        Ok(Ok(Ok(text))) if !text.is_empty() => Some(text),
        _ => None,
    }
}

/// Fix round 1 (m5), tests only: the path of every history read, so a test can tell an
/// adopt read none.
#[cfg(test)]
pub(in crate::run::driver) static HISTORY_READS: std::sync::Mutex<Vec<PathBuf>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
#[path = "chain_goal_tests.rs"]
pub(in crate::run::driver) mod tests;
