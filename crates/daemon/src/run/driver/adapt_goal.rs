//! M8b.14: `anthrex run start --goal` (decision 22). Everything happens before any engine
//! event, with no lock held across an await: M8a's early refusals, the stored profile,
//! the triage call, [`triage::route`], and then M8a's whole start path
//! ([`RunService::build_plan`]) for the fast path's one-task plan or, from milestone 9
//! (decision 26, replacing M8b's refusal), for a planned run with no task yet, whose
//! orchestrator plans it. Triage takes no reader slot: no run exists yet.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use proto::run_wire::request;
use proto::{
    DeciderMode, DeliveryMode, DesignMode, HistoryLine, OrchestratorChoice, Plan, ProfileSpec,
    ProposalState, RepoProfile, RoleRoutingInput, RunPath, RunReply, TriageInfo,
};

use super::super::build::{Planned, Shape, TuneOnce};
use super::super::delivery::{DeliveryStart, Frozen};
use super::super::{RunService, unix_now};
use super::{Adaptation, read_evidence, unparseable};
use crate::decider::call::decide;
use crate::decider::fallback::{OFF_REASON, fallback_decision};
use crate::decider::{DeciderKind::Triage, DeciderRequest, Decision, TriageInput};
use crate::profile::service::Effective;
use crate::run::confine;
use crate::run::design::{GoalOrigin, mode_for};
use crate::run::engine::{EventKind, HISTORY_FILE};
use crate::run::git::{self, Git, os};
use crate::run::history_io::append_once;
use crate::run::model::Run;
use crate::run::orch::contract_design::start_message;
use crate::run::orch::roles;
use crate::run::plan::{PlanError, Preflight};
use crate::run::triage::{self, TriageRoute};
use crate::scout::report::ONBOARDING_ALIAS;

/// Why [`RunService::build_plan`] built no run: M8a's plan errors, which the fast path
/// turns into the planned path (decision 22 step 5), or any other refusal.
pub(in crate::run::driver) enum BuildError {
    Plan(Vec<PlanError>),
    Refused(String),
    /// The fast path's own barrier ([`fast_barrier`]): the planned path, with the reason.
    NotFast(String),
}

impl From<String> for BuildError {
    fn from(message: String) -> Self {
        BuildError::Refused(message)
    }
}

/// Review m1: on the fast path, [`triage::fast_refusal`] right after `build_run`, so a
/// hub or L task gets its own reason before the runtime checks' refusals.
pub(in crate::run::driver) fn fast_barrier(fast: bool, run: &Run) -> Result<(), BuildError> {
    match fast.then(|| triage::fast_refusal(run)).flatten() {
        Some(reason) => Err(BuildError::NotFast(reason)),
        None => Ok(()),
    }
}

impl BuildError {
    /// `run start --plan`'s text: every plan error, one per line.
    pub(in crate::run::driver) fn text(self) -> String {
        match self {
            BuildError::Plan(errors) => errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
            BuildError::Refused(message) | BuildError::NotFast(message) => message,
        }
    }
}

/// Milestone 9.10 decision 13: a continued goal (`--continue`) cannot wait in a queue,
/// since its chain's orchestrator is running; with no stored profile it is refused.
pub const CONTINUE_NEEDS_PROFILE: &str = "this repository has no stored profile any more; start a new goal (anthrex run start --goal) to set it up";

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|error| format!("a blocking step did not finish: {error}"))?
}

fn refused(message: String) -> RunReply {
    RunReply::refused(request::START_GOAL, message)
}

/// What a goal has passed before triage (decision 22's steps 1 and 2, then milestone
/// 9.2's delivery preflight): the preflight, the stored profile and its report, and the
/// frozen delivery. Milestone 9.3 decision 22: a continued goal passes the same, and
/// skips triage (`driver/chain_goal.rs`).
pub(in crate::run::driver) struct GoalReady {
    pub pre: Preflight,
    pub profile: RepoProfile,
    pub report: Option<String>,
    pub frozen: Frozen,
}

