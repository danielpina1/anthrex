//! M9.17 fix round 2: `run promote` records what its installed check found
//! (`OrchEvent::Installed`, sent by the driver before `EventKind::Promote`), so the
//! promoted run's sub-planners' route agrees with the check: a Codex orchestrator on a
//! machine without Claude plans with Codex.

use proto::{OrchestratorChoice, Runtime};

use super::fixture::*;
use super::promote::{fast, promote};
use crate::run::engine::{EventKind, OrchEvent};
use crate::run::orch::launch::planner_route;

fn installed(claude: bool) -> std::collections::BTreeMap<String, bool> {
    [("claude".to_string(), claude), ("codex".to_string(), true)].into()
}

fn codex() -> Option<OrchestratorChoice> {
    Some(OrchestratorChoice {
        runtime: Runtime::Codex,
        model: None,
    })
}

#[test]
fn a_promotion_records_what_its_installed_check_found() {
    let mut fx = fast();
    fx.run_mut().roster = config::default_roster();
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

/// With both installed, or nothing recorded (a promotion from before milestone 9), the
/// planners keep M8b's route: the peer's frontier model.
#[test]
fn with_claude_installed_the_planners_step_to_its_frontier_model() {
    for recorded in [Some(installed(true)), None] {
        let mut fx = fast();
        fx.run_mut().roster = config::default_roster();
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

/// Whole-branch review B, M8: the promotion reads the `orchestrator` list over what the
/// orchestrator's window finds installed, as a planned start does, so a candidate only
/// the window finds (a `~` entry of `PATH` on macOS) is taken, not skipped.
#[test]
fn a_promotion_reads_its_orchestrator_list_over_the_windows_map() {
    use crate::run::model::{FrozenList, ListCandidate, ListPolicy};
    let mut fx = fast();
    fx.run_mut().roster = config::default_roster();
    fx.run_mut().limits.route_lists.orchestrator = FrozenList {
        candidates: vec![ListCandidate {
            runtime: Runtime::Claude,
            model: "claude-opus-5-5".into(),
            strength: proto::Strength::Frontier,
            effort: None,
        }],
        pick: ListPolicy::First,
    };
    fx.next(EventKind::Orch(OrchEvent::Installed {
        run_id: RUN_ID.into(),
        installed: installed(false),
        window: installed(true),
    }));
    promote(&mut fx, None);
    let record = fx.run().orch.orchestrator.clone().expect("promoted");
    assert_eq!(
        (record.route.runtime, record.route.model.as_str()),
        (Runtime::Claude, "claude-opus-5-5")
    );
    assert_eq!(record.routing.source, "configured_list");
    assert_eq!(
        fx.run().orch.installed,
        installed(false),
        "the record keeps it"
    );
    assert_eq!(fx.run().orch.promote_window, None, "taken by the promotion");
}
