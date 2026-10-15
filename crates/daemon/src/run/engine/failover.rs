//! Milestone 9.16 decisions 36–45 (U17): cross-runtime failover. Where the gate refuses
//! a fresh session's own runtime for a quota trip (never for the hold), the session may
//! start instead on its role row's fallback ("if it struggles") model when that model is
//! on the other runtime, which the gate allows, is installed for the run, did not fail in
//! the task, keeps the overlap rule and is not a possible author's model for a reviewer.
//! Only decision 11's gated entry points ask ([`admit`]); a resumed or in-flight session
//! never fails over. The fallback route is written where the session's route lives
//! ([`record`]), so the escalation ladder continues from it and nothing switches back.
//! Pure (design decision 2): no file, no clock, no lock.

use std::collections::BTreeSet;

use proto::models::Role;
use proto::{RoleRoutingDecision, Route, Runtime};

use super::brakes::{Gate, Refusal};
use super::quota_book::Trip;
use super::requests::log;
use crate::run::model::{FailoverRecord, Run, writes};
use crate::run::model_roles::{Mover, RunModels, failed_in, failed_routes, missing, runtime_open};

/// Decision 42: the newest failover records a run keeps.
pub const FAILOVERS_KEPT: usize = 32;
/// Decision 42: a routing record's `source` for a session that failed over.
pub const QUOTA_FAILOVER: &str = "quota_failover";
/// Decision 42: the failover attention lines shown before `… and <n> more failovers`.
const SHOWN: usize = 3;

/// Who is starting (decision 42's `<who>`): a task's session (worker, test writer,
/// research or review task, by index) or a run-bound reader (by name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Starter<'a> {
    Worker(usize),
    TestWriter(usize),
    Reader(usize),
    /// A sub-planner, by its epic.
    Planner(&'a str),
    /// A run scout, by its id.
    Scout(&'a str),
    /// A document reviewer, by its label (`spec-r1`).
    DocReviewer(&'a str),
}

impl Starter<'_> {
    fn task(self) -> Option<usize> {
        match self {
            Starter::Worker(i) | Starter::TestWriter(i) | Starter::Reader(i) => Some(i),
            _ => None,
        }
    }

    /// Decision 42's `<who>`: `t3`, `planner <epic>`, `scout <id>`, `doc review <label>`.
    pub fn who(self, run: &Run) -> String {
        match self {
            Starter::Worker(i) | Starter::TestWriter(i) | Starter::Reader(i) => {
                run.tasks[i].id().to_string()
            }
            Starter::Planner(epic) => format!("planner {epic}"),
            Starter::Scout(id) => format!("scout {id}"),
            Starter::DocReviewer(label) => format!("doc review {label}"),
        }
    }
}

/// A session moving to its row's fallback: the braked route it moves from, the route it
/// starts on, and the trip that refused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failover {
    pub from: Route,
    pub to: Route,
    pub trip: Trip,
}

/// What the gate lets a fresh session do at a gated entry point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admit {
    /// Its own runtime is allowed.
    Go,
    /// Decisions 36–41: it starts on its row's fallback instead.
    FailOver(Failover),
    /// It waits (decisions 11 and 13).
    Wait,
}

/// The gate's answer for `starter` on `current`, a session of `role`'s row (decision 37
/// (a)–(g); (h), the room on the fallback's runtime, stays at the call site).
pub fn admit(gate: &Gate, run: &Run, starter: Starter<'_>, role: Role, current: &Route) -> Admit {
    let trip = match gate.refusal(current.runtime) {
        None => return Admit::Go,
        // Decision 43: the hold blocks failover too.
        Some(Refusal::Held) => return Admit::Wait,
        Some(Refusal::Quota(trip)) => trip,
    };
    let Some(to) = candidate(run, role, current) else {
        return Admit::Wait;
    };
    let models = run.limits.models();
    // (c) the fallback's runtime is not braked and the hold is off.
    let allowed = gate.allows(to.runtime);
    // (e) ruling RL-1: the fallback did not fail in this task.
    let failed =
        (starter.task()).is_some_and(|i| failed_in(models, &failed_routes(&run.tasks[i]), &to));
    // (f) decision 39: the overlap rule.
    let overlap = match starter {
        Starter::Worker(i) => !runtime_open(run, i, to.runtime, Mover::Worker),
        Starter::TestWriter(i) => !runtime_open(run, i, to.runtime, Mover::Transient),
        _ => false,
    };
    // (g) decision 40: never an author's model for a reviewer.
    let author = role == Role::Reviewer && authors(run).iter().any(|a| models.same_model(a, &to));
    match allowed && !failed && !overlap && !author {
        true => Admit::FailOver(Failover {
            from: current.clone(),
            to,
            trip,
        }),
        false => Admit::Wait,
    }
}

/// Decision 37 (a), (b), (d): `role`'s row's fallback at its default effort, from the
/// run's frozen table, when `current` is the row's own model, the fallback is on another
/// runtime, and that runtime is installed for the run.
pub fn candidate(run: &Run, role: Role, current: &Route) -> Option<Route> {
    let models = run.limits.models();
    if !models.same_model(current, &models.route(role)) {
        return None;
    }
    let fallback = models.choice(role).fallback.as_ref()?;
    let ok = fallback.runtime != current.runtime && !missing(&run.orch.installed, fallback.runtime);
    ok.then(|| RunModels::route_of(fallback, None))
}

