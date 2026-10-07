//! Milestone 9.5's daemon model (review ruling I3): a racing task's lanes (decisions
//! 19–20), a paired task's test writer (decision 25) and a run's per-runtime writer
//! caps (decision 16). Added once, here, for every reducer task that uses them; nothing
//! reads them yet. Pure, like `model.rs`, which re-exports them.
//!
//! The fields they hang from (`Task.{race, pair, race_wait_since}`, `Run.concurrency`
//! and the `lane` of `AgentRound`, `CheckRecord`, `ProofRecord`, `ReviewRecord` and
//! `PendingOp`) are `#[serde(default)]` and skipped while unset, so a run with no race,
//! pair or cap writes the same `run.json` as milestone 9.3.

use proto::{
    ClassRoute, Effort, GateCounts, LaneState, ModelEntry, PairPhase, RaceLane, Route,
    RoutingCandidate, Runtime, Spend, Strength,
};
use serde::{Deserialize, Serialize};

use super::DoneClaim;

/// One runtime's writer cap inside a run (decision 16), keyed in `Run.concurrency` by
/// the runtime label `Run.rate_limits` uses. `cap` starts at `max_writers`; a rate
/// limit halves it, a quiet `recover_after_secs` raises it one step. No `Default`: an
/// entry is made by [`RuntimeConcurrency::new`] with the run's `max_writers`, never at
/// cap 0 (task M9.5.3b review m2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeConcurrency {
    pub cap: u8,
    pub last_rate_limit_at: Option<u64>,
    pub last_change_at: Option<u64>,
    pub halvings: u32,
    pub recoveries: u32,
}

impl RuntimeConcurrency {
    /// A runtime first seen in a run: at `max_writers`, never changed.
    pub fn new(max_writers: u8) -> Self {
        RuntimeConcurrency {
            cap: max_writers,
            last_rate_limit_at: None,
            last_change_at: None,
            halvings: 0,
            recoveries: 0,
        }
    }
}

/// One lane of a racing task (decision 19): its own standalone checkout `<task>.<lane>`,
/// its own racer session and its own gate counters (decision 20).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lane {
    pub lane: RaceLane,
    pub route: Route,
    pub review_route: Option<Route>,
    /// The lane's checkout name, `<task>.<lane>`.
    pub checkout: String,
    pub state: LaneState,
    /// The lane's racer session number (the task's session numbering).
    pub session: u32,
    pub start_commit: Option<String>,
    pub head: Option<String>,
    pub done: Option<DoneClaim>,
    pub failures: u8,
    pub bounces: GateCounts,
    pub stalls: u8,
    pub budget_exceeded: u8,
    pub spent: Spend,
    /// Why the lane left the race (`Out`).
    pub reason: Option<String>,
    pub salvage_ref: Option<String>,
    /// The stale lock files removed from the lane's git directory before salvage.
    pub cleared_locks: Vec<String>,
    /// When the lane's racer was sent `KillWindow` (decision 22's wait).
    pub kill_sent_at: Option<u64>,
    /// The lane's checkout was removed.
    pub removed: bool,
    /// The lane's checkout was kept: its racer did not exit in time.
    pub kept: bool,
    /// The rest of the lane's gate state (task M9.5.17a).
    #[serde(default, skip_serializing_if = "LaneGates::is_idle")]
    pub gates: LaneGates,
}

/// Milestone 9.5 decision 20: the task fields a lane keeps for itself besides those of
/// [`Lane`], each with the meaning of the `Task` field of the same name. While the
/// reducer handles one lane (`engine/race_view.rs`), these and the lane's own
/// `start_commit`, `head`, `done`, counters, `spent`, `route` and `review_route` are
/// the task's. `block` is why the lane went out, with `rung` the rung that took it out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaneGates {
    pub rung: u8,
    pub gate_op: Option<super::OpId>,
    pub claim: Option<super::PendingClaim>,
    pub review_misses: u8,
    pub pending_failure: Option<super::PendingFailure>,
    pub signals: Vec<crate::run::tiers::Signal>,
    pub signals_more: u32,
    pub signal_refusals: u8,
    pub fresh_session: Option<super::FreshSession>,
    pub ready_from: Option<String>,
    pub worktree_live: bool,
    pub clock: crate::run::engine::TaskClock,
    pub block: Option<proto::BlockInfo>,
    /// Ruling RR-4 (task M9.5.17b): the lane's own refresh of a racing task, the
    /// task's `orch.refresh` in the lane's view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<crate::run::orch::RefreshState>,
    /// The final fix wave's m2: the lane resolves a conflict its refresh handed back,
    /// the task's `resolving` in the lane's view.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resolving: bool,
    /// The final fix wave's m8: the lane went out on a `task_blocked` with no kind,
    /// which a race does not classify; an adoption classifies it after the crown.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unclassified: bool,
}

