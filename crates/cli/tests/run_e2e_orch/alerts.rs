//! Milestone 9.9 (OFA §6): a worker blocks on its environment; with a live (fake)
//! orchestrator no user alert appears, the orchestrator's retry runs, the task merges,
//! and the run shows "orchestrator handled 1". And an ask_user answered from the TUI.

use proto::{RunReply, RunState, TaskState};
use serde_json::json;
use tui::app::{AlertKey, App, alerts};
use tui::settings::UiSettings;

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_harness::{REQUEST_WAIT, RUN_WAIT};
use crate::support::run_orch::ORCH_WAIT;
use crate::support::run_plans::{approve, commit, done};

fn tui_now(h: &crate::support::run_harness::RunHarness) -> App {
    let mut app = App::new(h.windows(), "/tmp".into(), UiSettings::default());
    let _ = app.set_terminal_size(120, 40);
    app.on_daemon(proto::DaemonMsg::Run(RunReply::Snapshot(h.snapshot())));
    app
}

#[test]
fn e2e_an_environment_block_is_the_orchestrators_and_it_retries() {
    let h = harness("");
    let go = h.dir.path().join("go");
    h.script(
        "worker-t1-1",
        &[
            wait_file(&go),
            call(
                "task_blocked",
                json!({"kind": "environment", "reason": "the build cache is locked"}),
            ),
            read(None),
        ],
    );
    h.script(
        "worker-t1-2",
        &[commit("a.txt", "t1\n"), done("added a.txt")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], json!({})))],
            json!({"submit": true}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        read(Some("t1 blocked (environment)")),
        read(Some("retry now")),
        edit_plan(
            vec![json!({"op": "retry", "task_id": "t1", "reason": "the cache lock was transient"})],
            json!({}),
        ),
        marker(),
        read(None),
    ];
    let (run, window) = start(&h, &steps);
    approve_plan(&h, &run);
    std::fs::write(&go, "").unwrap();
    // The orchestrator has read the block's wake-up and waits; the task is blocked.
    h.wait_messages(ORCH, 1, ORCH_WAIT);
    h.wait_orchestrator_idle(window);
    let info = wait_task(&h, &run, "t1", TaskState::Blocked);
    let block = task(&info, "t1").block.clone().unwrap();
    assert!(!block.user_only, "{block:?}");
    assert!(tui::app::orchestrator_lives(&info));
    let app = tui_now(&h);
    let keys: Vec<AlertKey> = alerts(&app).into_iter().map(|a| a.key).collect();
    assert!(
        !keys.iter().any(|k| matches!(k, AlertKey::Blocked { .. })),
        "no user alert for an orchestrator-routed block: {keys:?}"
    );
    h.type_into(window, b"retry now\r");
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);
    wait_passed(&h, 1);
    assert_eq!(task(&info, "t1").state, TaskState::Merged);
    let o = info.orchestrator.clone().unwrap();
    assert_eq!(o.handled_total, 1);
    assert_eq!(
        (o.handled[0].op.as_str(), o.handled[0].target.as_str()),
        ("retry", "t1")
    );
    // Decision 27's Alerts footer: `<goal>: orchestrator handled <N> · o on its alerts
    // opens the run` (the interface sketch's bare "orchestrator handled 3" predates it).
    assert_eq!(
        tui::app::handled_line(&info).as_deref(),
        Some("add the files: orchestrator handled 1 · o on its alerts opens the run")
    );
    let log = h.run_json(&run)["log"].to_string();
    assert!(
        log.contains("orchestrator: retried t1 — the cache lock was transient"),
        "{log}"
    );
}

#[test]
fn e2e_ask_user_is_one_alert_and_the_tuis_answer_reaches_the_orchestrator() {
    let h = harness("");
    let steps = [
        prompt(),
        call(
            "ask_user",
            json!({"question": "tabs or spaces?", "options": ["tabs", "spaces"], "context": "no style guide"}),
        ),
        read(Some("the user chose: tabs")),
        marker(),
        read(None),
    ];
    let (run, _) = start(&h, &steps);
    let info = h.wait_run(
        &run,
        |r| r.orchestrator.as_ref().is_some_and(|o| o.ask.is_some()),
        ORCH_WAIT,
    );
    let app = tui_now(&h);
    let asks: Vec<_> = alerts(&app)
        .into_iter()
        .filter(|a| matches!(a.key, AlertKey::OrchestratorAsks(_)))
        .collect();
    assert_eq!(asks.len(), 1, "{asks:#?}");
    assert_eq!(asks[0].text, "orchestrator asks: tabs or spaces?");
    let request = tui::app::answer_request(&info, Some(0)).expect("a pending question");
    match h.tagged(7, request, REQUEST_WAIT) {
        RunReply::Done { request, .. } => assert_eq!(request, proto::run_wire::request::ANSWER_ASK),
        other => panic!("{other:?}"),
    }
    wait_passed(&h, 1);
    let info = h.run(&run).unwrap();
    assert!(info.orchestrator.unwrap().ask.is_none());
}
