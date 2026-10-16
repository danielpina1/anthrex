//! Milestone 9.16 decisions 8, 11–14, 18, 21 and 36: the brakes' gate. Pure: no file, no
//! clock, no lock; every time arrives as the engine event's `now`.
//!
//! [`Brakes`] is the daemon-wide state: the hold (decision 15), the brake's config and
//! its [`QuotaBook`], and the runtimes already announced as unmeasured (decision 8).
//! Each step computes one [`Gate`] from it (ruling C4: at the step's start, again after
//! the event's handler) and hands it to every scheduler pass, `orch::settle`'s included.
//! The scheduler's new-session entry points (decision 11) ask it per runtime; a refused
//! session waits where it is (a `Queued` task stays queued, a `Preparing` one keeps its
//! `ready_from`, a queued sub-planner, design agent or run scout stays queued). The
//! gate changes no run state: the hold pauses no run (decision 18).

use std::collections::{BTreeMap, BTreeSet};

use proto::brakes::{BrakesInfo, DispatchHold};
use proto::{Runtime, TaskState};

use super::quota_book::{BrakeConfig, QuotaBook, Trip, Verdict};
use super::requests::log;
use super::{Effect, ReplyId};
use crate::run::design::state::DesignAgentState;
use crate::run::model::Run;
use crate::run::orch::{PlannerPhase, RunScoutState};

/// Decision 19: the hold's refusal, and decision 21's attention line.
pub const HOLD_TEXT: &str = "dispatch is held daemon-wide · anthrex hold off";

/// Decision 8's run-log line (`QUOTA_BLIND_TEXT`), the first time per daemon start a
/// session of `runtime` starts while its quota is unmeasured or stale.
pub fn quota_blind_text(runtime: Runtime) -> String {
    format!("{runtime} quota is unmeasured; dispatch is not braked")
}

/// The brakes' daemon-wide state, in `EngineState.brakes`. In memory only: the driver
/// sends the hold (from `brakes.json`) and the config at start, and the book starts
/// unmeasured (decision 6).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Brakes {
    pub hold: Option<DispatchHold>,
    /// Off until the driver sends `[orchestrator.brakes]` (ruling C3).
    pub config: BrakeConfig,
    pub book: QuotaBook,
    /// The runtimes whose unmeasured quota was announced (decision 8): once each.
    pub announced: BTreeSet<Runtime>,
    /// [`info`] as the last step left it (decision 14): a step whose info differs
    /// republishes the snapshot, a release at `until` on a `Tick` included.
    pub shown: BrakesInfo,
}

/// Why the gate refuses a runtime (decision 36): the hold, or a quota trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Held,
    Quota(Trip),
}

/// One step's answer per runtime (see the module doc).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gate {
    held: bool,
    braked: BTreeMap<Runtime, Trip>,
    /// Unmeasured or stale, the brake on, and not yet announced.
    blind: BTreeSet<Runtime>,
    /// Announced during this step, for [`merge`].
    announced: BTreeSet<Runtime>,
}

impl Gate {
    /// Whether a fresh session of `runtime` may start: never under the hold
    /// (decision 43), never on a quota-braked runtime.
    pub fn allows(&self, runtime: Runtime) -> bool {
        self.refusal(runtime).is_none()
    }

    /// Whether `runtime` is unmeasured or stale and not yet announced (decision 8).
    pub fn blind(&self, runtime: Runtime) -> bool {
        self.blind.contains(&runtime)
    }

    /// Why `runtime` is refused (decision 36): `None` when allowed. The hold wins over
    /// a trip, so failover is never tried under it.
    pub fn refusal(&self, runtime: Runtime) -> Option<Refusal> {
        if self.held {
            return Some(Refusal::Held);
        }
        self.braked.get(&runtime).cloned().map(Refusal::Quota)
    }

    /// A fresh session of `runtime` starts in `run`: on a blind runtime the first one
    /// logs [`quota_blind_text`] to that run, once per daemon start (decision 8).
    pub fn started(&mut self, run: &mut Run, runtime: Runtime, now: u64) {
        if self.blind.remove(&runtime) {
            self.announced.insert(runtime);
            log(run, now, quota_blind_text(runtime));
        }
    }
}

