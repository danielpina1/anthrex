//! The role-routing history of the large path and of pre-run triage (decision 43,
//! spec §15): one record per session with its dispatch-time candidates and a factual
//! outcome, and a triage record even when the run it answered is never created.

use proto::RunState;
use serde_json::{Value, json};

use crate::common::*;
use crate::epics::*;
use crate::support::orch_script::*;
use crate::support::run_harness::{REQUEST_WAIT, RunHarness, git_in};
use crate::support::run_orch::{ORCH_WAIT, triage_plan};
use crate::support::run_plans::until as poll;

/// `role_route` lines whose `run_id` is `run` (`Value::Null` for pre-run triage).
fn routes(h: &RunHarness, run: &Value) -> Vec<Value> {
    h.history_lines("role_route")
        .into_iter()
        .filter(|l| &l["run_id"] == run)
        .collect()
}

/// Asserts `record` is a finished `role` record with its candidate snapshot: the
/// chosen route at `selected_index`, and no candidate labelled a failure.
fn check(record: &Value, role: &str, outcome: &str, result: &str) {
    assert_eq!(record["role"], role, "{record:#}");
    assert_eq!(record["outcome"], outcome, "{record:#}");
    let got = record["result"].as_str().unwrap_or_default();
    assert!(got.contains(result), "{role}: {got:?} lacks {result:?}");
    let candidates = record["candidates"].as_array().expect("candidates");
    assert!(!candidates.is_empty(), "{record:#}");
    let at = record["selected_index"].as_u64().unwrap() as usize;
    assert_eq!(candidates[at]["route"], record["chosen"], "{record:#}");
    assert!(candidates[at]["skipped_reason"].is_null(), "{record:#}");
    for c in candidates {
        let reason = c["skipped_reason"].as_str().unwrap_or_default();
        assert!(!reason.contains("fail"), "{role}: {c}");
    }
}

#[test]
fn e2e_role_history_for_large_and_triage_paths() {
    let h = harness_with(triage(&["code"], "large"));
    h.decider("triage", 2, triage_plan());
    h.script(
        "scout-s-1",
        &[call(
            "submit_scout_report",
            json!({"summary": "tests/ holds the tests.",
                "files": [{"path": "tests/t_ok.sh", "why": "a test"}]}),
        )],
    );
    // The run has a scout report, so the planners' tasks name it (rule 7.1), from the
    // context their `scout_refs` fill.
    for epic in ["a", "b"] {
        let task = plan_task(
            &format!("{epic}1"),
            &[&format!("src/{epic}/one.rs")],
            json!({"scout_refs": ["{{scout}}"]}),
        );
        planner(
            &h,
            epic,
            &[
                capture_json("scout", "/scouts/0/id"),
                submit_epic(vec![task]),
            ],
        );
    }
    let mut steps = vec![
        prompt(),
        call(
            "spawn_scout",
            json!({"id": "s", "question": "Where are the tests?", "area": ["tests/**"]}),
        ),
        until("/scouts/0/state", json!("reported"), ORCH_WAIT),
        // Each planner is given the report (a planner's context lists only its
        // `scout_refs` and the reports whose area meets its own).
        capture_json("scout", "/scouts/0/id"),
    ];
    for epic in ["a", "b"] {
        steps.push(call(
            "spawn_subplanner",
            json!({"epic": epic, "title": format!("Epic {epic}"),
                "area": [format!("src/{epic}/**")], "brief": format!("Plan epic {epic}."),
                "scout_refs": ["{{scout}}"]}),
        ));
    }
    steps.extend(until_planners_finished(2));
    steps.extend([
        edit_plan(vec![], json!({"submit": true})),
        expect("/awaiting_approval", json!(true)),
        marker(),
        read(None),
    ]);
    let (run, _) = start(&h, &steps);
    h.wait_run(&run, |r| r.state == RunState::AwaitingApproval, ORCH_WAIT);
    wait_passed(&h, 1);
    // The run ends; the orchestrator's session ends with it.
    ok(&h.anthrex(&["run", "reject", &run, "--confirm", &run]));
    h.wait_run(&run, |r| r.state == RunState::Discarded, REQUEST_WAIT);
    let lines = poll("the run's four records", REQUEST_WAIT, || {
        let lines = routes(&h, &json!(run));
        (lines.len() >= 4 && lines.iter().any(|l| l["role"] == "orchestrator")).then_some(lines)
    });
    assert_eq!(lines.len(), 4, "{lines:#?}");
    let of = |role: &str| -> Vec<&Value> { lines.iter().filter(|l| l["role"] == role).collect() };
    let orch = of("orchestrator");
    assert_eq!(orch.len(), 1, "{lines:#?}");
    check(
        orch[0],
        "orchestrator",
        "completed",
        "the run's plan had been submitted",
    );
    assert_eq!(orch[0]["trigger"], "start");
    let scout = of("scout");
    assert_eq!(scout.len(), 1, "{lines:#?}");
    check(scout[0], "scout", "completed", "reported");
    let planners = of("planner");
    assert_eq!(planners.len(), 2, "{lines:#?}");
    let mut epics: Vec<&str> = planners
        .iter()
        .map(|p| p["input"]["epic"].as_str().unwrap())
        .collect();
    epics.sort();
    assert_eq!(epics, ["a", "b"]);
    for p in &planners {
        check(p, "planner", "completed", "epic accepted");
        assert_eq!(p["input"]["run_path"], "large", "{p:#}");
    }
    let mut ids: Vec<&str> = lines
        .iter()
        .map(|l| l["record_id"].as_str().unwrap())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 4, "{ids:?}");
    assert!(
        ids.iter().all(|id| id.starts_with(&format!("{run}/"))),
        "{ids:?}"
    );

    // Pre-run triage: its own record, with no run.
    let triage_lines = routes(&h, &Value::Null);
    assert_eq!(triage_lines.len(), 1, "{triage_lines:#?}");
    check(&triage_lines[0], "decider", "completed", "answered");
    assert_eq!(triage_lines[0]["trigger"], "triage");
    assert!(
        triage_lines[0]["record_id"]
            .as_str()
            .unwrap()
            .starts_with("triage/")
    );

    // A goal whose run cannot be created still leaves its triage record. The start
    // refuses after triage (decision 9): a Codex orchestrator would load the project
    // configuration this repository now tracks, and `--trust-project` was not given.
    std::fs::create_dir_all(h.repo.join(".codex")).unwrap();
    std::fs::write(h.repo.join(".codex/config.toml"), "model = \"x\"\n").unwrap();
    git_in(&h.repo, &["add", ".codex/config.toml"]);
    git_in(&h.repo, &["commit", "-q", "-m", "track codex config"]);
    let runs_before = h.snapshot().runs.len();
    let out = h.start_goal("add the files", &["--orchestrator", "codex"]);
    assert!(!out.status.success(), "{}", stdout(&out));
    assert!(
        stderr(&out).contains(".codex/config.toml"),
        "{}",
        stderr(&out)
    );
    assert_eq!(h.snapshot().runs.len(), runs_before, "a run was created");
    let triage_lines = routes(&h, &Value::Null);
    assert_eq!(triage_lines.len(), 2, "{triage_lines:#?}");
    let second = &triage_lines[1];
    check(second, "decider", "completed", "answered");
    assert_ne!(second["record_id"], triage_lines[0]["record_id"]);
    assert_eq!(h.history_lines("role_route").len(), 6);
}
