//! `ScoutService`'s table and its one pure step (split from `service.rs` to keep it
//! under 600 lines): what the service knows about each scout, and how one machine step
//! under the table's lock becomes deadlines, an outcome and effects to run after it.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use proto::{Route, ScoutInfo, ScoutReport};
use tokio::sync::oneshot;

use super::ScoutOutcome;
use crate::run::driver::unix_now;
use crate::scout::machine::{self, ScoutEffect, ScoutEvent, ScoutLimits, ScoutMachine};
use crate::scout::planner::PlannerTag;
use crate::scout::spec::ScoutSpec;

/// One scout the service knows.
pub(super) struct Scout {
    pub(super) spec: ScoutSpec,
    pub(super) route: Route,
    pub(super) window_id: Option<u32>,
    pub(super) machine: ScoutMachine,
    pub(super) outcome: Option<oneshot::Sender<ScoutOutcome>>,
    /// A report is being validated or written: a second one is refused meanwhile.
    pub(super) storing: bool,
    pub(super) report: Option<ScoutReport>,
    pub(super) report_bytes: Option<u32>,
    pub(super) ended_at: Option<u64>,
    /// Codex runs a process per turn: an exit after a turn ended in that process is not
    /// the session's end.
    pub(super) turn_ended_pids: HashSet<u32>,
    /// A kill the machine asked for before the window was bound (ruling M4): done at
    /// the bind.
    pub(super) kill_on_bind: bool,
    /// The start's own bind has happened: `create_headless` returned, so the process is
    /// installed and a kill reaches it (ruling R-T9-3).
    pub(super) installed: bool,
    pub(super) kill_at: Option<Instant>,
    pub(super) remove_at: Option<Instant>,
    /// A sub-planner's session (milestone 9 decision 31): its own limits, and no
    /// report; `run_scouts` leaves it out.
    pub(super) planner: Option<PlannerTag>,
}

#[derive(Default)]
pub(super) struct Table {
    pub(super) scouts: HashMap<String, Scout>,
    pub(super) by_window: HashMap<u32, String>,
    /// Headless windows that are not scouts, so their spec is looked up once.
    pub(super) foreign: HashSet<u32>,
}

/// What one machine step left to do outside the lock.
pub(super) struct Pending {
    pub(super) window: Option<u32>,
    pub(super) effects: Vec<ScoutEffect>,
    pub(super) outcome: Option<(oneshot::Sender<ScoutOutcome>, ScoutOutcome)>,
}

/// One machine step under the table lock: the new machine, the deadlines, the outcome,
/// and the effects left to execute.
pub(super) fn step_locked(scout: &mut Scout, event: ScoutEvent, limits: &ScoutLimits) -> Pending {
    let (next, effects) = machine::step(scout.machine.clone(), event, limits);
    scout.machine = next;
    // Ruling R-T9-3: until the start's own bind, a kill cannot reach the process (an
    // early bind from the session feed may come before it is installed), so it is owed.
    if !scout.installed && effects.contains(&ScoutEffect::Kill) {
        scout.kill_on_bind = true;
    }
    let mut outcome = None;
    let now = Instant::now();
    for effect in &effects {
        match effect {
            ScoutEffect::KillAfter(after) => scout.kill_at = Some(now + *after),
            ScoutEffect::RemoveAfter(after) => scout.remove_at = Some(now + *after),
            ScoutEffect::Finished(result) => {
                scout.ended_at = Some(unix_now());
                let ended = match (result, &scout.report) {
                    (Ok(()), Some(report)) => ScoutOutcome::Report(report.clone()),
                    (Err(reason), _) => ScoutOutcome::Failed {
                        reason: reason.clone(),
                    },
                    (Ok(()), None) if scout.planner.is_some() => ScoutOutcome::Accepted,
                    (Ok(()), None) => ScoutOutcome::Failed {
                        reason: "the scout reported nothing".into(),
                    },
                };
                outcome = scout.outcome.take().map(|sender| (sender, ended));
            }
            _ => {}
        }
    }
    Pending {
        window: scout.window_id,
        effects,
        outcome,
    }
}

pub(super) fn info(scout: &Scout) -> ScoutInfo {
    ScoutInfo {
        id: scout.spec.id.clone(),
        kind: scout.spec.kind,
        question: scout.spec.question.clone(),
        state: scout.machine.state,
        failure: scout.machine.failure.clone(),
        window_id: scout.window_id,
        route: scout.route.clone(),
        started_at: scout.machine.started_at,
        ended_at: scout.ended_at,
        tool_calls: scout.machine.tool_calls,
        report_bytes: scout.report_bytes,
        files: scout
            .report
            .as_ref()
            .map(|r| r.files.iter().map(|f| f.path.clone()).collect())
            .unwrap_or_default(),
        usage: scout.machine.usage,
    }
}
