//! `--design off` (decision 3, DF §1): a planned goal started with the flow off, where
//! the configured default is `full`, behaves as 9.5 does: the same states and the same
//! requests to its agents as the same goal on a harness whose default is off (every
//! pre-9.6 scenario's harness, ruling T3-2), and no design field anywhere.

use proto::{DaemonMsg, RunReply, RunState};
use serde_json::{Value, json};

use crate::common::{GOAL, ORCH, green, ok};
use crate::support::orch_script::*;
use crate::support::run_harness::{RUN_WAIT, RunHarness};
use crate::support::run_orch::ORCH_WAIT;

/// What one run of the planned goal showed: the states its snapshots went through,
/// and every request made of its agents (each session's argv and every message its
/// terminal or stdin got, every tool call the orchestrator made, and the triage
/// decider's request), normalised.
#[derive(Debug, PartialEq)]
struct Seen {
    states: Vec<RunState>,
    requests: Vec<(String, Vec<String>)>,
    status_keys: Vec<String>,
    /// `run.json`'s keys, and its `orch` table's.
    saved_keys: (Vec<String>, Vec<String>),
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort_unstable();
    keys
}

/// The run's ids, the harness's temp dir, the folders named from its paths' hashes,
/// its git objects (named from their times) and each session's random id, made the
/// same for both harnesses.
fn normalise(text: &str, h: &RunHarness, run: &str) -> String {
    let dir = h.dir.path().display().to_string();
    let canonical = h.dir.path().canonicalize().unwrap().display().to_string();
    let mut text = text.to_string();
    // The repository's worktrees folder is named from a hash of its path.
    let worktrees = std::fs::read_dir(h.data().join("worktrees"))
        .into_iter()
        .flatten();
    for entry in worktrees.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        text = text.replace(&format!("/worktrees/{name}/"), "/worktrees/<repo>/");
    }
    // Commits are named from their times: every object the repository holds.
    let objects = h.git(&["rev-list", "--all", "--objects"]);
    for sha in objects.lines().filter_map(|l| l.split(' ').next()) {
        text = text.replace(sha, "<sha>").replace(&sha[..7], "<sha>");
    }
    let text = text
        .replace(&canonical, "<tmp>")
        .replace(&dir, "<tmp>")
        .replace(run, "<run>")
        .replace(&run[run.len() - 4..], "<h4>");
    hashed_dirs(&uuids(&text))
}

