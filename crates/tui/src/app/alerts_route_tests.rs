use super::*;
use RouteKind::*;
use proto::{BlockReason as B, DeliveryAlertKind as D};

/// OFA §4.1, row by row: (kind, route with a live orchestrator).
fn table() -> Vec<(RouteKind, Route)> {
    let mut rows = Vec::new();
    for reason in [
        B::Question,
        B::MisSized,
        B::DepCancelled,
        B::Environment,
        B::Human,
        B::Conflict,
    ] {
        rows.push((
            Blocked {
                reason: Some(reason),
                user_only: false,
            },
            Route::Orchestrator,
        ));
    }
    rows.push((
        Blocked {
            reason: None,
            user_only: false,
        },
        Route::Orchestrator,
    ));
    rows.push((
        Blocked {
            reason: Some(B::Environment),
            user_only: true,
        },
        Route::User,
    ));
    rows.push((Halted { user_only: false }, Route::Orchestrator));
    rows.push((Halted { user_only: true }, Route::User));
    for stage in [StageAlert::Held, StageAlert::PropagateRed, StageAlert::Red] {
        rows.push((Stage(stage), Route::Orchestrator));
    }
    for kind in [
        D::CiHandedToUser,
        D::ReviewRoundsOverCap,
        D::HostOpHeld,
        D::PrClosedUnmerged,
    ] {
        rows.push((
            Delivery {
                kind,
                user_only: false,
            },
            Route::Orchestrator,
        ));
    }
    rows.push((
        Delivery {
            kind: D::HostOpHeld,
            user_only: true,
        },
        Route::User,
    ));
    rows.push((
        Delivery {
            kind: D::GhLoggedOut,
            user_only: false,
        },
        Route::User,
    ));
    rows.push((Gate, Route::User));
    rows.push((Hold, Route::Both));
    for kind in [
        Accept,
        Proposal,
        Orchestrator,
        OrchestratorAsks,
        OrchestratorStuck,
    ] {
        rows.push((kind, Route::User));
    }
    rows
}

#[test]
fn every_kind_with_a_live_orchestrator_routes_as_ofa_says() {
    for (kind, route) in table() {
        assert_eq!(alert_route(kind, true), route, "{kind:?}");
    }
}

#[test]
fn with_no_living_orchestrator_every_kind_is_the_users() {
    for (kind, _) in table() {
        assert_eq!(alert_route(kind, false), Route::User, "{kind:?}");
    }
}

fn run_with(live: bool, stuck: Option<proto::OrchestratorStuck>) -> proto::RunInfo {
    let mut run = crate::tree::alert_fixtures::with_orch(
        crate::tree::alert_fixtures::at("r", proto::RunState::Running, 1),
        9,
    );
    let o = run.orchestrator.as_mut().unwrap();
    o.live = live;
    o.stuck = stuck;
    run
}

#[test]
fn a_stalled_or_dead_orchestrator_does_not_live() {
    assert!(orchestrator_lives(&run_with(true, None)));
    assert!(!orchestrator_lives(&run_with(
        true,
        Some(proto::OrchestratorStuck::Stalled { since: 1 })
    )));
    assert!(!orchestrator_lives(&run_with(
        false,
        Some(proto::OrchestratorStuck::Dead { since: None })
    )));
    assert!(!orchestrator_lives(&run_with(false, None)));
}