impl LaneGates {
    pub fn is_idle(&self) -> bool {
        *self == LaneGates::default()
    }
}

/// Decision 18 (ruling T17a-1): what dispatch decided for a racing task, latched the
/// first time it decided: it races, or it runs one worker for `reason` (its note).
/// Cleared only when the task goes back to be dispatched again (a reset or retry).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RaceDecision {
    Race,
    Single { reason: String },
}

/// A racing task's two lanes and, once decided, the lane that became the task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Race {
    pub lanes: Vec<Lane>,
    pub winner: Option<RaceLane>,
    /// The winner was adopted (the other lane was out), not crowned by passing.
    pub adopted: bool,
    pub started_at: u64,
    /// The winner's `CrownRacer` came back `Crowned` (task M9.5.17a): the task is the
    /// winning lane's from then on. Until then the winner is shown its own view, where
    /// it can do nothing, so no import runs between `Won` and the crown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub crowned: bool,
    /// Task M9.5.17b's addendum (b): the race is over without a winner (both lanes
    /// out) and `run retry` dispatched the task again: it runs single and counts as an
    /// ordinary task. Its lanes stay, for their salvage and the run's clean-up.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ended: bool,
}

/// A paired task's test writer and its red commit (decisions 25–26).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    pub phase: PairPhase,
    pub writer_route: Route,
    pub test: Option<String>,
    pub red: Option<String>,
    /// `Some(true)` once the red-only proof saw the test fail at `red`.
    pub red_checked: Option<bool>,
    /// The test writer's gate failures, moved here when the implementer starts.
    pub writer_failures: u8,
    pub writer_sessions: u32,
    /// Ruling T16-2 (decision 9a): the route rung 2 or `run retry` stepped the writer
    /// from; the next writer launch records that escalation and clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalated_from: Option<Route>,
    /// Milestone 9.5 ruling T16-9 (1): the test writer's accepted claim's signals, kept
    /// at the hand-over and shown to the implementer's reviewer after the implementer's
    /// own, labelled `test writer`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writer_signals: Vec<crate::run::tiers::Signal>,
    /// How many more the writer's claim had past `SIGNALS_MAX`.
    #[serde(default, skip_serializing_if = "no_more")]
    pub writer_signals_more: u32,
    /// Whole-branch review B, I1: the implementer's accepted claim had its signals read,
    /// so the reviewer is given `contract_patterns::pair_reviewer_note`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub signals_read: bool,
}

/// The class default routes a run is frozen with (decisions 9 and 12): M8a's defaults,
/// or a route applied from `tuning.toml`. Hub is never proposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassRoutes {
    pub s: ClassRoute,
    pub m: ClassRoute,
    pub hub: ClassRoute,
}

impl Default for ClassRoutes {
    fn default() -> Self {
        let route = |strength, effort| ClassRoute { strength, effort };
        ClassRoutes {
            s: route(Strength::Standard, Effort::LOW),
            m: route(Strength::Standard, Effort::MEDIUM),
            hub: route(Strength::Frontier, Effort::HIGH),
        }
    }
}

impl ClassRoutes {
    /// M8a's defaults: a run that froze nothing writes no `class_routes`.
    pub fn is_default(&self) -> bool {
        *self == ClassRoutes::default()
    }
}

/// Decision 9a: one model-list candidate as a run freezes it, with its roster strength.
/// `effort` `None` takes the class's (or the review level's) effort where it is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListCandidate {
    pub runtime: Runtime,
    pub model: String,
    pub strength: Strength,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
}

impl ListCandidate {
    /// The route this candidate gives, at `effort` when it names none.
    pub fn route(&self, effort: Effort) -> Route {
        Route {
            runtime: self.runtime,
            model: self.model.clone(),
            strength: self.strength,
            effort: self.effort.clone().unwrap_or(effort),
        }
    }
}

/// `pick`: the first unskipped candidate, or round-robin (decision 9a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListPolicy {
    #[default]
    First,
    Spread,
}

impl ListPolicy {
    /// The routing history's `pick_policy`.
    pub fn label(self) -> &'static str {
        match self {
            ListPolicy::First => "first",
            ListPolicy::Spread => "spread",
        }
    }
}

/// One frozen `[orchestrator.routes.<name>]` list. Empty: no list.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FrozenList {
    pub candidates: Vec<ListCandidate>,
    pub pick: ListPolicy,
}

