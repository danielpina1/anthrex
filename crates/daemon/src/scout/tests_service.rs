//! M8b.9 review fixes: `ScoutService`'s own guards, on a manager that never starts a
//! process (both runtime commands are paths that do not exist).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use proto::{AgentRole, ScoutKind, ScoutReport, ScoutState, ToolCall};

use super::{Scout, ScoutService};
use crate::manager::{ManagerConfig, WindowManager};
use crate::scout::machine::{ScoutEvent, ScoutMachine};
use crate::scout::spec::{ScoutContext, ScoutSpec};

const ID: &str = "onboarding-1";
const WINDOW: u32 = 5;

fn service() -> Arc<ScoutService> {
    let mut config = ManagerConfig::new("/tmp/ax-m8b9-unused.sock".into(), "/bin/sh".into());
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    let (manager, _events) = WindowManager::new(config);
    let orchestrator = config::Orchestrator::default();
    ScoutService::new(
        manager,
        ScoutContext {
            roster: orchestrator.models.clone(),
            default_runtime: proto::Runtime::Claude,
            scouts: orchestrator.scouts.clone(),
            claude: orchestrator.claude.clone(),
            caps: crate::headless::argv::CLI_CAPS,
            data_dir: PathBuf::from("/nonexistent/anthrex-test/data"),
        },
    )
}

fn spec(run_id: Option<&str>) -> ScoutSpec {
    ScoutSpec {
        id: ID.into(),
        kind: ScoutKind::Area,
        run_id: run_id.map(String::from),
        question: "q".into(),
        first_turn: "go".into(),
        cwd: "/nonexistent/anthrex-test/repo".into(),
        project: "/nonexistent/anthrex-test/repo".into(),
        web: false,
        codex_config: Vec::new(),
        base_sha: String::new(),
        repo_paths: Vec::new(),
    }
}

/// Puts a working scout straight into the table, started at unix time 0.
fn insert(service: &ScoutService, window_id: Option<u32>) {
    let spec = spec(Some("r1"));
    let route = crate::scout::spec::scout_route(&service.ctx);
    let (machine, _) = crate::scout::machine::step(
        ScoutMachine::default(),
        ScoutEvent::Start { now: 0 },
        &crate::scout::machine::ScoutLimits::new(&service.ctx.scouts, route.runtime),
    );
    let mut table = crate::lock(&service.table);
    if let Some(window) = window_id {
        table.by_window.insert(window, ID.into());
    }
    table.scouts.insert(
        ID.into(),
        Scout {
            spec,
            route,
            window_id,
            machine,
            outcome: None,
            storing: false,
            report: None,
            report_bytes: None,
            ended_at: None,
            turn_ended_pids: HashSet::new(),
            kill_on_bind: false,
            installed: false,
            kill_at: None,
            remove_at: None,
        },
    );
}

fn call(tool: &str) -> ToolCall {
    ToolCall {
        run_id: "r1".into(),
        task_id: None,
        role: AgentRole::Scout,
        window_id: WINDOW,
        tool: tool.into(),
        args: serde_json::json!({"summary": "s", "files": []}),
        scout_id: Some(ID.into()),
        epic: None,
    }
}

fn report() -> ScoutReport {
    serde_json::from_value(serde_json::json!({
        "id": ID, "kind": "area", "run_id": "r1", "question": "q", "summary": "s",
        "files": [], "route": {"runtime": "claude", "model": "m", "strength": "fast",
        "effort": "low"}, "window_id": WINDOW, "started_at": 0, "finished_at": 0,
        "tool_calls": 0, "usage": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0},
    }))
    .unwrap()
}

/// Ruling M1: a report in flight refuses a second one.
#[tokio::test]
async fn a_report_in_flight_refuses_another() {
    let service = service();
    insert(&service, Some(WINDOW));
    assert!(service.claim(&call("submit_scout_report")).is_ok());
    assert_eq!(
        service.claim(&call("submit_scout_report")).err(),
        Some(format!("a report for scout {ID} was already recorded"))
    );
    service.unclaim(ID);
    assert!(service.claim(&call("submit_scout_report")).is_ok());
}

