//! M8b.14: `anthrex run start --goal` (decision 22). Everything happens before any engine
//! event, with no lock held across an await: M8a's early refusals, the stored profile,
//! the triage call, [`triage::route`], and then either M8a's whole start path for the
//! fast path's one-task plan ([`RunService::build_plan`]) or a refusal that creates
//! nothing. Triage takes no reader slot: no run exists yet.

use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::run_wire::request;
use proto::{DeciderMode, ProposalOrigin, ProposalState, RepoProfile, RunReply};

use super::super::{RunService, unix_now};
use super::{Adaptation, read_evidence, unparseable};
use crate::decider::call::decide;
use crate::decider::fallback::{OFF_REASON, fallback_decision};
use crate::decider::{DeciderRequest, Decision, TriageInput};
use crate::profile::service::{Effective, state_label};
use crate::run::confine;
use crate::run::engine::EventKind;
use crate::run::git::{self, Git, os};
use crate::run::model::Run;
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

/// Decision 22 step 2's first refusal (and the one that starts detection).
pub const DETECTION_STARTED: &str = "this repository has no stored profile; detection has started (anthrex profile status), then confirm it with anthrex profile confirm and start the goal again";
/// … with `onboarding.auto` off.
pub const DETECT_FIRST: &str = "this repository has no stored profile; run anthrex profile detect, then anthrex profile confirm";
/// … with a proposal ready.
pub const PROPOSAL_READY: &str = "this repository has no stored profile; a proposal is ready: anthrex profile show --proposed, then anthrex profile confirm";

/// … while a proposal is in progress.
fn detection_running(state: &ProposalState) -> String {
    format!(
        "this repository has no stored profile; detection is {} (anthrex profile status)",
        state_label(state)
    )
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|error| format!("a blocking step did not finish: {error}"))?
}

fn refused(message: String) -> RunReply {
    RunReply::Refused {
        request: request::START_GOAL.to_string(),
        message,
    }
}

impl RunService {
    /// Decision 22, steps 1 to 6.
    pub(in crate::run::driver) async fn start_goal(
        &self,
        goal: String,
        dir: PathBuf,
        trust_project: bool,
        unconfined_checks: bool,
    ) -> RunReply {
        // Review m2: a blank goal never spends a triage call.
        if let Some(refusal) = triage::blank_goal(&goal) {
            return refused(refusal);
        }
        let Some(adaptation) = self.adaptation.get() else {
            return refused("the profile service is not running".to_string());
        };
        // 1. M8a's early refusals, exactly as `build_plan` applies them.
        let config = &self.ctx.orchestrator;
        let allowed = unconfined_checks || config.unconfined_checks;
        if let Some(refusal) =
            confine::start_refusal(config.worker_sandbox, confine::available(), allowed)
        {
            return refused(refusal);
        }
        let timeout = Duration::from_secs(config.git_timeout_secs);
        let (g, d) = (self.ctx.git.clone(), dir.clone());
        let pre = match blocking(move || git::preflight(&g, &d, timeout)).await {
            Ok(pre) => pre,
            Err(message) => return refused(message),
        };
        // 2. A stored profile, or the reason there is none.
        let (profile, report) = match adaptation.profiles.effective(&pre.project).await {
            Effective::Stored { profile, meta, .. } => (profile, meta.report),
            Effective::Unparseable { path, error } => return refused(unparseable(&path, &error)),
            Effective::Absent { proposal } => {
                let flags = (trust_project, unconfined_checks);
                return refused(no_profile(adaptation, &pre, proposal, flags).await);
            }
        };
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
        let TriageRoute::Fast(task) = route else {
            // 6. Nothing is created.
            return planned(info);
        };
        // 5. The fast path: M8a's whole start path for a one-task plan.
        let plan = triage::fast_plan(&goal, *task, profile.spec());
        let built = match self
            .build_plan(plan, dir, true, trust_project, unconfined_checks, true)
            .await
        {
            Ok(run) => Ok(run),
            Err(BuildError::Plan(errors)) => Err(errors),
            Err(BuildError::NotFast(reason)) => return planned(triage::not_fast(info, reason)),
            Err(BuildError::Refused(message)) => return refused(message),
        };
        let mut run = match triage::check_fast(built) {
            Ok(run) => run,
            Err(reason) => return planned(triage::not_fast(info, reason)),
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
        let mut input = TriageInput {
            goal: triage::goal_input(goal),
            profile_summary: crate::profile::summary(profile),
            report_summary: None,
            report_files: Vec::new(),
            files: Vec::new(),
            files_total: 0,
        };
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
            Ok((report, triage::tracked_files(&listing)))
        })
        .await;
        match read {
            Ok((report, (files, total))) => {
                if let Some(report) = report {
                    input.report_summary = Some(report.summary);
                    input.report_files = report.files;
                }
                input.files = files;
                input.files_total = total;
                decide(&adaptation.deciders, &DeciderRequest::Triage(input)).await
            }
            Err(error) => fallback_decision(
                &DeciderRequest::Triage(input),
                format!("the decider could not start: could not list tracked files: {error}"),
            ),
        }
    }
}

/// Decision 22 step 6: the planned or large path, refused until milestone 9.
fn planned(info: proto::TriageInfo) -> RunReply {
    let message = triage::refused_message(&info);
    RunReply::Triaged {
        triage: info,
        run_id: None,
        message,
    }
}

/// Decision 22 step 2's refusal for a repository with no stored profile. With
/// `onboarding.auto` on and nothing pending, detection starts first (with the goal's
/// own `--trust-project` and `--unconfined-checks`); its refusals are stored as the
/// proposal's `Failed` reason.
async fn no_profile(
    adaptation: &Adaptation,
    pre: &Preflight,
    proposal: Option<ProposalState>,
    (trust_project, unconfined_checks): (bool, bool),
) -> String {
    match proposal {
        Some(ProposalState::Ready) => PROPOSAL_READY.to_string(),
        Some(
            state @ (ProposalState::Preparing | ProposalState::Scouting | ProposalState::Verifying),
        ) => detection_running(&state),
        None | Some(ProposalState::Failed { .. }) => {
            if !adaptation.profiles.onboarding_auto() {
                return DETECT_FIRST.to_string();
            }
            adaptation
                .profiles
                .detect_or_record(pre, ProposalOrigin::Goal, trust_project, unconfined_checks)
                .await;
            DETECTION_STARTED.to_string()
        }
    }
}

#[cfg(test)]
#[path = "adapt_goal_tests.rs"]
mod tests;