/// Why a goal is not ready to triage: a refusal in `run start --goal`'s words, or
/// (milestone 9.10 decision 12) no stored profile, which queues the goal.
pub(in crate::run::driver) enum GoalNotReady {
    Refused(String),
    NoProfile {
        // Boxed: `Preflight` is large, and clippy's result_large_err (1.99) refuses an
        // `Err` variant this big.
        pre: Box<Preflight>,
        proposal: Option<ProposalState>,
    },
}

impl From<String> for GoalNotReady {
    fn from(message: String) -> Self {
        GoalNotReady::Refused(message)
    }
}

impl RunService {
    /// Decision 22's steps 1 and 2, and the delivery's preflight (milestone 9.2
    /// decision 17), in that order, each refusal as `run start --goal` words it.
    pub(in crate::run::driver) async fn goal_ready(
        &self,
        goal: &str,
        dir: &Path,
        unconfined_checks: bool,
        delivery: Option<DeliveryMode>,
    ) -> Result<GoalReady, GoalNotReady> {
        // Review m2: a blank goal never spends a triage call.
        if let Some(refusal) = triage::blank_goal(goal) {
            return Err(refusal.into());
        }
        let Some(adaptation) = self.adaptation.get() else {
            return Err(GoalNotReady::Refused(
                "the profile service is not running".to_string(),
            ));
        };
        // 1. M8a's early refusals, exactly as `build_plan` applies them.
        let live = self.ctx.settings.current();
        let config = &live.orchestrator;
        let allowed = unconfined_checks || config.unconfined_checks;
        if let Some(refusal) =
            confine::start_refusal(config.worker_sandbox, confine::available(), allowed)
        {
            return Err(refusal.into());
        }
        let timeout = Duration::from_secs(config.git_timeout_secs);
        let (g, d) = (self.ctx.git.clone(), dir.to_path_buf());
        let pre = blocking(move || git::preflight(&g, &d, timeout)).await?;
        // 2. A stored profile, or (milestone 9.10 decision 12) none: the caller queues
        // the goal, or refuses a continued one (decision 13).
        let (profile, report) = match adaptation.profiles.effective(&pre.project).await {
            Effective::Stored { profile, meta, .. } => (profile, meta.report),
            Effective::Unparseable { path, error } => {
                return Err(unparseable(&path, &error).into());
            }
            Effective::Absent { proposal } => {
                return Err(GoalNotReady::NoProfile {
                    pre: Box::new(pre),
                    proposal,
                });
            }
        };
        // Milestone 9.2 decision 17: preflight before triage, so a refusal costs no
        // decider call; the mode is resolved here, once.
        let frozen = self
            .freeze_delivery(&pre, delivery, profile.delivery.as_ref())
            .await?;
        Ok(GoalReady {
            pre,
            profile,
            report,
            frozen,
        })
    }

    /// Decision 22, steps 1 to 6.
    pub(in crate::run::driver) async fn start_goal(
        &self,
        goal: String,
        dir: PathBuf,
        (trust_project, unconfined_checks): (bool, bool),
        yes: bool,
        (orchestrator, delivery, design): (
            Option<OrchestratorChoice>,
            Option<DeliveryMode>,
            Option<DesignMode>,
        ),
    ) -> RunReply {
        // M9.8.13 fix round 1: a bad model name (the CLI's flag or the TUI's goal form)
        // never reaches the orchestrator's `--model`.
        if let Some(problem) = orchestrator.as_ref().and_then(|c| c.model_problem().err()) {
            return refused(format!("orchestrator.model: {problem}"));
        }
        // Final review M3: nor a bad effort name reach its `--effort`.
        if let Some(problem) = orchestrator.as_ref().and_then(|c| c.effort_problem().err()) {
            return refused(format!("orchestrator.effort: {problem}"));
        }
        let flags = (trust_project, unconfined_checks);
        let (ready, stored) = match self
            .goal_ready(&goal, &dir, unconfined_checks, delivery)
            .await
        {
            Ok(ready) => (ready, String::new()),
            Err(GoalNotReady::Refused(message)) => return refused(message),
            // Milestone 9.10 decision 9: a `--yes` goal stores a ready proposal at once.
            Err(GoalNotReady::NoProfile {
                pre,
                proposal: Some(ProposalState::Ready),
            }) if yes => match self.use_ready_for(&pre, &goal, &dir, flags, delivery).await {
                Ok(pair) => pair,
                Err(message) => return refused(message),
            },
            // Decision 12: the goal waits for the profile.
            Err(GoalNotReady::NoProfile { pre, proposal }) => {
                let start = NoProfileStart {
                    pre: *pre,
                    proposal,
                    goal,
                    dir,
                    flags,
                    yes,
                    choices: (orchestrator, delivery, design),
                };
                return self.queue_goal_start(start).await;
            }
        };
        let choices = (orchestrator, design);
        let reply = self
            .start_ready(goal, dir, flags, yes, choices, ready)
            .await;
        stored_first(reply, &stored)
    }

