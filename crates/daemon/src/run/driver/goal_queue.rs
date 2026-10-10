//! Milestone 9.10 decisions 6, 9 and 12: a goal with no stored profile waits in its
//! repository's queue while the set-up runs, and a drained goal starts through the run
//! service as any `StartGoal` does.
//!
//! I/O: the queue's file and the detection's start are the profile service's (on
//! `spawn_blocking`, behind its `writes` mutex); nothing here holds a lock across an
//! await, and a drained start runs with no lock of either service held.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::{
    DeliveryMode, DesignMode, OrchestratorChoice, ProposalOrigin, ProposalState, RunReply,
};

use super::{GoalNotReady, GoalReady, refused};
use crate::profile::queue::{self, QueuedGoal};
use crate::profile::service::Effective;
use crate::profile::service_queue::GoalStarter;
use crate::run::driver::{RunService, unix_now};
use crate::run::plan::Preflight;

/// A goal `goal_ready` found no stored profile for (decision 12), with its request.
pub(in crate::run::driver) struct NoProfileStart {
    pub pre: Preflight,
    pub proposal: Option<ProposalState>,
    pub goal: String,
    pub dir: PathBuf,
    /// `(trust_project, unconfined_checks)`.
    pub flags: (bool, bool),
    pub yes: bool,
    pub choices: (
        Option<OrchestratorChoice>,
        Option<DeliveryMode>,
        Option<DesignMode>,
    ),
}

