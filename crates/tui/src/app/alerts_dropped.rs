//! Milestone 9.11 decision 22 (A7): a queued goal that could not start raises an alert,
//! with no wire change. The client remembers the queued goals of the last snapshot;
//! when one leaves it and no run with its goal and project appears in the same
//! snapshot, the client waits for it and asks for the project's `profile status`. Every
//! `dropped_goals` entry there with the goal's text, recorded at or after the goal was
//! queued, raises one alert for the project (priority 3, the user's). Enter opens the
//! Profile screen and clears it; nothing else does. The alerts live in `App` for this
//! client session only. Mounted from `alerts.rs`. Pure: no I/O.
//!
//! Fix round 1 (review I1, the controller's ruling): the daemon's drain hides a
//! project's goals from the snapshot before their starts return
//! (`profile/service_queue.rs`, `claim_drain`), so a goal leaves the snapshot before its
//! drop is recorded. A status without the drop therefore does not end the wait. The
//! wait ends when a run for the goal appears, when a status reply holds its drop, or,
//! silently, at [`GOAL_START_BUDGET`] per goal up to and including it in its project's
//! queue (`profile status` still lists any drop). While goals wait, every later
//! snapshot makes their directory's status due again; it is asked with at most one
//! request in flight per directory and at least [`ASK_GAP`] between asks.

use super::{Alert, AlertKey, AlertWho, project_name};
use crate::app::profile_screen::ProfileAsk;
use crate::app::replies::PendingWhat;
use crate::app::{App, Effect};
use crate::safe_text::one_line;
use proto::{DroppedGoal, ProfileRequest, ProfileStatus, RunsSnapshot};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How much of a goal the alert's text quotes.
const GOAL_CHARS: usize = 60;

/// The least time between two status asks for one directory.
pub(crate) const ASK_GAP: Duration = Duration::from_secs(1);

/// The CLI's `RUN_REQUEST_TIMEOUT` (`cli/src/run_cmd.rs`), which the client cannot
/// import: the CLI depends on this crate, not the other way round.
const RUN_REQUEST_SECS: u64 = 180;

/// One queued goal's start, at its worst: the CLI's `GOAL_REQUEST_TIMEOUT`
/// (`cli/src/run_cmd/adapt.rs`, 1220 s; `docs/timing-budgets.md`, stage 11d's row),
/// rebuilt from the same terms: `RUN_START_TIMEOUT` (`RUN_REQUEST_TIMEOUT` +
/// `PREFLIGHT_BOUND` + 30 s + `TUNING_START_BOUND`), then the triage's 600 + 30 s and
/// `INSTALLED_PROBE_TIMEOUT`. A drained goal is started by the same `StartGoal` path.
pub(crate) const GOAL_START_BUDGET: Duration =
    Duration::from_secs(RUN_REQUEST_SECS + 30 + 600 + 30)
        .saturating_add(daemon::host::PREFLIGHT_BOUND)
        .saturating_add(daemon::run::driver::tuning::TUNING_START_BOUND)
        .saturating_add(daemon::run::driver::INSTALLED_PROBE_TIMEOUT);

/// A goal that left the queue with no run for it, waiting for its project's status.
/// `give_up_at` is in the daemon's unix seconds (the snapshots' `now`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Left {
    id: String,
    project: PathBuf,
    goal: String,
    queued_at: u64,
    give_up_at: u64,
}

/// The client's dropped-goal state: the goals waiting for a status, the directories
/// whose status is due and when each was last asked, and each project's raised drops
/// in the order they were raised.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DroppedGoals {
    left: Vec<Left>,
    due: Vec<PathBuf>,
    asked: Vec<(PathBuf, Instant)>,
    raised: Vec<(PathBuf, Vec<DroppedGoal>)>,
}