    /// Decision 22, steps 3 to 6, for a goal that passed steps 1 and 2.
    async fn start_ready(
        &self,
        goal: String,
        dir: PathBuf,
        (trust_project, unconfined_checks): (bool, bool),
        yes: bool,
        (orchestrator, design): (Option<OrchestratorChoice>, Option<DesignMode>),
        ready: GoalReady,
    ) -> RunReply {
        let GoalReady {
            pre,
            profile,
            report,
            frozen,
        } = ready;
        let Some(adaptation) = self.adaptation.get() else {
            return refused("the profile service is not running".to_string());
        };
        let live = self.ctx.settings.current();
        let config = &live.orchestrator;
        let timeout = Duration::from_secs(config.git_timeout_secs);
        // 3. Triage.
        let decision = self
            .triage(
                adaptation,
                &goal,
                &profile,
                report.as_deref(),
                &pre,
                timeout,
            )
            .await;
        // 4. The route.
        let route = triage::route(&decision, config.fast_path);
        let info = triage::info(&decision, &route, unix_now());
        // Milestone 9.6 decision 3: the mode, decided once on triage's own route, before
        // any build (ruling T3-1: a fast goal whose build falls back to planned stays off;
        // `--design full` on a goal DF §1 puts off is refused here).
        let design = match mode_for(GoalOrigin::Goal(Some(&info)), design, &config.design) {
            Ok(mode) => Some(mode),
            Err(refusal) => return refused(refusal),
        };
        // 6. Milestone 9 decision 26: the planned and large paths build a planned run.
        let flags = (trust_project, unconfined_checks);
        let planned = |info: TriageInfo| Planned {
            triage: Some(info),
            usage: decision.usage,
            yes,
            choice: orchestrator.clone(),
            design,
        };
        let spec = profile.spec();
        // Milestone 9.5 decision 12 (ruling T9-5): one settings read and one tuning for
        // the fast build and its fallback.
        let once = TuneOnce::with_config(config.clone());
        let TriageRoute::Fast(task) = route else {
            let p = (planned(info), frozen, &once);
            return self
                .start_planned(&goal, &spec, dir.clone(), flags, p)
                .await;
        };
        // 5. The fast path: M8a's whole start path for a one-task plan.
        let plan = triage::fast_plan(&goal, *task, spec.clone());
        let all = (true, trust_project, unconfined_checks);
        let done = DeliveryStart::Done(frozen.clone());
        let built = match self
            .build_delivered(plan, dir.clone(), all, Shape::Fast, done, &once)
            .await
        {
            Ok(run) => Ok(run),
            Err(BuildError::Plan(errors)) => Err(errors),
            Err(BuildError::NotFast(reason)) => {
                let p = (planned(triage::not_fast(info, reason)), frozen, &once);
                return self.start_planned(&goal, &spec, dir, flags, p).await;
            }
            Err(BuildError::Refused(message)) => return refused(message),
        };
        let mut run = match triage::check_fast(built) {
            Ok(run) => run,
            Err(reason) => {
                let p = (planned(triage::not_fast(info, reason)), frozen, &once);
                return self.start_planned(&goal, &spec, dir, flags, p).await;
            }
        };
        triage::mark_fast(&mut run, info.clone(), decision.usage);
        let message = triage::started_message(&info, &run.id, &run.tasks[0]);
        let run_id = run.id.clone();
        match self
            .ask(|reply| EventKind::Start {
                reply,
                run: Box::new(run),
            })
            .await
        {
            Ok(_) => RunReply::Triaged {
                triage: info,
                run_id: Some(run_id),
                message,
                request_id: None,
            },
            Err(message) => refused(message),
        }
    }