impl FrozenList {
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// `list` with each candidate's roster strength (one the roster lacks left out).
    pub fn freeze(list: &config::RouteList, roster: &[ModelEntry]) -> Self {
        let candidates = (list.candidates.iter())
            .filter_map(|c| {
                let entry = crate::run::roster::find(roster, c.runtime, &c.model)?;
                Some(ListCandidate {
                    runtime: c.runtime,
                    model: c.model.clone(),
                    strength: entry.strength,
                    effort: c.effort.clone(),
                })
            })
            .collect();
        let pick = match list.pick {
            config::Pick::First => ListPolicy::First,
            config::Pick::Spread => ListPolicy::Spread,
        };
        FrozenList { candidates, pick }
    }
}

/// Decision 9a: the user's model lists as a run freezes them at start
/// (`RunLimits.route_lists`), so edits, rung-2 sessions and reviewers of a running run
/// use the run's copy, whatever the config says later. Empty lists are not written.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RouteListsFrozen {
    #[serde(skip_serializing_if = "FrozenList::is_empty")]
    pub s: FrozenList,
    #[serde(skip_serializing_if = "FrozenList::is_empty")]
    pub m: FrozenList,
    #[serde(skip_serializing_if = "FrozenList::is_empty")]
    pub hub: FrozenList,
    #[serde(skip_serializing_if = "FrozenList::is_empty")]
    pub review: FrozenList,
    #[serde(skip_serializing_if = "FrozenList::is_empty")]
    pub scout: FrozenList,
    #[serde(skip_serializing_if = "FrozenList::is_empty")]
    pub decider: FrozenList,
    #[serde(skip_serializing_if = "FrozenList::is_empty")]
    pub planner: FrozenList,
    #[serde(skip_serializing_if = "FrozenList::is_empty")]
    pub orchestrator: FrozenList,
    /// Milestone 9.6 decision 10: the brainstormers' list.
    #[serde(skip_serializing_if = "FrozenList::is_empty")]
    pub brainstorm: FrozenList,
}

impl RouteListsFrozen {
    /// `lists` with each candidate's roster strength (a candidate the roster lacks,
    /// which config already dropped, is left out).
    pub fn freeze(lists: &config::RouteLists, roster: &[ModelEntry]) -> Self {
        let f = |list| FrozenList::freeze(list, roster);
        RouteListsFrozen {
            s: f(&lists.s),
            m: f(&lists.m),
            hub: f(&lists.hub),
            review: f(&lists.review),
            scout: f(&lists.scout),
            decider: f(&lists.decider),
            planner: f(&lists.planner),
            orchestrator: f(&lists.orchestrator),
            brainstorm: f(&lists.brainstorm),
        }
    }

    /// Every list with its table name, in config order.
    pub fn named(&self) -> [(&'static str, &FrozenList); 9] {
        [
            ("s", &self.s),
            ("m", &self.m),
            ("hub", &self.hub),
            ("review", &self.review),
            ("scout", &self.scout),
            ("decider", &self.decider),
            ("planner", &self.planner),
            ("orchestrator", &self.orchestrator),
            ("brainstorm", &self.brainstorm),
        ]
    }

    pub fn is_empty(&self) -> bool {
        self.named().iter().all(|(_, list)| list.is_empty())
    }
}

/// Decision 9a: the class-list choice behind a task's worker route, snapshotted when it
/// is made (at the build, an edit or rung 2) for the routing history: the list, each
/// candidate with why it was skipped (`None`: usable), and the index chosen (`None`:
/// every candidate was skipped, or the task's route is its own, so today's resolution
/// holds). `slot` is a `spread` list's rotation position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListPick {
    pub candidates: Vec<RoutingCandidate>,
    pub chosen: Option<u32>,
    pub pick: ListPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
}

impl ListPick {
    /// The chosen candidate's route.
    pub fn chosen_route(&self) -> Option<&Route> {
        let k = self.chosen? as usize;
        self.candidates.get(k).map(|c| &c.route)
    }
}

// Its run.json helpers are shared with milestone 9.6's `model_design_tests.rs`.
#[cfg(test)]
#[path = "model_tuning_tests.rs"]
pub(super) mod tests;

/// Ruling T12-2: a task's share of its run's paused time (`engine/pause.rs`), which the
/// estimate's `done` leaves out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskPaused {
    /// The run's paused total ([`super::Run::paused_total`]) when the phase began.
    #[serde(default)]
    pub base: u64,
    /// The paused seconds inside the task's earlier active phases.
    #[serde(default)]
    pub active: u64,
}

impl TaskPaused {
    pub fn is_zero(&self) -> bool {
        *self == TaskPaused::default()
    }
}

fn no_more(n: &u32) -> bool {
    *n == 0
}