/// Whether `snapshot` holds a run of `goal` in `project`.
fn has_run(snapshot: &RunsSnapshot, project: &Path, goal: &str) -> bool {
    (snapshot.runs.iter()).any(|run| run.goal == goal && run.project == project)
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
    /// Before `next` replaces the snapshot (`now` the client's clock): every queued goal
    /// it no longer lists, with no run of the same goal and project in it, is waited
    /// for; a wait ends when its goal is queued again, its run appears, or its bound
    /// passes. Every directory still waited for is then due, and asked as the gap and
    /// the one-in-flight rule allow.
    pub(in crate::app) fn watch_queued_goals(
        &mut self,
        next: &RunsSnapshot,
        now: Instant,
    ) -> Vec<Effect> {
        let old = &self.runs.queued_goals;
        let budget = GOAL_START_BUDGET.as_secs();
        let departed: Vec<Left> = (old.iter())
            .filter(|goal| !next.queued_goals.iter().any(|g| g.id == goal.id))
            .filter(|goal| !has_run(next, &goal.project, &goal.goal))
            .map(|goal| {
                // The drain starts a project's goals in queue order, one at a time.
                let ahead = (old.iter())
                    .take_while(|g| g.id != goal.id)
                    .filter(|g| g.project == goal.project)
                    .count() as u64;
                Left {
                    id: goal.id.clone(),
                    project: goal.project.clone(),
                    goal: goal.goal.clone(),
                    queued_at: goal.queued_at,
                    give_up_at: next.now.saturating_add((ahead + 1).saturating_mul(budget)),
                }
            })
            .collect();
        let state = &mut self.dropped_goals;
        state.left.retain(|left| {
            !next.queued_goals.iter().any(|g| g.id == left.id)
                && !has_run(next, &left.project, &left.goal)
                && next.now < left.give_up_at
        });
        state.left.extend(departed);
        for left in &state.left {
            if !state.due.contains(&left.project) {
                state.due.push(left.project.clone());
            }
        }
        self.ask_dropped_due(now)
    }

    /// The due status asks at `now`: one per directory still waited for, none while one
    /// is in flight for it, none within [`ASK_GAP`] of its last; the rest stay due.
    pub(in crate::app) fn ask_dropped_due(&mut self, now: Instant) -> Vec<Effect> {
        let state = &mut self.dropped_goals;
        let waits = |dir: &Path, left: &[Left]| left.iter().any(|l| l.project == dir);
        state.due.retain(|dir| waits(dir, &state.left));
        state.asked.retain(|(dir, _)| waits(dir, &state.left));
        if state.due.is_empty() || !self.connected() {
            return vec![];
        }
        let mut effects = Vec::new();
        for dir in self.dropped_goals.due.clone() {
            let what = PendingWhat::Profile {
                dir: dir.clone(),
                ask: ProfileAsk::Status,
            };
            let recent = (self.dropped_goals.asked.iter())
                .any(|(d, at)| *d == dir && now.saturating_duration_since(*at) < ASK_GAP);
            if recent || self.replies.waits_for(&what) {
                continue;
            }
            let request = ProfileRequest::Status { dir: dir.clone() };
            effects.push(self.profile_send(dir.clone(), ProfileAsk::Status, request));
            let state = &mut self.dropped_goals;
            state.due.retain(|d| *d != dir);
            state.asked.retain(|(d, _)| *d != dir);
            state.asked.push((dir, now));
        }
        effects
    }

    /// A `profile status` reply for `dir` (any request's, a late one included): each
    /// goal waiting on `dir` takes the drops of its text recorded at or after it was
    /// queued, and is then no longer waited for. A goal with no such drop keeps
    /// waiting (fix round 1: the drain may not have recorded it yet).
    pub(in crate::app) fn note_profile_status(&mut self, dir: &Path, status: &ProfileStatus) {
        let mut found: Vec<DroppedGoal> = Vec::new();
        self.dropped_goals.left.retain(|left| {
            if left.project != dir {
                return true;
            }
            let drops: Vec<&DroppedGoal> = (status.dropped_goals.iter())
                .filter(|drop| drop.goal == left.goal && drop.at >= left.queued_at)
                .collect();
            for drop in &drops {
                if !found.contains(drop) {
                    found.push((*drop).clone());
                }
            }
            drops.is_empty()
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