/// The project's short name in messages: its directory's name.
fn name(project: &Path) -> String {
    project.file_name().map_or_else(
        || project.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Decision 12's `Queued` message (Interfaces, "Exact user-visible text").
fn queued_message(project: &Path, proposal: Option<&ProposalState>, yes: bool) -> String {
    let name = name(project);
    match (proposal, yes) {
        (Some(ProposalState::Ready), _) => format!(
            "queued: anthrex has a proposal for {name}; the goal starts once you review it (C-b P, or anthrex profile use)"
        ),
        (_, true) => format!(
            "queued: setting up anthrex for {name}; the goal starts as soon as every command checks out, or once you review it (anthrex profile status)"
        ),
        (_, false) => format!(
            "queued: setting up anthrex for {name}; the goal starts once you review how anthrex will work here (C-b P, or anthrex profile status)"
        ),
    }
}

/// Whether `proposal` is a set-up the goal can wait for.
fn in_progress(proposal: Option<&ProposalState>) -> bool {
    matches!(
        proposal,
        Some(
            ProposalState::Preparing
                | ProposalState::Scouting
                | ProposalState::Verifying
                | ProposalState::Ready
        )
    )
}

/// Milestone 9.10 decision 9: a goal that stored the proposal says so first.
pub(super) fn stored_first(reply: RunReply, stored: &str) -> RunReply {
    if stored.is_empty() {
        return reply;
    }
    match reply {
        RunReply::Triaged {
            triage,
            run_id,
            message,
            request_id,
        } => RunReply::Triaged {
            triage,
            run_id,
            message: format!("{stored}{message}"),
            request_id,
        },
        RunReply::Refused {
            request,
            message,
            request_id,
        } => RunReply::Refused {
            request,
            message: format!("{stored}{message}"),
            request_id,
        },
        other => other,
    }
}

impl RunService {
    /// Decision 12: with no proposal, or a failed one, the set-up starts as `profile
    /// detect` would, whatever `[orchestrator.onboarding] auto` says; a set-up that
    /// cannot start refuses the goal with its reason and queues nothing. Otherwise the
    /// goal waits for the proposal in progress or ready.
    pub(in crate::run::driver) async fn queue_goal_start(&self, start: NoProfileStart) -> RunReply {
        let Some(adaptation) = self.adaptation.get() else {
            return refused("the profile service is not running".to_string());
        };
        let profiles = &adaptation.profiles;
        let NoProfileStart {
            pre,
            mut proposal,
            goal,
            dir,
            flags: (trust_project, unconfined_checks),
            yes,
            choices: (orchestrator, delivery, design),
        } = start;
        if !in_progress(proposal.as_ref()) {
            let origin = ProposalOrigin::Goal;
            if let Err(refusal) = profiles
                .start_detection(&pre, origin, trust_project, unconfined_checks)
                .await
            {
                // Another goal's set-up may have started meanwhile: wait for that one.
                match profiles.effective(&pre.project).await {
                    Effective::Absent { proposal: now } if in_progress(now.as_ref()) => {
                        proposal = now;
                    }
                    _ => return refused(refusal),
                }
            }
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let queued = QueuedGoal {
            id: queue::next_id(nanos),
            project: pre.project.clone(),
            dir,
            goal,
            queued_at: unix_now(),
            yes,
            trust_project,
            unconfined_checks,
            orchestrator,
            delivery,
            design,
        };
        match profiles.queue_goal(queued).await {
            Ok(info) => RunReply::Queued {
                message: queued_message(&info.project, proposal.as_ref(), yes),
                goal_id: info.id,
                project: info.project,
                request_id: None,
            },
            Err(refusal) => refused(refusal),
        }
    }

    /// Decision 9: a `--yes` goal and a ready review proposal: the proposal is stored
    /// (`confirm` with no `shown`, which also drains the queue), then the goal passes
    /// `goal_ready` again. The second value is the reply's prefix.
    pub(in crate::run::driver) async fn use_ready_for(
        &self,
        pre: &Preflight,
        goal: &str,
        dir: &Path,
        flags: (bool, bool),
        delivery: Option<DeliveryMode>,
    ) -> Result<(GoalReady, String), String> {
        let Some(adaptation) = self.adaptation.get() else {
            return Err("the profile service is not running".to_string());
        };
        adaptation.profiles.use_ready(&pre.project).await?;
        let stored = format!(
            "stored the proposed profile for {} (--yes); ",
            name(&pre.project)
        );
        match self.goal_ready(goal, dir, flags, delivery).await {
            Ok(ready) => Ok((ready, stored)),
            Err(GoalNotReady::Refused(message)) => Err(format!("{stored}{message}")),
            Err(GoalNotReady::NoProfile { .. }) => Err(format!(
                "{stored}the stored profile is gone again; start the goal again"
            )),
        }
    }

    /// Decision 6: a drained goal started with its stored request; `Ok` is the run id,
    /// `Err` the refusal, which the drain records as a drop.
    pub(crate) async fn start_queued_goal(&self, goal: QueuedGoal) -> Result<String, String> {
        let flags = (goal.trust_project, goal.unconfined_checks);
        let choices = (goal.orchestrator, goal.delivery, goal.design);
        match self
            .start_goal(goal.goal, goal.dir, flags, goal.yes, choices)
            .await
        {
            RunReply::Triaged {
                run_id: Some(run_id),
                ..
            } => Ok(run_id),
            RunReply::Triaged { message, .. } | RunReply::Refused { message, .. } => Err(message),
            // The profile went away before the drain: the goal waits again, under a new
            // id, so it is not a drop.
            RunReply::Queued { goal_id, .. } => Ok(goal_id),
            other => Err(format!(
                "unexpected reply to a queued goal's start: {other:?}"
            )),
        }
    }

    /// Decision 6: the callback `profile::service::wire` installs, over a weak handle
    /// (the run service holds the profile service, as `set_live_runs`).
    pub(crate) fn queued_starter(self: &Arc<Self>) -> GoalStarter {
        let weak = Arc::downgrade(self);
        Arc::new(move |goal| {
            let weak = weak.clone();
            Box::pin(async move {
                match weak.upgrade() {
                    Some(runs) => runs.start_queued_goal(goal).await,
                    None => Err("the daemon is stopping".to_string()),
                }
            })
        })
    }
}

#[cfg(test)]
#[path = "goal_queue_tests.rs"]
mod tests;