/// The step's gate at `now`; drops the trips that expired (`QuotaBook::verdict`).
pub fn gate(brakes: &mut Brakes, now: u64) -> Gate {
    let mut gate = Gate {
        held: brakes.hold.is_some(),
        ..Gate::default()
    };
    for runtime in [Runtime::Claude, Runtime::Codex] {
        match brakes.book.verdict(runtime, now, &brakes.config) {
            Verdict::Braked(trip) => {
                gate.braked.insert(runtime, trip);
            }
            Verdict::Blind if !brakes.announced.contains(&runtime) => {
                gate.blind.insert(runtime);
            }
            _ => {}
        }
    }
    gate
}

/// Ruling C4: the gate's announcements this step join [`Brakes::announced`].
pub(super) fn merge(brakes: &mut Brakes, gate: &mut Gate) {
    brakes.announced.append(&mut gate.announced);
}

/// The snapshot's `brakes` (decision 13). `quota` is left empty while the brake is off,
/// so `BrakesInfo::is_empty()` holds for a daemon that brakes nothing.
pub fn info(brakes: &Brakes, now: u64) -> BrakesInfo {
    let on = brakes.config.quota_brake;
    BrakesInfo {
        hold: brakes.hold.clone(),
        quota_brake: on,
        quota: match on {
            true => brakes.book.info(now, &brakes.config),
            false => Vec::new(),
        },
    }
}

/// Decisions 13 and 21: a non-terminal run's brake lines, after its own attention
/// (ruling C22). [`HOLD_TEXT`] while the hold is on; one [`quota_held_text`] per braked
/// runtime that has something of this run waiting on it
/// (`common::brakes_text::quota_held_text`, ruling C13); then decision 42's failover
/// lines (`failover::attention`).
pub fn attention(brakes: &Brakes, run: &Run, now: u64) -> Vec<String> {
    if run.state.is_terminal() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    if brakes.hold.is_some() {
        lines.push(HOLD_TEXT.to_string());
    }
    if !brakes.config.quota_brake {
        return lines;
    }
    let mut braked = BTreeSet::new();
    for quota in brakes.book.info(now, &brakes.config) {
        let Some(trip) = quota.trip.as_ref() else {
            continue;
        };
        braked.insert(quota.runtime);
        if waits_on(run, quota.runtime) {
            lines.push(common::brakes_text::quota_held_text(quota.runtime, trip));
        }
    }
    // Decision 42: the failovers away from a runtime braked now.
    lines.extend(super::failover::attention(&braked, run));
    lines
}

/// Whether `run` has a session waiting to start on `runtime`: a `Queued` task, a
/// `Preparing` one ready to launch, or a queued sub-planner, design agent or run scout.
fn waits_on(run: &Run, runtime: Runtime) -> bool {
    let task = (0..run.tasks.len()).any(|i| {
        let t = &run.tasks[i];
        let waiting = t.state == TaskState::Queued
            || (t.state == TaskState::Preparing && t.ready_from.is_some());
        waiting && super::pair::dispatch_runtime(run, i) == runtime
    });
    let epic = (run.orch.epics.iter())
        .any(|e| e.phase == PlannerPhase::Queued && e.route.runtime == runtime);
    let scout = (run.orch.run_scouts.iter()).any(|s| s.state == RunScoutState::Queued)
        && crate::run::orch::launch::scout_route(run).runtime == runtime;
    let design = run.orch.design.as_ref().is_some_and(|d| {
        (d.brainstormers.iter().chain(d.reviewer.as_ref()))
            .any(|a| a.state == DesignAgentState::Queued && a.route.runtime == runtime)
    });
    task || epic || scout || design
}

/// `EventKind::Hold` (decision 17): sets the hold, idempotently (an `on` while on keeps
/// the first `since`), and answers with the message the request prints. Changes no
/// run (decision 18).
pub(super) fn hold(
    brakes: &mut Brakes,
    reply: ReplyId,
    hold: Option<DispatchHold>,
    fx: &mut Vec<Effect>,
) {
    let message = match (&brakes.hold, hold) {
        (Some(on), Some(_)) => format!(
            "dispatch was already held since {} UTC",
            common::text::hh_mm(on.since)
        ),
        (None, Some(new)) => {
            brakes.hold = Some(new);
            "dispatch held daemon-wide; running sessions continue · anthrex hold off releases it"
                .to_string()
        }
        (Some(_), None) => {
            brakes.hold = None;
            "dispatch released".to_string()
        }
        (None, None) => "dispatch is not held".to_string(),
    };
    fx.push(Effect::Reply {
        reply,
        result: Ok(message),
    });
}
