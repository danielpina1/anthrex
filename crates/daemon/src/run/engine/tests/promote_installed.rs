//! M9.17 fix round 2: `run promote` records what its installed check found
//! (`OrchEvent::Installed`, sent by the driver before `EventKind::Promote`). Milestone
//! 9.8: the promoted run's orchestrator is the explicit choice, else its row, and its
//! sub-planners take its `planner` row whatever is installed (D2).

use proto::models::Role;
use proto::{OrchestratorChoice, Runtime};

use super::fixture::*;
use super::promote::{fast, promote};
use crate::run::engine::{EventKind, OrchEvent};
use crate::run::orch::launch::planner_route;
use crate::run::test_support::set_row;

fn installed(claude: bool) -> std::collections::BTreeMap<String, bool> {
    [("claude".to_string(), claude), ("codex".to_string(), true)].into()
}

fn codex() -> Option<OrchestratorChoice> {
    Some(OrchestratorChoice {
        runtime: Runtime::Codex,
        model: None,
        effort: None,
    })
}

#[test]
fn a_promotion_records_what_its_installed_check_found() {
    let mut fx = fast();
    set_row(fx.run_mut(), Role::Planner, "codex:default", None, None);
    fx.next(EventKind::Orch(OrchEvent::Installed {
        run_id: RUN_ID.into(),
        installed: installed(false),
        window: installed(false),
    }));
    assert_eq!(fx.run().orch.installed, installed(false));
    promote(&mut fx, codex());
    let planner = planner_route(fx.run()).expect("a promoted run");
    assert_eq!(
        (planner.runtime, planner.model.as_str()),
        (Runtime::Codex, "")
    );
    // Once the run has an orchestrator, its record is not replaced.
    fx.next(EventKind::Orch(OrchEvent::Installed {
        run_id: RUN_ID.into(),
        installed: installed(true),
        window: installed(true),
    }));
    assert_eq!(fx.run().orch.installed, installed(false));
}

/// Milestone 9.8: whatever is installed or recorded, the planners take the row (the
/// built-in Opus).
#[test]
fn the_planners_take_their_row() {
    for recorded in [Some(installed(true)), None] {
        let mut fx = fast();
        if let Some(installed) = recorded {
            fx.next(EventKind::Orch(OrchEvent::Installed {
                run_id: RUN_ID.into(),
                window: installed.clone(),
                installed,
            }));
        }
        promote(&mut fx, codex());
        let planner = planner_route(fx.run()).unwrap();
        assert_eq!(
            (planner.runtime, planner.model.as_str()),
            (Runtime::Claude, "claude-opus-5-5")
        );
    }
}

/// Milestone 9.8: with no choice the promotion takes the run's `orchestrator` row; the
/// window's map the check sent is spent by it.
#[test]
fn a_promotion_takes_the_orchestrator_row() {
    let mut fx = fast();
    set_row(
        fx.run_mut(),
        Role::Orchestrator,
        "codex:gpt-6.1-sol",
        Some("high"),
        None,
    );
    fx.next(EventKind::Orch(OrchEvent::Installed {
        run_id: RUN_ID.into(),
        installed: installed(false),
        window: installed(true),
    }));
    promote(&mut fx, None);
    let record = fx.run().orch.orchestrator.clone().expect("promoted");
    assert_eq!(
        (record.route.runtime, record.route.model.as_str()),
        (Runtime::Codex, "gpt-6.1-sol")
    );
    assert_eq!(record.routing.source, "role_table");
    assert_eq!(
        fx.run().orch.installed,
        installed(false),
        "the record keeps it"
    );
    assert_eq!(fx.run().orch.promote_window, None, "taken by the promotion");
}