/// Decision 40's possible authors: the run's orchestrator route, and the route of every
/// writing round of every task in the run.
pub fn authors(run: &Run) -> Vec<Route> {
    let orchestrator = run.orch.orchestrator.as_ref().map(|o| o.route.clone());
    let rounds = (run.tasks.iter()).flat_map(|t| {
        (t.rounds.iter())
            .filter(|r| writes(t, r))
            .map(|r| r.route.clone())
    });
    orchestrator.into_iter().chain(rounds).collect()
}

/// Decision 42's run-log line.
pub fn log_text(who: &str, failover: &Failover) -> String {
    let Failover { from, to, trip } = failover;
    format!(
        "{who}: {} braked ({} at {}%); started on {} (fallback)",
        from.runtime,
        trip.window.label(),
        trip.used.percent(),
        RunModels::label_of(to)
    )
}

/// Records the failover (decisions 41, 42): the run-log line, `Run.failovers` (the
/// newest [`FAILOVERS_KEPT`]), and the fallback route written where the session's route
/// lives, before its spec is built (ruling C29). A run scout's route is read back from
/// its record ([`scout_route`]).
pub fn record(run: &mut Run, starter: Starter<'_>, failover: &Failover, now: u64) {
    let who = starter.who(run);
    log(run, now, log_text(&who, failover));
    run.failovers.push(FailoverRecord {
        who,
        from: failover.from.clone(),
        to: failover.to.clone(),
        at: now,
    });
    let over = run.failovers.len().saturating_sub(FAILOVERS_KEPT);
    run.failovers.drain(..over);
    let to = failover.to.clone();
    match starter {
        Starter::Worker(i) => {
            let reviewer = (run.tasks[i].review_level).map(|_| {
                let models = run.limits.models();
                models.reviewer_route(&to).0
            });
            let task = &mut run.tasks[i];
            task.route = to;
            task.review_route = reviewer;
        }
        Starter::TestWriter(i) => {
            if let Some(pair) = run.tasks[i].pair.as_mut() {
                pair.writer_route = to;
            }
        }
        Starter::Reader(i) => run.tasks[i].route = to,
        Starter::Planner(epic) => {
            if let Some(e) = run.orch.epics.iter_mut().find(|e| e.epic == epic) {
                e.route = to;
            }
        }
        Starter::Scout(_) => {}
        Starter::DocReviewer(label) => {
            let design = run.orch.design.as_mut();
            if let Some(agent) = design.and_then(|d| d.reviewer.as_mut())
                && agent.label == label
            {
                agent.route = to;
            }
        }
    }
}

/// A reader's start at a gated entry point: `admit`, then the start's blind
/// announcement on the runtime it runs on and, for a failover, [`record`]. `None`: it
/// waits; `Some(from)`: it starts, `from` the braked route it moved from when it failed
/// over.
pub(super) fn start(
    gate: &mut Gate,
    run: &mut Run,
    starter: Starter<'_>,
    admit: Admit,
    current: Runtime,
    now: u64,
) -> Option<Option<Route>> {
    match admit {
        Admit::Wait => None,
        Admit::Go => {
            gate.started(run, current, now);
            Some(None)
        }
        Admit::FailOver(f) => {
            gate.started(run, f.to.runtime, now);
            record(run, starter, &f, now);
            Some(Some(f.from))
        }
    }
}

/// Decision 42 for a role routing record: the session failed over from `from`.
pub fn mark(decision: &mut RoleRoutingDecision, from: Option<Route>) {
    if let Some(from) = from {
        decision.source = QUOTA_FAILOVER.to_string();
        decision.failover = Some(from);
    }
}

/// The failover task `task`'s session of `role` made onto `chosen`, if it made one: the
/// newest record for the task whose `to` is `chosen` and whose `from` is the row's model.
pub fn task_from(run: &Run, task: usize, role: Role, chosen: &Route) -> Option<Route> {
    let models = run.limits.models();
    let id = run.tasks[task].id();
    (run.failovers.iter())
        .rfind(|f| {
            f.who == id && f.to == *chosen && models.same_model(&f.from, &models.route(role))
        })
        .map(|f| f.from.clone())
}

/// The run scout `id`'s failover record, if it failed over: the route it runs on is the
/// record's `to`, else the `research` row's (`launch::scout_route`).
pub fn scout_route<'a>(run: &'a Run, id: &str) -> Option<&'a FailoverRecord> {
    let who = Starter::Scout(id).who(run);
    run.failovers.iter().rfind(|f| f.who == who)
}

/// Decision 42's attention lines: per record whose `from` runtime is in `braked` now,
/// newest first, the newest [`SHOWN`], then `… and <n> more failovers`.
pub fn attention(braked: &BTreeSet<Runtime>, run: &Run) -> Vec<String> {
    let live: Vec<&FailoverRecord> = (run.failovers.iter().rev())
        .filter(|f| braked.contains(&f.from.runtime))
        .collect();
    let mut lines: Vec<String> = (live.iter().take(SHOWN))
        .map(|f| common::brakes_text::failover_attention_text(f.from.runtime, &f.who, f.to.runtime))
        .collect();
    if live.len() > SHOWN {
        lines.push(format!("… and {} more failovers", live.len() - SHOWN));
    }
    lines
}
