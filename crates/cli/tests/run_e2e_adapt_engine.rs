//! Milestone 8b, task 18 (II): the deciders inside a run (the check summary and its
//! fallback, the blocked-reason classifier, the size cross-check), the output filter
//! and OTLP metering end to end, through a real daemon with `fake-agent` as both
//! runtimes and as the decider (`ANTHREX_DECIDER_BIN`). Every run here uses the brief's
//! stored profile. The goal scenarios are in `run_e2e_adapt.rs`.

mod support;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

use proto::{BlockReason, DeciderSource, RunInfo, Size, TaskState, TokenUsage};
use serde_json::{Value, json};
use support::run_adapt::{ADAPT_FILES, STORED_PROFILE};
use support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use support::run_plans::*;

/// A harness running deciders in `mode`, with the stored profile, the base files it
/// needs, and `files` over them.
fn harness(mode: &str, env: &[(&str, &str)], files: &[(&str, &str)]) -> RunHarness {
    let mut all: Vec<(&str, &str)> = ADAPT_FILES.to_vec();
    all.extend_from_slice(files);
    let h = RunHarness::adapt(mode, "", env, &all);
    h.stored_profile(STORED_PROFILE);
    h
}

/// A plan with no `[profile]`: the stored profile is the run's.
fn plan_of(tasks: &[String]) -> String {
    format!("goal = \"Adapt\"\n{}", tasks.concat())
}

/// `check.sh`: 200 numbered lines and exit 1 while a file `broken` exists.
const NOISY_CHECK: &str = "if [ -e broken ]; then i=1; while [ $i -le 200 ]; do echo \"raw line $i\"; i=$((i+1)); done; exit 1; fi\necho check ok\n";

