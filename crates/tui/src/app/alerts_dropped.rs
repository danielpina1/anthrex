//! Milestone 9.11 decision 22 (A7): a queued goal that could not start raises an alert,
//! with no wire change. The client remembers the queued goals of the last snapshot;
//! when one leaves it and no run with its goal and project appears in the same
//! snapshot, the client asks for the project's `profile status`. Every
//! `dropped_goals` entry there with the goal's text, recorded at or after the goal was
//! queued, raises one alert for the project (priority 3, the user's). Enter opens the
//! Profile screen and clears it; nothing else does. The alerts live in `App` for this
//! client session only. Mounted from `alerts.rs`. Pure: no I/O.

use super::{Alert, AlertKey, AlertWho, project_name};
use crate::app::profile_screen::ProfileAsk;
use crate::app::replies::PendingWhat;
use crate::app::{App, Effect};
use crate::safe_text::one_line;
use proto::{DroppedGoal, ProfileRequest, ProfileStatus, RunsSnapshot};
use std::path::{Path, PathBuf};

/// How much of a goal the alert's text quotes.
const GOAL_CHARS: usize = 60;

/// A goal that left the queue with no run for it, waiting for its project's status.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Left {
    project: PathBuf,
    goal: String,
    queued_at: u64,
}

/// The client's dropped-goal state: the goals waiting for a status, and each project's
/// raised drops in the order they were raised.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DroppedGoals {
    left: Vec<Left>,
    raised: Vec<(PathBuf, Vec<DroppedGoal>)>,
}

/// `goal`, one line, cut to [`GOAL_CHARS`] with an ellipsis when longer.
fn quoted(goal: &str) -> String {
    let line = one_line(goal);
    let mut out: String = line.chars().take(GOAL_CHARS).collect();
    if line.chars().nth(GOAL_CHARS).is_some() {
        out.push('…');
    }
    out
}

/// `could not start "<goal>": <reason>`, one line.
fn drop_text(drop: &DroppedGoal) -> String {
    format!(
        "could not start \"{}\": {}",
        quoted(&drop.goal),
        one_line(&drop.reason)
    )
}

/// One alert per project with raised drops, its text the latest drop's and its detail
/// every drop's, oldest first.
pub(super) fn dropped_alerts(app: &App, out: &mut Vec<Alert>) {
    for (project, drops) in &app.dropped_goals.raised {
        let Some(latest) = drops.last() else {
            continue;
        };
        out.push(Alert {
            priority: 3,
            key: AlertKey::Dropped(project.clone()),
            who: AlertWho::Project(one_line(&project_name(project))),
            task: None,
            text: drop_text(latest),
            detail: drops.iter().map(drop_text).collect::<Vec<_>>().join("\n"),
            age: Some(app.run_age(latest.at)),
            you: false,
        });
    }
}

impl App {
    /// Before `next` replaces the snapshot: every queued goal it no longer lists, with
    /// no run of the same goal and project in it, waits for its project's status, which
    /// is asked once per project (unless one is already out).
    pub(in crate::app) fn watch_queued_goals(&mut self, next: &RunsSnapshot) -> Vec<Effect> {
        let left: Vec<Left> = (self.runs.queued_goals.iter())
            .filter(|goal| !next.queued_goals.iter().any(|g| g.id == goal.id))
            .filter(|goal| {
                !(next.runs.iter()).any(|run| run.goal == goal.goal && run.project == goal.project)
            })
            .map(|goal| Left {
                project: goal.project.clone(),
                goal: goal.goal.clone(),
                queued_at: goal.queued_at,
            })
            .collect();
        let mut projects: Vec<PathBuf> = Vec::new();
        for goal in left {
            if !projects.contains(&goal.project) {
                projects.push(goal.project.clone());
            }
            self.dropped_goals.left.push(goal);
        }
        if !self.connected() {
            return vec![];
        }
        let mut effects = Vec::new();
        for dir in projects {
            let what = PendingWhat::Profile {
                dir: dir.clone(),
                ask: ProfileAsk::Status,
            };
            if self.replies.waits_for(&what) {
                continue;
            }
            let request = ProfileRequest::Status { dir: dir.clone() };
            effects.push(self.profile_send(dir, ProfileAsk::Status, request));
        }
        effects
    }

    /// A `profile status` reply for `dir` (any request's): each goal that left `dir`'s
    /// queue takes the drops of its text recorded at or after it was queued, and is
    /// then no longer waited for.
    pub(in crate::app) fn note_profile_status(&mut self, dir: &Path, status: &ProfileStatus) {
        let mut found: Vec<DroppedGoal> = Vec::new();
        self.dropped_goals.left.retain(|left| {
            if left.project != dir {
                return true;
            }
            let drops = (status.dropped_goals.iter())
                .filter(|drop| drop.goal == left.goal && drop.at >= left.queued_at);
            for drop in drops {
                if !found.contains(drop) {
                    found.push(drop.clone());
                }
            }
            false
        });
        if found.is_empty() {
            return;
        }
        let raised = &mut self.dropped_goals.raised;
        let at = match raised.iter().position(|(project, _)| project == dir) {
            Some(at) => at,
            None => {
                raised.push((dir.to_path_buf(), Vec::new()));
                raised.len() - 1
            }
        };
        let drops = &mut raised[at].1;
        for drop in found {
            if !drops.contains(&drop) {
                drops.push(drop);
            }
        }
    }

    /// Decision 22's Enter: the Profile screen on `project`, and the alert is cleared.
    pub(in crate::app) fn enter_dropped(&mut self, project: &Path) -> Vec<Effect> {
        self.dropped_goals.raised.retain(|(p, _)| p != project);
        self.open_profile_on(project.to_path_buf())
    }
}
