//! Milestone 9.3 task M9.3.11: next goals on one orchestrator session end to end (KG
//! §3), through the real binary and a real daemon on temporary paths, with `fake-agent`
//! as every agent; no real agent and no `gh`. `run start --goal … --continue` after an
//! accepted two-round run adopts the same orchestrator window and session; an idle
//! orchestrator may only read and start a goal, and the goal it starts stops at the
//! gate even though its first run had `--yes`.

mod support;

use proto::{RunInfo, RunState};
use serde_json::{Value, json};
use support::orch_script::*;
use support::run_harness::{FINISH_WAIT, REQUEST_WAIT, RUN_WAIT, RunHarness};
use support::run_pr::log_lines;
use support::run_rounds::*;

/// The run id `start_goal`'s answer names (`run <id> started`).
fn started(log: &[Value]) -> Option<String> {
    let call = log
        .iter()
        .find(|l| l["script"] == ORCH && l["tool"] == "start_goal" && l["ok"] == true)?;
    let reply: Value = serde_json::from_str(call["result"].as_str()?).ok()?;
    let text = reply["message"].as_str()?;
    Some(
        text.strip_prefix("run ")?
            .strip_suffix(" started")?
            .to_string(),
    )
}

#[test]
fn e2e_next_goal_continues_the_same_orchestrator_session() {
    let h = RunHarness::orch("", &[]);
    let mut steps = two_rounds();
    steps.extend([
        read(Some("a new goal, run ")),
        call("run_status", json!({})),
        marker(),
        read(None),
    ]);
    let (first, window) = h.two_rounds_accepted(&steps);
    // The script's poll saw the accept, so its next step is the read the wake answers.
    h.wait_saw(ORCH, "/run/state", json!("accepted"), REQUEST_WAIT);
    let main = h.git(&["rev-parse", "main"]);

    let next = h.continue_goal("next", &first);
    assert_ne!(next, first);

    // The same window, renamed for the new run; the new run starts from the accepted
    // base.
    assert_eq!(h.orchestrator_window(&next), window);
    let name = |w: &proto::WindowInfo| w.name == format!("{}/orchestrator", h4(&next));
    h.wait_window(window, "renamed", name, CONTINUE_WAIT);
    let info = h.run(&next).unwrap();
    assert_eq!(info.base_sha, main);
    assert_eq!(
        log_lines(&h, &next).first().map(String::as_str),
        Some(format!("based on main at {}", &main[..7]).as_str())
    );
    let chain = format!("o-{}", h4(&first));
    for id in [&first, &next] {
        assert_eq!(
            h.run(id).unwrap().chain.as_deref(),
            Some(chain.as_str()),
            "{id}"
        );
    }

    // One session: one process, started with the first goal's prompt, which read the
    // round wake and then the next-goal wake, in order; its later read describes the
    // new run.
    let log = h.wait_log(
        "the orchestrator to read the next-goal wake",
        |log| passed(log, ORCH) >= 1,
        CONTINUE_WAIT,
    );
    let argv = h.io_lines(ORCH, "args");
    assert_eq!(argv.len(), 1, "one orchestrator process: {argv:#?}");
    assert!(argv[0].contains("add a"), "{}", argv[0]);
    let texts = texts(&h);
    let heads: Vec<&str> = texts.iter().map(|t| t.lines().next().unwrap()).collect();
    assert_eq!(
        heads,
        [
            round_wake_head(&first, 2, 1),
            next_goal_head(&next, &first, "accepted")
        ]
    );
    assert_eq!(texts[1], format!("{}\n{}", heads[1], fenced("next")));
    let read = log
        .iter()
        .rfind(|l| l["script"] == ORCH && l["tool"] == "run_status" && l["args"] == json!({}))
        .unwrap();
    let digest: Value = serde_json::from_str(read["result"].as_str().unwrap()).unwrap();
    assert_eq!(digest["run"]["id"], json!(next), "{digest}");
}

#[test]
fn e2e_an_idle_orchestrator_may_only_read_and_start_a_goal() {
    let h = RunHarness::orch("", &[]);
    green(&h, "t1", "a.txt");
    let idle = "has ended; start a new goal with start_goal when the user gives you one";
    let steps = [
        prompt(),
        edit_plan(vec![staged("t1", "a.txt", 1)], json!({"submit": true})),
        until("/run/complete", json!(true), RUN_WAIT),
        edit_plan(vec![], json!({"summary": SUMMARY_1})),
        until("/run/state", json!("accepted"), FINISH_WAIT),
        // Reads still answer; anything else gets decision 21's text.
        call("run_status", json!({})),
        call("get_context", json!({})),
        call_err(
            "edit_plan",
            json!({"edits": [staged("t2", "b.txt", 1)], "submit": true}),
        ),
        expect_error(idle),
        call_err(
            "spawn_scout",
            json!({"id": "core", "question": "What is here?", "area": ["a.txt"]}),
        ),
        expect_error(idle),
        call("start_goal", json!({"goal": "add b"})),
        read(Some("a new goal, run ")),
        edit_plan(vec![staged("t2", "b.txt", 1)], json!({"submit": true})),
        marker(),
        read(None),
    ];
    h.script(ORCH, &steps);
    let first = h.start_goal_id("add a", &["--yes"]);
    h.wait_summary(&first, SUMMARY_1, SUMMARY_WAIT);
    h.accept(&first);

    // The wait ends early when a call the script expects refused with the idle text
    // was answered otherwise: the script exits there and never calls `start_goal`.
    let otherwise = |log: &[Value]| {
        log.iter().any(|l| {
            let refused = l["result"].as_str().unwrap_or_default().contains(idle);
            let expected =
                l["tool"] == "spawn_scout" || l["args"]["edits"][0]["task"]["id"] == "t2";
            l["script"] == ORCH && (l["ok"] == false && !refused || l["ok"] == true && expected)
        })
    };
    let log = h.wait_log(
        "start_goal",
        |log| started(log).is_some() || otherwise(log),
        CONTINUE_WAIT.saturating_add(REQUEST_WAIT),
    );
    let refusal = format!("run {} {idle}", h4(&first));
    let refused: Vec<&Value> = (log.iter())
        .filter(|l| l["script"] == ORCH && l["ok"] == false)
        .collect();
    let tools: Vec<&Value> = refused.iter().map(|l| &l["tool"]).collect();
    assert_eq!(tools, [&json!("edit_plan"), &json!("spawn_scout")]);
    for l in refused {
        let reply: Value = serde_json::from_str(l["result"].as_str().unwrap()).unwrap();
        assert_eq!(reply, json!({"error": refusal}), "{l}");
    }
    let next = started(&log).unwrap();

    // The goal it started stops at the plan gate for the user, though the first run
    // had --yes; its wake said so.
    let gate = |r: &RunInfo| r.state == RunState::AwaitingApproval;
    h.wait_log(
        "the plan's submit",
        |log| passed(log, ORCH) >= 1,
        CONTINUE_WAIT,
    );
    let info = h.wait_run(&next, gate, REQUEST_WAIT);
    assert_eq!(info.approved_by, None);
    assert_eq!(
        info.chain.as_deref(),
        Some(format!("o-{}", h4(&first)).as_str())
    );
    let texts = texts(&h);
    assert_eq!(texts.len(), 1, "{texts:#?}");
    assert_eq!(
        texts[0],
        format!(
            "{}\n{}",
            next_goal_head(&next, &first, "accepted"),
            fenced("add b")
        )
    );
}