/// The worker of the check-bounce scenarios: a first `task_done` with `broken`
/// committed, then (after the bounce, which must contain `expect`) the fix.
fn bouncing_worker(h: &RunHarness, expect: &str) {
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            commit("broken", "x\n"),
            done("added a"),
            read(expect),
            sh("git rm -q broken && git commit -qm 'remove broken'"),
            done("fixed the check"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
}

/// The worker's bounce: the one message starting `[anthrex] The check failed`.
fn check_bounce(h: &RunHarness) -> String {
    let texts = user_texts(&h.io_lines("worker-t1-1", "stdin"));
    let bounces: Vec<&String> = texts
        .iter()
        .filter(|t| t.starts_with("[anthrex] The check failed"))
        .collect();
    assert_eq!(bounces.len(), 1, "{texts:#?}");
    bounces[0].clone()
}

/// The first (failed) task check recorded in `run.json`.
fn failed_check(run: &RunInfo) -> Value {
    let json = run_json(run);
    let checks = json["tasks"][0]["checks"].as_array().unwrap().clone();
    let failed: Vec<&Value> = checks
        .iter()
        .filter(|c| c["on_candidate"] != true && c["ok"] == false)
        .collect();
    assert_eq!(failed.len(), 1, "{checks:#?}");
    failed[0].clone()
}

fn has_line(text: &str, line: &str) -> bool {
    text.lines().any(|l| l == line)
}

#[test]
fn e2e_check_bounce_carries_the_decider_summary() {
    let h = harness("claude", &[], &[("check.sh", NOISY_CHECK)]);
    let first = "the check failed because the file broken is present";
    h.decider(
        "check_summary",
        1,
        json!({"answer": {"lines": [first, "remove broken and commit"]}}),
    );
    bouncing_worker(&h, first);
    let id = h.start(&plan_of(&[task("t1", &["a.txt", "broken"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.bounces.check, 1, "{:?}", t1.bounces);

    let bounce = check_bounce(&h);
    assert!(
        bounce.contains(&format!(
            "Summary of its output:\n{first}\nremove broken and commit"
        )),
        "{bounce}"
    );
    for raw in ["raw line 150", "raw line 200"] {
        assert!(
            !has_line(&bounce, raw),
            "{raw} reached the worker:\n{bounce}"
        );
    }
    assert!(!bounce.contains("Last 40 lines:"), "{bounce}");
    let check = failed_check(&run);
    assert_eq!(check["summary_source"], "decider", "{check:#}");
    let kinds: Vec<Value> = h
        .decider_calls()
        .iter()
        .map(|c| c["kind"].clone())
        .collect();
    assert_eq!(kinds, vec![json!("check_summary")]);
    let usage = run.usage.expect("usage");
    assert_eq!((usage.decider_calls, usage.decider_fallbacks), (1, 0));
}

#[test]
fn e2e_check_bounce_falls_back_to_the_tail_when_the_decider_fails() {
    let h = harness("claude", &[], &[("check.sh", NOISY_CHECK)]);
    // No `check_summary` script: the decider exits 2 without answering.
    bouncing_worker(&h, "Last 40 lines:");
    let id = h.start(&plan_of(&[task("t1", &["a.txt", "broken"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);

    let bounce = check_bounce(&h);
    assert!(bounce.contains("Last 40 lines:\n"), "{bounce}");
    assert!(!bounce.contains("Summary of its output:"), "{bounce}");
    for kept in ["raw line 161", "raw line 200"] {
        assert!(has_line(&bounce, kept), "{kept} is missing:\n{bounce}");
    }
    assert!(!has_line(&bounce, "raw line 160"), "{bounce}");
    let check = failed_check(&run);
    assert_eq!(check["summary_source"], "fallback", "{check:#}");
    assert_eq!(h.decider_calls().len(), 1, "the decider was asked once");
    let usage = run.usage.expect("usage");
    assert_eq!((usage.decider_calls, usage.decider_fallbacks), (1, 1));
}

#[test]
fn e2e_unclassified_block_is_classified_as_environment() {
    let h = harness("claude", &[], &[]);
    h.decider(
        "blocked_reason",
        1,
        json!({"answer": {"kind": "environment", "reason": "a tool is missing"}}),
    );
    let reason = "cargo is not installed in this checkout, so nothing can be built";
    h.script(
        "worker-t1-1",
        &[json!({"mcp_call": {"tool": "task_blocked", "args": {"reason": reason}}})],
    );
    let id = h.start(&plan_of(&[task("t1", &["a.txt"], "")]), true);
    let run = h.wait_run(&id, |r| t(r, "t1").block_source.is_some(), RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Blocked);
    let block = t1.block.as_ref().expect("a block");
    assert_eq!(block.reason, BlockReason::Environment, "{block:?}");
    assert_eq!(t1.block_source, Some(DeciderSource::Decider));
    let calls = h.decider_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["kind"], "blocked_reason");
    assert!(
        calls[0]["prompt"].as_str().unwrap().contains(reason),
        "{:#}",
        calls[0]
    );
}

#[test]
fn e2e_size_cross_check_raises_a_task_and_reports_it() {
    let h = harness("claude", &[], &[]);
    h.stored_onboarding_report(json!({
        "summary": "Two modules, core and web, each checked by check.sh.",
        "files": [{"path": "check.sh", "why": "the check"}],
        "modules": ["core", "web"],
    }));
    let why = "it touches both modules";
    h.decider(
        "size_check",
        1,
        json!({"answer": {"tasks": [{"id": "t1", "size": "M", "reason": why}]}}),
    );
    for id in ["t1", "t2"] {
        let file = format!("{id}.txt");
        h.script(
            &format!("worker-{id}-1"),
            &[commit(&file, "x\n"), done("added it")],
        );
        h.script(&format!("reviewer-{id}-1"), &[approve()]);
    }
    let plan = plan_of(&[
        task("t1", &["t1.txt"], ""),
        task("t2", &["t2.txt"], "scout_refs = [\"nope\"]"),
    ]);
    let id = h.start(&plan, true);
    let run = h.wait_run(&id, complete, RUN_WAIT);

    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.size, Size::M);
    let check = t1.size_check.as_ref().expect("t1 was cross-checked");
    assert_eq!(check.engine, Size::S);
    assert_eq!(check.decided, Some(Size::M));
    assert!(!check.agreed);
    assert_eq!(check.source, DeciderSource::Decider);
    assert_eq!(check.reason, why);
    report_with(
        &run,
        &format!("Size cross-check: S by the engine, M by the decider: {why}"),
    );

    let t2 = t(&run, "t2");
    assert_eq!(t2.state, TaskState::Merged);
    assert_eq!(t2.size, Size::S);
    assert!(t2.size_check.is_none(), "{:?}", t2.size_check);
    assert!(
        t2.notes
            .iter()
            .any(|n| n == "size cross-check skipped: no scout evidence"),
        "{:?}",
        t2.notes
    );
    let calls = h.decider_calls();
    assert_eq!(calls.len(), 1, "{calls:#?}");
    assert_eq!(calls[0]["kind"], "size_check");
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(prompt.contains("Two modules, core and web"), "{prompt}");
}

/// `tests/noisy.sh`: 5000 lines, one of them `FAILED`, and exit 1.
const NOISY_TEST: &str = "i=1\nwhile [ $i -le 5000 ]; do\n  if [ $i -eq 2500 ]; then echo \"test_two FAILED at step $i\"; else echo \"step $i ok\"; fi\n  i=$((i+1))\ndone\nexit 1\n";

/// The log lines of the last `[anthrex] full output: <path> (<n> lines, exit <c>)`.
fn full_output_path(lines: &[Value]) -> PathBuf {
    let last = lines
        .last()
        .and_then(Value::as_str)
        .expect("an output line");
    let rest = last
        .strip_prefix("[anthrex] full output: ")
        .unwrap_or_else(|| panic!("not the log line: {last}"));
    let (path, counts) = rest.rsplit_once(" (").unwrap();
    assert_eq!(counts, "5000 lines, exit 1)", "{last}");
    PathBuf::from(path)
}

#[test]
fn e2e_filter_hook_shrinks_test_output_for_a_claude_worker() {
    // `/bin/sh` for filter-run, so no user shell startup file is read.
    let h = harness(
        "claude",
        &[("SHELL", "/bin/sh")],
        &[("tests/noisy.sh", NOISY_TEST)],
    );
    let seen = h.io.join("logs");
    let tmpdir = h.io.join("tmpdir");
    h.script(
        "worker-t1-1",
        &[
            json!({"bash": {"cmd": "sh tests/noisy.sh"}}),
            sh(&format!(
                "printf '%s' \"$TMPDIR\" > '{}'; for f in \"$TMPDIR\"/anthrex-logs/*.log; do printf '%s\\n' \"$f\" >> '{s}'; wc -l < \"$f\" | tr -d ' ' >> '{s}'; done",
                tmpdir.display(),
                s = seen.display()
            )),
            commit("a.txt", "a\n"),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan_of(&[task("t1", &["a.txt"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.done_signal, Some(proto::DoneSignal::TaskDone));
    assert_eq!(t1.bounces.check, 0, "{:?}", t1.bounces);

    let log = h.bash_log();
    assert_eq!(log.len(), 1, "{log:#?}");
    let entry = &log[0];
    assert_eq!(entry["original"], "sh tests/noisy.sh");
    let ran = entry["ran"].as_str().unwrap();
    assert!(
        ran.contains(" filter-run --mode failures-only --log-dir ")
            && ran.ends_with(" -c 'sh tests/noisy.sh'"),
        "{ran}\nworker argv: {:?}",
        h.io_lines("worker-t1-1", "args")
    );
    assert_eq!(entry["exit"], 1);
    let lines = entry["output_lines"].as_array().unwrap();
    assert!(lines.len() <= 32, "{} lines: {lines:#?}", lines.len());
    assert!(
        lines.iter().any(|l| l == "test_two FAILED at step 2500"),
        "{lines:#?}"
    );

    // The full log: every line, under the task's own TMPDIR, never in the checkout.
    let path = full_output_path(lines);
    let seen = std::fs::read_to_string(&seen).unwrap();
    let seen: Vec<&str> = seen.lines().collect();
    assert_eq!(seen, [path.display().to_string().as_str(), "5000"]);
    let tmpdir = PathBuf::from(std::fs::read_to_string(&tmpdir).unwrap());
    let run_dir = run.report_path.parent().unwrap();
    let task_tmp = daemon::run::role_launch::task_tmp_dir(run_dir, "t1");
    assert_eq!(tmpdir, task_tmp);
    assert_eq!(path.parent(), Some(task_tmp.join("anthrex-logs").as_path()));
    assert!(!path.starts_with(&t1.worktree), "{}", path.display());
    assert!(!path.starts_with(&h.repo), "{}", path.display());
}

const ORCHESTRATOR_FIXTURE: &str =
    include_str!("../../daemon/tests/fixtures/otlp/claude-2.1.280-metrics.json");

/// M8b.1 item 5's recorded totals (one delta export).
fn fixture_usage(times: u64) -> TokenUsage {
    TokenUsage {
        input: 10 * times,
        output: 170 * times,
        cache_read: 13_689 * times,
        cache_write: 12_503 * times,
    }
}

/// `POST /v1/metrics` to `addr` (`http://127.0.0.1:<port>`), with a `Content-Length`
/// or a chunked body; the status line.
fn post(addr: &str, body: &[u8], chunked: bool) -> String {
    let host = addr.strip_prefix("http://").expect("an http address");
    let mut stream = TcpStream::connect(host).unwrap();
    stream.set_read_timeout(Some(REQUEST_WAIT)).unwrap();
    let mut request = format!(
        "POST /v1/metrics HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nConnection: close\r\n"
    )
    .into_bytes();
    if chunked {
        request.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");
        for chunk in body.chunks(1000) {
            request.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
            request.extend_from_slice(chunk);
            request.extend_from_slice(b"\r\n");
        }
        request.extend_from_slice(b"0\r\n\r\n");
    } else {
        request.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        request.extend_from_slice(body);
    }
    stream.write_all(&request).unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer.lines().next().unwrap_or_default().to_string()
}

fn orchestrator_usage(run: &RunInfo) -> Option<TokenUsage> {
    run.usage.as_ref()?.by_role.get("orchestrator").copied()
}

/// `<data>/otlp.addr`, once the receiver has written it.
fn otlp_addr(data: &Path) -> String {
    until("otlp.addr", REQUEST_WAIT, || {
        std::fs::read_to_string(data.join("otlp.addr"))
            .ok()
            .filter(|a| !a.trim().is_empty())
    })
    .trim()
    .to_string()
}

#[test]
fn e2e_otlp_usage_reaches_the_run_snapshot() {
    let h = harness("off", &[], &[]);
    // A run held at its plan gate: live, so the receiver meters it.
    let id = h.start(&plan_of(&[task("t1", &["a.txt"], "")]), false);
    assert_eq!(
        orchestrator_usage(&h.run(&id).unwrap()),
        Some(TokenUsage::default())
    );
    let body = ORCHESTRATOR_FIXTURE.replace("\"r-fix\"", &format!("{id:?}"));
    assert!(!body.contains("r-fix"));
    let addr = otlp_addr(&h.data());

    assert_eq!(post(&addr, body.as_bytes(), false), "HTTP/1.1 200 OK");
    h.wait_run(
        &id,
        |r| orchestrator_usage(r) == Some(fixture_usage(1)),
        REQUEST_WAIT,
    );
    // A second, chunked export of the same deltas adds them again.
    assert_eq!(post(&addr, body.as_bytes(), true), "HTTP/1.1 200 OK");
    let run = h.wait_run(
        &id,
        |r| orchestrator_usage(r) == Some(fixture_usage(2)),
        REQUEST_WAIT,
    );
    let usage = run.usage.unwrap();
    assert_eq!(usage.total, fixture_usage(2), "no other role has spent");
}