/// Every path component of exactly 16 hex digits (a checkout's hashed folder under
/// `/tmp/ax-<uid>/`) as `<hash>`: a `/`, the digits, then no further hex digit.
fn hashed_dirs(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut i) = (String::new(), 0);
    while i < text.len() {
        let hashed = bytes[i] == b'/'
            && bytes
                .get(i + 1..i + 17)
                .is_some_and(|d| d.iter().all(u8::is_ascii_hexdigit))
            && !bytes.get(i + 17).is_some_and(u8::is_ascii_hexdigit);
        if hashed {
            out.push_str("/<hash>");
            i += 17;
            continue;
        }
        let c = text[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// Every UUID (`8-4-4-4-12` hex digits) in `text` as `<uuid>`.
fn uuids(text: &str) -> String {
    const SHAPE: [usize; 5] = [8, 4, 4, 4, 12];
    let bytes = text.as_bytes();
    let at = |i: usize| -> Option<usize> {
        let mut j = i;
        for (k, n) in SHAPE.iter().enumerate() {
            let end = j + n;
            let hex = bytes.get(j..end)?.iter().all(u8::is_ascii_hexdigit);
            if !hex {
                return None;
            }
            j = end;
            if k < 4 {
                (bytes.get(j) == Some(&b'-')).then_some(())?;
                j += 1;
            }
        }
        Some(j)
    };
    let (mut out, mut i) = (String::new(), 0);
    while i < text.len() {
        match at(i) {
            Some(end) => {
                out.push_str("<uuid>");
                i = end;
            }
            None => {
                let c = text[i..].chars().next().unwrap();
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}

/// Runs the planned goal on `h` with `flags`: the orchestrator plans `t1`, the user
/// approves the plan, the task merges, the orchestrator writes its summary.
fn planned(h: &RunHarness, flags: &[&str]) -> Seen {
    green(h, "t1", "a.txt");
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], json!({})))],
            json!({"submit": true}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        until("/run/complete", json!(true), RUN_WAIT),
        edit_plan(vec![], json!({"summary": "a.txt was added."})),
        marker(),
        read(None),
    ];
    h.script(ORCH, &steps);
    let watcher = h.subscribe();
    let run = h.start_goal_id(GOAL, flags);
    h.wait_run(&run, |r| r.state == RunState::AwaitingApproval, ORCH_WAIT);
    ok(h, &["run", "approve", &run]);
    h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);
    crate::common::wait_passed(h, 1);

    let mut states: Vec<RunState> = Vec::new();
    for message in watcher.received() {
        let DaemonMsg::Run(RunReply::Snapshot(snapshot)) = message else {
            continue;
        };
        let state = snapshot
            .runs
            .iter()
            .find(|r| r.run_id == run)
            .map(|r| r.state);
        if let Some(state) = state.filter(|s| states.last() != Some(s)) {
            states.push(state);
        }
    }
    let mut requests = Vec::new();
    for name in [ORCH, "worker-t1-1", "reviewer-t1-1"] {
        for ext in ["args", "stdin"] {
            let lines = h.io_lines(name, ext);
            let lines = lines.iter().map(|l| normalise(l, h, &run)).collect();
            requests.push((format!("{name}.{ext}"), timeless(lines)));
        }
    }
    // A poll's repeats depend on timing: consecutive equal calls count once.
    let mut calls: Vec<String> = (h.mcp_log().iter())
        .filter(|l| l["script"] == ORCH)
        .map(|l| normalise(&format!("{} {} {}", l["tool"], l["args"], l["ok"]), h, &run))
        .collect();
    calls.dedup();
    requests.push(("orchestrator tool calls".into(), calls));
    // Final fix wave FW-64: the triage decider's request (its kind, argv and prompt).
    let deciders = (h.decider_calls().iter())
        .map(|c| normalise(&c.to_string(), h, &run))
        .collect();
    requests.push(("decider calls".into(), timeless(deciders)));
    let saved = h.run_json(&run);
    Seen {
        states,
        requests,
        status_keys: keys(&h.status_json(&run)),
        saved_keys: (keys(&saved), keys(&saved["orch"])),
    }
}

/// Each JSON line without its timestamps (`at`, `first_at`), which differ by run.
fn timeless(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|l| match serde_json::from_str::<Value>(&l) {
            Ok(Value::Object(mut map)) => {
                map.remove("at");
                map.remove("first_at");
                Value::Object(map).to_string()
            }
            _ => l,
        })
        .collect()
}

#[test]
fn e2e_design_off_is_9_5() {
    // 9.5's planned goal: a harness whose design default is off (ruling T3-2).
    let before = RunHarness::orch("", &[]);
    let nine_five = planned(&before, &[]);
    // The same goal with `--design off` where the default is `full`.
    let after = RunHarness::design("", &[]);
    let off = planned(&after, &["--design", "off"]);

    assert_eq!(
        nine_five.states,
        [
            RunState::Planning,
            RunState::AwaitingApproval,
            RunState::Running,
            RunState::Complete
        ]
    );
    assert_eq!(off.states, nine_five.states);
    for ((name, a), (_, b)) in nine_five.requests.iter().zip(&off.requests) {
        assert_eq!(a, b, "{name}");
    }
    assert_eq!(off.requests.len(), nine_five.requests.len());
    let (name, deciders) = nine_five.requests.last().unwrap();
    assert_eq!(name, "decider calls");
    assert_eq!(
        deciders.len(),
        1,
        "the triage decider's one call: {deciders:?}"
    );
    assert_eq!(off.status_keys, nine_five.status_keys);
    for key in ["design", "doc_gate", "docs", "design_agents"] {
        assert!(
            !off.status_keys.iter().any(|k| k == key),
            "{key}: {:?}",
            off.status_keys
        );
    }
    assert_eq!(off.saved_keys, nine_five.saved_keys);
    assert!(
        !off.saved_keys.1.iter().any(|k| k == "design"),
        "{:?}",
        off.saved_keys
    );
}
