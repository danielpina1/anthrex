//! Milestone 9.10 decisions 33 and 34: the project alerts. A ready review proposal is
//! one review alert, listing the goals that wait on it; a repository whose goals wait
//! for its profile and that has no ready review proposal is one set-up alert, with the
//! set-up's progress or its failure (whose Enter retries the detection). Both are the
//! user's (`you: false`), pushed straight into the list: no orchestrator can take
//! them. Mounted from `alerts.rs`. Pure: no I/O.

use super::{Alert, AlertKey, AlertWho, project_name};
use crate::app::profile_screen::ProfileAsk;
use crate::app::{App, Effect, ToastLevel};
use crate::profile_words::setup_text;
use crate::safe_text::one_line;
use proto::{ProfileRequest, QueuedGoalInfo, SetupState};
use std::path::Path;

/// Decision 33's text and the first line of its detail.
const REVIEW_TEXT: &str = "review how anthrex will work here";
const REVIEW_DETAIL: &str = "anthrex learned how to work in this repo; ⏎ opens the review";

/// The goals waiting for `project`'s profile, in queue order.
fn waiting<'a>(app: &'a App, project: &'a Path) -> impl Iterator<Item = &'a QueuedGoalInfo> {
    app.runs
        .queued_goals
        .iter()
        .filter(move |goal| goal.project == project)
}

/// Where `project`'s set-up stands, read off its first queued goal; `None` with none.
pub(crate) fn setup_state<'a>(app: &'a App, project: &'a Path) -> Option<&'a SetupState> {
    waiting(app, project).next().map(|goal| &goal.setup)
}

fn project_who(project: &Path) -> AlertWho {
    AlertWho::Project(one_line(&project_name(project)))
}

/// Decisions 33 and 34: the review alerts (priority 4), then one set-up alert per
/// repository with queued goals and no ready review proposal (priority 3 when it
/// failed, else 5), in the order the snapshot lists them.
pub(super) fn profile_alerts(app: &App, out: &mut Vec<Alert>) {
    for proposal in &app.runs.proposals {
        let mut detail = vec![REVIEW_DETAIL.to_owned()];
        detail.extend(
            waiting(app, &proposal.project)
                .map(|goal| format!("waiting: {} (profile needs review)", one_line(&goal.goal))),
        );
        out.push(Alert {
            priority: 4,
            key: AlertKey::Proposal(proposal.project.clone()),
            who: project_who(&proposal.project),
            task: None,
            text: REVIEW_TEXT.to_owned(),
            detail: detail.join("\n"),
            age: Some(app.run_age(proposal.updated_at)),
            you: false,
        });
    }
    let mut projects: Vec<&Path> = Vec::new();
    for goal in &app.runs.queued_goals {
        let project = goal.project.as_path();
        let reviewed = (app.runs.proposals.iter()).any(|p| p.project == project);
        if !reviewed && !projects.contains(&project) {
            projects.push(project);
        }
    }
    for project in projects {
        let goals: Vec<&QueuedGoalInfo> = waiting(app, project).collect();
        let Some(first) = goals.first() else {
            continue;
        };
        let priority = match first.setup {
            SetupState::Failed { .. } => 3,
            _ => 5,
        };
        let detail: Vec<String> = (goals.iter())
            .map(|goal| format!("waiting: {}", one_line(&goal.goal)))
            .collect();
        out.push(Alert {
            priority,
            key: AlertKey::Setup(project.to_path_buf()),
            who: project_who(project),
            task: None,
            text: one_line(&setup_text(&first.setup)),
            detail: detail.join("\n"),
            age: Some(app.run_age(first.queued_at)),
            you: false,
        });
    }
}

impl App {
    /// Decision 7's **Retry** on a failed set-up alert: `Detect` with the first queued
    /// goal's flags, tagged; its outcome is toasted (or the Profile screen, open on
    /// the project, takes it). The alert stays until the next snapshot.
    pub(in crate::app) fn retry_setup(&mut self, project: &Path) -> Vec<Effect> {
        let Some(first) = waiting(self, project).next() else {
            return vec![];
        };
        let request = ProfileRequest::Detect {
            dir: project.to_path_buf(),
            trust_project: first.trust_project,
            unconfined_checks: first.unconfined_checks,
        };
        if !self.connected() {
            self.toast_at(ToastLevel::Warn, "not connected");
            return vec![];
        }
        vec![self.profile_send(project.to_path_buf(), ProfileAsk::Detect, request)]
    }
}