    /// Decision 22 step 3: the triage call, or its fallback. With the deciders off
    /// nothing is read or spawned.
    async fn triage(
        &self,
        adaptation: &Adaptation,
        goal: &str,
        profile: &RepoProfile,
        report: Option<&str>,
        pre: &Preflight,
        timeout: Duration,
    ) -> Decision {
        let mut input = triage_input(goal, profile, &self.ctx.settings.current().orchestrator);
        if adaptation.deciders.mode == DeciderMode::Off {
            return fallback_decision(&DeciderRequest::Triage(input), OFF_REASON.into());
        }
        let repo_dir = crate::profile::repo_dir(&self.ctx.data_dir, &pre.project);
        let (g, root, id) = (
            self.ctx.git.clone(),
            pre.root.clone(),
            report.map(str::to_string),
        );
        let read = blocking(move || {
            let report = id.and_then(|id| {
                let refs = [ONBOARDING_ALIAS.to_string()];
                read_evidence(&refs, Path::new(""), &repo_dir, Some(&id))
                    .ok()
                    .and_then(|mut e| e.pop())
            });
            let listing = Git::new(&g, timeout).ok(&root, &[os("ls-files"), os("-z")])?;
            let thresholds = crate::run::tuning_io::thresholds(&repo_dir);
            Ok((report, triage::tracked_files(&listing), thresholds))
        })
        .await;
        match read {
            Ok((report, (files, total), thresholds)) => {
                input.thresholds = thresholds;
                if let Some(report) = report {
                    input.report_summary = Some(report.summary);
                    input.report_files = report.files;
                }
                input.files = files;
                input.files_total = total;
                // Milestone 9.5 rulings RL-2, I6: routed over what is installed now;
                // milestone 9.8: on the triage row, the repository's first.
                let (deciders, project) = (&adaptation.deciders, Some(pre.project.as_path()));
                let routed = crate::decider::call::routed(deciders, Triage, project).await;
                let decision = decide(&routed.ctx, &DeciderRequest::Triage(input)).await;
                self.record_triage(pre, (goal, profile), (&routed, &decision))
                    .await;
                decision
            }
            Err(error) => fallback_decision(
                &DeciderRequest::Triage(input),
                format!("the decider could not start: could not list tracked files: {error}"),
            ),
        }
    }
}

/// How long the goal's start waits for pre-run triage's history line (review M-6).
pub(in crate::run::driver) const TRIAGE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Pre-run triage records made by this daemon: each record id's `<n>`.
static TRIAGE_SEQ: AtomicU64 = AtomicU64::new(1);

