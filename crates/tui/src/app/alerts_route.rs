//! Milestone 9.9 (OFA §4.1, decisions 21 and 22): who an alert is for. While a run's
//! orchestrator lives, the alerts it can act on are its, not the user's; the user
//! keeps approvals, accept, proposals, `ask_user`, what only they can fix, and an
//! orchestrator that is stuck. With none living every alert is the user's. Pure: no I/O.

use super::alerts::AlertKey;
use super::alerts_stage::StageAlert;
use proto::{BlockReason, DeliveryAlertKind, RunInfo};

/// Who an alert is routed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    User,
    Orchestrator,
    /// The user sees it and the orchestrator may also act on it (a hold).
    Both,
}

/// The facts of an alert that routing reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteKind {
    Blocked {
        reason: Option<BlockReason>,
        user_only: bool,
    },
    Halted {
        user_only: bool,
    },
    Stage(StageAlert),
    Delivery {
        kind: DeliveryAlertKind,
        user_only: bool,
    },
    Gate,
    Hold,
    Accept,
    Proposal,
    Orchestrator,
    OrchestratorAsks,
    OrchestratorStuck,
}

/// OFA §4.1's table. With no living orchestrator every alert is the user's.
pub fn alert_route(kind: RouteKind, orchestrator_lives: bool) -> Route {
    if !orchestrator_lives {
        return Route::User;
    }
    match kind {
        RouteKind::Blocked { user_only, .. } | RouteKind::Halted { user_only } => {
            if user_only {
                Route::User
            } else {
                Route::Orchestrator
            }
        }
        RouteKind::Stage(_) => Route::Orchestrator,
        RouteKind::Delivery { kind, user_only } => {
            if user_only || kind == DeliveryAlertKind::GhLoggedOut {
                Route::User
            } else {
                Route::Orchestrator
            }
        }
        RouteKind::Hold => Route::Both,
        RouteKind::Gate
        | RouteKind::Accept
        | RouteKind::Proposal
        | RouteKind::Orchestrator
        | RouteKind::OrchestratorAsks
        | RouteKind::OrchestratorStuck => Route::User,
    }
}

/// `key`'s routing facts, read off its run (the daemon's `user_only` marks).
pub fn route_kind(key: &AlertKey, run: Option<&RunInfo>) -> RouteKind {
    match key {
        AlertKey::Blocked { task, .. } => {
            let block = run
                .and_then(|run| run.tasks.iter().find(|t| t.id == *task))
                .and_then(|task| task.block.as_ref());
            RouteKind::Blocked {
                reason: block.map(|block| block.reason),
                user_only: block.is_some_and(|block| block.user_only),
            }
        }
        AlertKey::Halted(_) => RouteKind::Halted {
            user_only: run.is_some_and(|run| run.halt_user_only),
        },
        AlertKey::Stage { kind, .. } => RouteKind::Stage(*kind),
        AlertKey::Delivery { kind, .. } => {
            let user_only = run.is_some_and(|run| {
                super::alerts::delivery_alerts(run)
                    .into_iter()
                    .any(|(other, alert)| other == *key && alert.user_only)
            });
            RouteKind::Delivery {
                kind: *kind,
                user_only,
            }
        }
        AlertKey::Gate(_) => RouteKind::Gate,
        AlertKey::Hold { .. } => RouteKind::Hold,
        AlertKey::Accept(_) => RouteKind::Accept,
        // Milestone 9.10 decision 34: a project alert, the user's; the list never asks.
        AlertKey::Proposal(_) | AlertKey::Setup(_) => RouteKind::Proposal,
        AlertKey::Orchestrator(_) => RouteKind::Orchestrator,
        AlertKey::OrchestratorAsks(_) => RouteKind::OrchestratorAsks,
        AlertKey::OrchestratorStuck(_) => RouteKind::OrchestratorStuck,
    }
}

/// OFA §4.1: "lives" is a live window that is not stalled or dead (decision 21).
pub fn orchestrator_lives(run: &RunInfo) -> bool {
    run.orchestrator
        .as_ref()
        .is_some_and(|orch| orch.live && orch.stuck.is_none())
}

/// Whether the alert is the user's only because the cause is user-only (the `you`
/// badge, decision 23): the orchestrator lives and the daemon marked it so.
pub(super) fn is_you(kind: RouteKind, orchestrator_lives: bool) -> bool {
    orchestrator_lives
        && matches!(
            kind,
            RouteKind::Blocked {
                user_only: true,
                ..
            } | RouteKind::Halted { user_only: true }
                | RouteKind::Delivery {
                    user_only: true,
                    ..
                }
        )
}

#[cfg(test)]
#[path = "alerts_route_tests.rs"]
mod tests;
