//! The run title change end to end: `anthrex run start --goal` through a real daemon,
//! with `fake-agent` as both runtimes and as the decider (`ANTHREX_DECIDER_BIN`, scripted
//! as `run_name-<n>.json`; no test reaches a model). A scripted name gives the run its
//! title and a slug-based id; with no answer the run keeps today's id from its goal and
//! no title.

mod support;

use std::process::Output;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::run_adapt::{ADAPT_FILES, STORED_PROFILE, triage_single};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

/// How long the run-name record, written off the start's path, may take to appear.
const RECORD_WAIT: Duration = Duration::from_secs(10);

fn harness() -> RunHarness {
    let h = RunHarness::adapt("claude", "", &[], ADAPT_FILES);
    h.stored_profile(STORED_PROFILE);
    h
}

fn started(h: &RunHarness, out: &Output) -> String {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        h.log_tail()
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The repository's `run_name` records, once `n` are there.
fn run_name_records(h: &RunHarness, n: usize) -> Vec<Value> {
    let deadline = Instant::now() + RECORD_WAIT;
    loop {
        let records: Vec<Value> = h
            .history_lines("role_route")
            .into_iter()
            .filter(|l| l["trigger"] == "run_name")
            .collect();
        if records.len() >= n || Instant::now() >= deadline {
            return records;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_goal_gets_the_scripted_title_and_a_slug_based_id() {
    let h = harness();
    h.decider("triage", 1, triage_single(&["a.txt"]));
    h.decider(
        "run_name",
        1,
        json!({"answer": {"title": "Create file a", "slug": "create-file-a"}}),
    );
    green_scripts(&h.repo);
    let goal = "in anthrex, at the bottom of the repository I want a file called a";
    let id = started(&h, &h.start_goal(goal, &[]));

    let (head, suffix) = id.rsplit_once('-').unwrap();
    assert_eq!(head, "create-file-a", "{id}");
    assert_eq!(suffix.len(), 4, "{id}");
    assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()), "{id}");
    let run = h.run(&id).expect("the run is listed");
    assert_eq!(run.title, "Create file a");
    assert_eq!(run.goal, goal);

    // The decider was asked once, with the goal; triage's calls are its own.
    let calls = h.run_name_calls();
    assert_eq!(calls.len(), 1, "{calls:#?}");
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(
        prompt.starts_with("[anthrex decider] run_name v1\n"),
        "{prompt}"
    );
    assert!(prompt.ends_with(goal), "{prompt}");
    assert!(h.decider_calls().iter().all(|c| c["kind"] != "run_name"));

    // Recorded as pre-run triage is: a finished decider record with no run.
    let records = run_name_records(&h, 1);
    assert_eq!(records.len(), 1, "{records:#?}");
    let record = &records[0];
    assert!(
        record["record_id"]
            .as_str()
            .unwrap()
            .starts_with("run_name/"),
        "{record:#}"
    );
    assert_eq!(record["run_id"], Value::Null);
    assert_eq!(record["role"], "decider");
    assert_eq!(record["outcome"], "completed");

    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(run.title, "Create file a", "the title persists");
}

#[test]
fn without_an_answer_the_id_comes_from_the_goal_and_there_is_no_title() {
    let h = harness();
    h.decider("triage", 1, triage_single(&["a.txt"]));
    // No `run_name` script: `fake-agent` exits 2 and the decider falls back.
    green_scripts(&h.repo);
    let id = started(&h, &h.start_goal("add a", &[]));
    let (head, suffix) = id.rsplit_once('-').unwrap();
    assert_eq!(head, "add-a", "{id}");
    assert_eq!(suffix.len(), 4, "{id}");
    let run = h.run(&id).expect("the run is listed");
    assert_eq!(run.title, "");
    assert_eq!(h.run_name_calls().len(), 1);

    let records = run_name_records(&h, 1);
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_eq!(records[0]["outcome"], "fallback");
    h.wait_run(&id, complete, RUN_WAIT);
}