/// Ruling M1: a scout that fails while its report is written does not take it.
#[tokio::test]
async fn a_scout_that_failed_during_the_write_refuses_its_report() {
    let service = service();
    insert(&service, Some(WINDOW));
    service.claim(&call("submit_scout_report")).unwrap();
    // Past `scouts.timeout_secs` (900) since the start at 0: the scout fails.
    service.drive(ID, ScoutEvent::Tick { now: 10_000 });
    assert_eq!(
        service.commit(ID, report(), 10).err(),
        Some(format!("scout {ID} is failed"))
    );
    let table = crate::lock(&service.table);
    let scout = &table.scouts[ID];
    assert_eq!(scout.machine.state, ScoutState::Failed);
    assert!(scout.report.is_none() && !scout.storing);
}

/// Ruling M5: only `submit_scout_report` is taken.
#[tokio::test]
async fn another_tool_is_refused() {
    let service = service();
    insert(&service, Some(WINDOW));
    assert_eq!(
        service.claim(&call("task_done")).err(),
        Some("tool task_done is not available to the scout role".into())
    );
    assert!(!crate::lock(&service.table).scouts[ID].storing);
}

/// Ruling M4: a kill asked for before the window is bound is done at the bind that
/// follows `create_headless`, not at an early bind from the session feed.
#[tokio::test]
async fn a_stop_before_the_bind_is_remembered() {
    let service = service();
    insert(&service, None);
    service.stop(ID);
    let owed = |service: &ScoutService| crate::lock(&service.table).scouts[ID].kill_on_bind;
    assert!(owed(&service));
    service.bind(ID, WINDOW, false);
    assert!(owed(&service));
    service.bind(ID, WINDOW, true);
    assert!(!owed(&service));
}

/// Ruling M7: a run id that is not a slug never reaches the report's path.
#[tokio::test]
async fn a_bad_run_id_is_refused_at_start() {
    let service = service();
    for run in ["../escape", "", "R1", "a/b"] {
        let error = service.start(spec(Some(run))).await.unwrap_err();
        assert_eq!(error.to_string(), format!("invalid run id {run:?}"));
    }
    assert!(crate::lock(&service.table).scouts.is_empty());
}

/// Ruling R-T9-3: a stop after the session feed bound the window early, but before
/// `create_headless` installed the process, is still owed to the start's bind.
#[tokio::test]
async fn a_stop_after_an_early_bind_is_still_owed_to_the_install() {
    let service = service();
    insert(&service, None);
    service.bind(ID, WINDOW, false);
    service.stop(ID);
    let owed = |service: &ScoutService| crate::lock(&service.table).scouts[ID].kill_on_bind;
    assert!(owed(&service));
    service.bind(ID, WINDOW, true);
    assert!(!owed(&service));
    // Once installed, a kill is the window's own, never owed.
    let (machine, _) = crate::scout::machine::step(
        ScoutMachine::default(),
        ScoutEvent::Start { now: 0 },
        &crate::scout::machine::ScoutLimits::new(
            &service.ctx.scouts,
            crate::scout::spec::scout_route(&service.ctx).runtime,
        ),
    );
    crate::lock(&service.table)
        .scouts
        .get_mut(ID)
        .unwrap()
        .machine = machine;
    service.stop(ID);
    assert!(!owed(&service));
}

/// Ruling R-T9-2: a finished scout leaves the table when its window is removed.
#[tokio::test]
async fn a_finished_scout_leaves_the_table_with_its_window() {
    let service = service();
    insert(&service, Some(WINDOW));
    service.stop(ID);
    {
        let mut table = crate::lock(&service.table);
        let scout = table.scouts.get_mut(ID).unwrap();
        assert!(
            scout.remove_at.is_some(),
            "a failed scout's window is removed later"
        );
        scout.remove_at = Some(std::time::Instant::now());
    }
    service.tick();
    let table = crate::lock(&service.table);
    assert!(!table.scouts.contains_key(ID));
    assert!(!table.by_window.contains_key(&WINDOW));
}
