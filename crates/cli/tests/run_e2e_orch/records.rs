//! The goal form's request (decision 44, spec §16) and the role-routing history across
//! a daemon restart (decision 43, spec §15).

use daemon::run::orch::contract::planned_message;
use proto::{OrchestratorChoice, RunPath, RunReply, RunRequest, RunState, Runtime, ScoutState};
use serde_json::{Value, json};

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_adapt::GOAL_WAIT;
use crate::support::run_harness::{REQUEST_WAIT, RunHarness};
use crate::support::run_orch::{ORCH_WAIT, triage_plan};

#[test]
fn e2e_goal_form_request_matches_the_cli() {
    let h = harness("");
    h.decider("triage", 2, triage_plan());
    let goal = "add the files";
    let cli = h.start_goal_id(goal, &["--orchestrator", "claude"]);

    // The goal form's request: tagged, the plan gate on, the orchestrator chosen, and
    // unconfined checks only where the platform cannot confine them.
    let request = RunRequest::StartGoal {
        goal: goal.into(),
        dir: h.repo.clone(),
        yes: false,
        trust_project: false,
        unconfined_checks: !cfg!(target_os = "macos"),
        orchestrator: Some(OrchestratorChoice {
            runtime: Runtime::Claude,
            model: None,
        }),
    };
    let RunReply::Triaged {
        triage,
        run_id: Some(form),
        message,
        request_id,
    } = h.tagged(7, request, GOAL_WAIT)
    else {
        panic!("the form's goal did not start a run\n{}", h.log_tail());
    };
    assert_eq!(request_id, Some(7));
    assert_eq!(message, planned_message(&triage, &form, RunPath::Plan));
    assert_ne!(form, cli);

    let (a, b) = (h.run(&cli).unwrap(), h.run(&form).unwrap());
    for run in [&a, &b] {
        assert_eq!(run.state, RunState::Planning, "{}", run.run_id);
        assert_eq!(run.path, Some(RunPath::Plan));
        assert!(run.tasks.is_empty());
        assert_eq!(run.goal, goal);
    }
    let (ta, tb) = (a.triage.unwrap(), b.triage.unwrap());
    assert_eq!(
        (ta.kinds, ta.scale, ta.source, ta.reason),
        (tb.kinds, tb.scale, tb.source, tb.reason)
    );
    let (oa, ob) = (a.orchestrator.unwrap(), b.orchestrator.unwrap());
    assert_eq!(oa.route, ob.route);
    assert_eq!(oa.route.runtime, Runtime::Claude);
    // Both orchestrators start, each in its own window.
    let (wa, wb) = (h.orchestrator_window(&cli), h.orchestrator_window(&form));
    assert_ne!(wa, wb);
}

/// This run's `role_route` history lines.
fn routes(h: &RunHarness, run: &str) -> Vec<Value> {
    h.history_lines("role_route")
        .into_iter()
        .filter(|l| l["run_id"] == run)
        .collect()
}

/// `anthrex run stats --json` for the harness repository.
fn stats(h: &RunHarness) -> Value {
    let repo = h.repo.display().to_string();
    let out = h.anthrex(&["run", "stats", "--json", "--dir", &repo]);
    ok(&out);
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn e2e_role_routing_records_survive_restart() {
    let mut h = harness("");
    h.script("scout-slow-1", &[json!({"hang": {}})]);
    let steps = [
        prompt(),
        call(
            "spawn_scout",
            json!({"id": "slow", "question": "What is slow?", "area": ["tests/**"]}),
        ),
        read(None),
    ];
    let (run, _) = start(&h, &steps);
    h.wait_run(
        &run,
        |r| {
            r.scouts
                .first()
                .is_some_and(|s| s.state == ScoutState::Working)
        },
        ORCH_WAIT,
    );
    // Both records are kept in the run before their sessions start (decision 43).
    let kept = h.run_json(&run)["role_routing_decisions"]
        .as_array()
        .cloned()
        .unwrap();
    let role = |records: &[Value], role: &str| -> Value {
        records
            .iter()
            .find(|r| r["role"] == role)
            .cloned()
            .unwrap_or_else(|| panic!("no {role} record: {records:#?}"))
    };
    let (orch, scout) = (role(&kept, "orchestrator"), role(&kept, "scout"));
    let before = stats(&h);
    assert!(
        routes(&h, &run).is_empty(),
        "appended before the sessions ended"
    );

    h.restart_daemon(&[]);
    h.wait_run(&run, |r| r.state == RunState::Paused, REQUEST_WAIT);
    let lines = routes(&h, &run);
    assert_eq!(lines.len(), 2, "{lines:#?}");
    for (kept, line) in [
        (&orch, role(&lines, "orchestrator")),
        (&scout, role(&lines, "scout")),
    ] {
        assert_eq!(line["record_id"], kept["record_id"]);
        assert_eq!(line["session_id"], kept["session_id"]);
        assert_eq!(
            line["candidates"], kept["candidates"],
            "the dispatch-time snapshot"
        );
        assert_eq!(line["chosen"], kept["chosen"]);
        assert_eq!(line["outcome"], "interrupted", "{line}");
    }
    assert_ne!(orch["session_id"], scout["session_id"]);
    assert_ne!(orch["record_id"], scout["record_id"]);
    // M8b's aggregates do not count the routing records (decision 43).
    assert_eq!(stats(&h), before);

    // The resumed orchestrator is a new session with a record of its own, ended with
    // the run.
    ok(&h.anthrex(&["run", "resume", &run]));
    h.wait_run(
        &run,
        |r| r.orchestrator.as_ref().is_some_and(|o| o.live),
        ORCH_WAIT,
    );
    ok(&h.anthrex(&["run", "reject", &run, "--confirm", &run]));
    h.wait_run(&run, |r| r.state == RunState::Discarded, REQUEST_WAIT);
    let lines =
        crate::support::run_plans::until("the second session's record", REQUEST_WAIT, || {
            let lines = routes(&h, &run);
            (lines.len() == 3).then_some(lines)
        });
    let sessions: Vec<&Value> = lines
        .iter()
        .filter(|l| l["role"] == "orchestrator")
        .map(|l| &l["session_id"])
        .collect();
    assert_eq!(sessions.len(), 2, "{lines:#?}");
    assert_ne!(sessions[0], sessions[1]);
    let second = lines
        .iter()
        .find(|l| l["role"] == "orchestrator" && l["session_id"] != orch["session_id"])
        .unwrap();
    assert_eq!(second["candidates"], orch["candidates"], "{second}");
    assert_eq!(second["trigger"], "restart", "{second}");
    // The discarded run adds its run record; the task aggregates stay as they were.
    let after = stats(&h);
    for key in ["rows", "task_records", "problems", "decider_calls"] {
        assert_eq!(after[key], before[key], "{key}");
    }
}