impl RunService {
    /// Milestone 9 decision 43: the triage decider's record, with no run, appended to
    /// the repository's `history.jsonl` in anthrex's data directory once its answer or
    /// fallback is in, whatever becomes of the goal. Its id is
    /// `triage/<unix nanos>/<n>`, which no run's or task's record can take. Written on
    /// `spawn_blocking`, never under a lock; a failure is only logged.
    async fn record_triage(
        &self,
        pre: &Preflight,
        (goal, profile): (&str, &RepoProfile),
        (routed, decision): (&crate::decider::call::Routed, &Decision),
    ) {
        let n = TRIAGE_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let input = RoleRoutingInput {
            goal: Some(roles::capped_goal(goal)),
            languages: profile.languages.clone(),
            ..RoleRoutingInput::default()
        };
        let session = format!("{nanos}/{n}");
        let session = (session.as_str(), "triage");
        let at = (input, unix_now());
        let mut record = roles::decider_routed(None, session, &[], routed, at);
        // Ruling T10b-1: a triage has no run log yet; the daemon's log says it.
        if let Some(line) = routed.moved_line() {
            tracing::info!("{line}");
        }
        let (outcome, result) = roles::decider_outcome(decision);
        roles::finish(&mut record, outcome, result);
        let repo_dir = crate::profile::repo_dir(&self.ctx.data_dir, &pre.project);
        let path = repo_dir.join(HISTORY_FILE);
        let line = HistoryLine::RoleRoute(record);
        // Review M-6: a stalled file system never holds the goal's start.
        let write = blocking(move || append_once(&path, &line).map_err(|e| e.to_string()));
        match tokio::time::timeout(TRIAGE_WRITE_TIMEOUT, write).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, "the triage decider's history record was not written");
            }
            Err(_) => tracing::warn!(
                "the triage decider's history record was not written within {} s",
                TRIAGE_WRITE_TIMEOUT.as_secs()
            ),
        }
    }
}

/// The triage decider's input before any evidence is read: the goal, the profile
/// summary, and `[orchestrator] planner_task_cap`, the plan scale's upper bound (M9.3).
fn triage_input(
    goal: &str,
    profile: &RepoProfile,
    orchestrator: &config::Orchestrator,
) -> TriageInput {
    TriageInput {
        goal: triage::goal_input(goal),
        profile_summary: crate::profile::summary(profile),
        report_summary: None,
        report_files: Vec::new(),
        files: Vec::new(),
        files_total: 0,
        planner_task_cap: orchestrator.agent.planner_task_cap,
        // Milestone 9.5 decision 13: `triage` reads the repository's own.
        thresholds: Default::default(),
    }
}

impl RunService {
    /// Milestone 9 decision 26: a planned run for `goal`, built with no task through
    /// M8a's whole start path (every start check, decision 9's project-settings check
    /// covering the orchestrator's and its sub-planners' runtimes), then started: the
    /// engine makes its run branch and launches its orchestrator. `BuildContext.yes` is
    /// false; the run's `yes` applies when the orchestrator submits (decision 27).
    async fn start_planned(
        &self,
        goal: &str,
        profile: &ProfileSpec,
        dir: PathBuf,
        (trust_project, unconfined_checks): (bool, bool),
        (planned, frozen, once): (Planned, Frozen, &TuneOnce),
    ) -> RunReply {
        // D14: only a continued goal (`chain_goal.rs`) builds an untriaged planned run.
        let Some(info) = planned.triage.clone() else {
            return refused("a planned goal needs its triage".to_string());
        };
        let plan = Plan {
            goal: goal.to_string(),
            max_writers: None,
            max_readers: None,
            max_bounces: None,
            profile: profile.clone(),
            tasks: Vec::new(),
        };
        let shape = Shape::Planned(Box::new(planned));
        let flags = (false, trust_project, unconfined_checks);
        let done = DeliveryStart::Done(frozen);
        let run = match self
            .build_delivered(plan, dir, flags, shape, done, once)
            .await
        {
            Ok(run) => run,
            Err(error) => return refused(error.text()),
        };
        let (run_id, path) = (run.id.clone(), run.path.unwrap_or(RunPath::Plan));
        // Final fix wave FW-72: a design run's says its orchestrator may ask questions.
        let message = start_message(&info, &run_id, path, run.design_mode);
        match self
            .ask(|reply| EventKind::Start {
                reply,
                run: Box::new(run),
            })
            .await
        {
            Ok(_) => RunReply::Triaged {
                triage: info,
                run_id: Some(run_id),
                message,
                request_id: None,
            },
            Err(message) => refused(message),
        }
    }
}

#[path = "goal_queue.rs"]
mod queue;
use queue::{NoProfileStart, stored_first};

#[cfg(test)]
#[path = "adapt_goal_tests.rs"]
mod tests;
