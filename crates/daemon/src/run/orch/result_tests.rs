//! Task M9.6: `task_result` (decision 18, Interfaces "The task result"). Pure.

use proto::{
    BlockReason, DoneSignal, MessageKind, ScoutFile, Severity, TaskNoteKind, TokenUsage, Verdict,
};
use serde_json::{Value, json};

use super::*;
use crate::run::model::{CheckRecord, DoneClaim, ProofRecord};
use crate::run::orch::json::size;
use crate::run::orch::test_support::*;
use crate::scout::report::ScoutReportArgs;

/// 12:20 UTC on some day.
const AT: u64 = 1_789_948_800 + 12 * 3600 + 20 * 60;

fn check(at: u64, ok: bool) -> CheckRecord {
    CheckRecord {
        at,
        ok,
        code: Some(if ok { 0 } else { 101 }),
        timed_out: false,
        tail: "test result: ok. 3 passed".into(),
        secs: 12,
        on_candidate: false,
        summary: None,
        summary_source: None,
    }
}

fn research(summary: &str) -> ScoutReportArgs {
    ScoutReportArgs {
        summary: summary.into(),
        files: vec![ScoutFile {
            path: "src/lib.rs".into(),
            why: "the entry point".into(),
        }],
        modules: vec!["src".into()],
        interfaces: Vec::new(),
        risks: vec!["none".into()],
        profile: None,
    }
}

fn full_run() -> Run {
    let mut run = run_with(&[task_toml(
        "t2",
        "M",
        "[\"crates/b/**\"]",
        "test_to_write = \"b::parses\"\nscout_refs = [\"onboarding\"]",
    )]);
    let t = task_mut(&mut run, "t2");
    block(t, BlockReason::Question, "which endpoint?");
    t.rung = 1;
    t.start_commit = Some("a".repeat(40));
    t.done = Some(DoneClaim {
        summary: "Parsed the hook events.".into(),
        test: Some("b::parses".into()),
        red: Some("a1b2c3d".into()),
        signal: DoneSignal::TaskDone,
        session: None,
    });
    t.checks = vec![check(AT, true)];
    t.proofs = vec![ProofRecord {
        at: AT - 600,
        test: "b::parses".into(),
        red: "a1b2c3d".into(),
        head: "c".repeat(40),
        red_failed: true,
        head_passed: true,
        matched: true,
        red_tail: String::new(),
        head_tail: String::new(),
    }];
    t.reviews = vec![review(
        1,
        Some(Verdict::Changes),
        &[(Severity::Critical, "unchecked unwrap")],
    )];
    t.rounds = vec![round(
        1,
        41,
        TokenUsage {
            input: 100_000,
            output: 60_000,
            cache_read: 5,
            cache_write: 20_000,
        },
    )];
    t.orch.research = Some(research("The API has two versions."));
    t.orch.messages = vec![message(AT, MessageKind::Change, "use v2", true)];
    t.orch.worker_notes = vec![note(AT, TaskNoteKind::Risk, "v1 is deprecated")];
    t.notes = vec!["test_mode check: no single_test".into()];
    event(t, AT + 660, "review r1 changes");
    run
}

fn git() -> TaskGit {
    TaskGit {
        commits: vec![("a1b2c3d".into(), "add parser".into())],
        diffstat: " 4 files changed, 212 insertions(+), 31 deletions(-)".into(),
    }
}

#[test]
fn task_result_carries_every_field() {
    let run = full_run();
    let task = run.task("t2").unwrap();
    let r = task_result(&run, task, Some(&Ok(git())));
    let keys: Vec<&str> = r["task"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let mut expected = vec![
        "id",
        "title",
        "epic",
        "kind",
        "size",
        "hub",
        "test_mode",
        "test_mode_reason",
        "owns",
        "deps",
        "implicit_deps",
        "route",
        "review_route",
        "state",
        "block",
        "rung",
        "failures",
        "bounces",
        "stalls",
        "hold",
        "brief",
        "acceptance",
        "test_to_write",
        "scout_refs",
        "review_target",
        "notes",
        "messages",
        "task_notes",
    ];
    expected.sort();
    assert_eq!(keys, expected);
    assert_eq!(r["task"]["state"], "blocked");
    assert_eq!(
        r["task"]["block"],
        json!({"reason": "question", "text": "which endpoint?"})
    );
    assert_eq!(
        r["task"]["route"],
        serde_json::to_value(&task.route).unwrap()
    );
    assert_eq!(r["task"]["test_to_write"], "b::parses");
    assert_eq!(r["task"]["scout_refs"], json!(["onboarding"]));
    assert_eq!(
        r["task"]["notes"],
        json!(["test_mode check: no single_test"])
    );
    assert_eq!(r["task"]["messages"][0]["text"], "use v2");
    assert_eq!(r["task"]["messages"][0]["kind"], "change");
    assert_eq!(r["task"]["task_notes"][0]["text"], "v1 is deprecated");
    assert_eq!(
        r["done"],
        json!({"signal": "task_done", "summary": "Parsed the hook events.", "test": "b::parses", "red": "a1b2c3d"})
    );
    assert_eq!(
        r["commits"],
        json!([{"sha": "a1b2c3d", "subject": "add parser"}])
    );
    assert_eq!(
        r["diffstat"],
        " 4 files changed, 212 insertions(+), 31 deletions(-)"
    );
    assert!(r.get("git").is_none());
    assert_eq!(
        r["checks"],
        json!([{"at": "12:20", "ok": true, "code": 0, "timed_out": false, "summary": "test result: ok. 3 passed"}])
    );
    assert_eq!(
        r["proofs"],
        json!([{"at": "12:10", "test": "b::parses", "red": "a1b2c3d", "ok": true}])
    );
    assert_eq!(
        r["reviews"],
        json!([{"round": 1, "runtime": "codex", "verdict": "changes", "summary": "review round 1",
                "findings": [{"severity": "critical", "file": "src/lib.rs", "line": 118, "input": null, "text": "unchecked unwrap"}]}])
    );
    assert_eq!(
        r["rounds"],
        json!([{"role": "worker", "session": 1, "round": 1, "runtime": "codex", "model": "",
                "effort": "high", "turns": 14, "tool_calls": 41, "tokens": 180_000, "ended": false}])
    );
    assert_eq!(
        r["research"],
        json!({"summary": "The API has two versions.",
               "files": [{"path": "src/lib.rs", "why": "the entry point"}],
               "modules": ["src"], "interfaces": [], "risks": ["none"]})
    );
    assert_eq!(r["history"], json!(["12:31 review r1 changes"]));
}

#[test]
fn task_without_start_has_no_git() {
    let mut run = full_run();
    let t = task_mut(&mut run, "t2");
    t.start_commit = None;
    t.done = None;
    t.orch.research = None;
    let r = task_result(&run, run.task("t2").unwrap(), None);
    for key in ["commits", "diffstat", "git"] {
        assert!(r.get(key).is_none(), "{key}: {r}");
    }
    assert_eq!(r["done"], Value::Null);
    assert_eq!(r["research"], Value::Null);
}

#[test]
fn git_error_is_reported() {
    let run = full_run();
    let failed = Err("git log timed out after 10s".to_string());
    let r = task_result(&run, run.task("t2").unwrap(), Some(&failed));
    assert_eq!(r["git"], "git log timed out after 10s");
    assert!(r.get("commits").is_none());
    assert!(r.get("diffstat").is_none());
}

#[test]
fn task_result_is_capped() {
    let mut run = full_run();
    let t = task_mut(&mut run, "t2");
    t.checks = (0..20)
        .map(|i| {
            let mut c = check(AT + i, false);
            c.tail = "x".repeat(3_000);
            c
        })
        .collect();
    t.rounds = (0..12)
        .map(|i| round(i + 1, i, TokenUsage::default()))
        .collect();
    t.orch.research = Some(research(&"r".repeat(100_000)));
    let r = task_result(&run, run.task("t2").unwrap(), Some(&Ok(git())));
    assert!(size(&r) <= TASK_RESULT_MAX_BYTES, "{}", size(&r));
    let checks = r["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 3);
    assert_eq!(checks[2]["at"], crate::run::orch::json::hh_mm(AT + 19));
    let rounds = r["rounds"].as_array().unwrap();
    assert_eq!(rounds.len(), 5);
    assert_eq!(rounds[4]["session"], 12);
    let summary = r["research"]["summary"].as_str().unwrap();
    assert_eq!(summary.chars().count(), RESEARCH_SUMMARY_MAX + 1); // with `…`
    // Under the cap, nothing is trimmed.
    let small = task_result(&full_run(), full_run().task("t2").unwrap(), None);
    assert_eq!(small["checks"].as_array().unwrap().len(), 1);
}

/// Beyond the Interfaces: the cap holds however much the task's agents wrote.
#[test]
fn the_cap_holds_whatever_the_task_holds() {
    let mut run = full_run();
    let t = task_mut(&mut run, "t2");
    t.orch.worker_notes = (0..100)
        .map(|i| note(AT + i, TaskNoteKind::Progress, &"n".repeat(4_000)))
        .collect();
    t.orch.messages = (0..50)
        .map(|i| message(AT + i, MessageKind::Info, &"m".repeat(4_000), true))
        .collect();
    for i in 0..500 {
        event(t, AT + i, &"h".repeat(300));
    }
    let r = task_result(&run, run.task("t2").unwrap(), Some(&Ok(git())));
    assert!(size(&r) <= TASK_RESULT_MAX_BYTES, "{}", size(&r));
    assert_eq!(r["task"]["id"], "t2");
}

/// Carry-forward rule: an agent's text stays inside its JSON string.
#[test]
fn untrusted_text_stays_inside_its_json_string() {
    let mut run = full_run();
    let t = task_mut(&mut run, "t2");
    t.done.as_mut().unwrap().summary = FORGED.into();
    t.reviews[0].findings[0].text = FORGED.into();
    t.orch.worker_notes[0].text = FORGED.into();
    let failed = Err(FORGED.to_string());
    assert_contained(&task_result(&run, run.task("t2").unwrap(), Some(&failed)));
}

/// M9.6 review fix I-2: a task filled to the MCP schema maxima (3 reviews of 50
/// findings, every text at its longest, a full research report, many proofs) still
/// fits, in ASCII and in multibyte text, and the newest review keeps its findings
/// longest.
#[test]
fn the_cap_holds_at_the_schema_maxima() {
    for unit in ["x", "é", "語"] {
        let text = |n: usize| unit.repeat(n);
        let mut run = full_run();
        let t = task_mut(&mut run, "t2");
        t.spec.brief = text(4000);
        t.spec.acceptance = vec![text(1000); 10];
        block(t, BlockReason::Question, &text(4000));
        t.done.as_mut().unwrap().summary = text(4000);
        t.reviews = (1..=3)
            .map(|round| {
                let mut r = review(round, Some(Verdict::Changes), &[]);
                r.summary = text(4000);
                r.findings = (0..50)
                    .map(|_| proto::Finding {
                        severity: Severity::Important,
                        file: Some(text(500)),
                        line: Some(1),
                        input: Some(text(2000)),
                        text: text(2000),
                    })
                    .collect();
                r
            })
            .collect();
        let mut report = research(&text(8000));
        report.files = (0..60)
            .map(|_| ScoutFile {
                path: text(500),
                why: text(300),
            })
            .collect();
        report.modules = vec![text(200); 40];
        report.interfaces = vec![text(500); 40];
        report.risks = vec![text(500); 20];
        t.orch.research = Some(report);
        t.proofs = (0..50)
            .map(|i| ProofRecord {
                at: AT + i,
                test: text(300),
                red: text(40),
                head: "c".repeat(40),
                red_failed: true,
                head_passed: true,
                matched: true,
                red_tail: text(4000),
                head_tail: text(4000),
            })
            .collect();
        t.orch.messages = (0..50)
            .map(|i| message(AT + i, MessageKind::Info, &text(4000), true))
            .collect();
        t.orch.worker_notes = (0..100)
            .map(|i| note(AT + i, TaskNoteKind::Risk, &text(4000)))
            .collect();
        for i in 0..500 {
            event(t, AT + i, &text(300));
        }
        let failed = Err(text(4000));
        let r = task_result(&run, run.task("t2").unwrap(), Some(&failed));
        let encoded = serde_json::to_string(&r).unwrap();
        assert!(
            encoded.len() <= TASK_RESULT_MAX_BYTES,
            "{unit}: {}",
            encoded.len()
        );
        let parsed: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(parsed["task"]["id"], "t2");
        let reviews = parsed["reviews"].as_array().unwrap();
        let newest = reviews.last().unwrap();
        assert_eq!(newest["round"], 3, "{unit}");
        assert!(
            !newest["findings"].as_array().unwrap().is_empty(),
            "{unit}: the newest review keeps findings"
        );
        let kept: usize = reviews
            .iter()
            .map(|r| r["findings"].as_array().unwrap().len())
            .sum();
        let omitted = parsed["omitted"]["findings"].as_u64().unwrap_or(0) as usize;
        assert_eq!(kept + omitted, 2 * 50, "{unit}: {}", parsed["omitted"]);
        // The count trims ran (second review, Minor 4), not only the string cuts.
        assert!(omitted > 0, "{unit}: {}", parsed["omitted"]);
        let research = &parsed["research"];
        let listed: usize = ["files", "modules", "interfaces", "risks"]
            .iter()
            .map(|k| research[k].as_array().unwrap().len())
            .sum();
        let omitted_research = parsed["omitted"]["research"].as_u64().unwrap_or(0) as usize;
        assert!(omitted_research > 0, "{unit}: {}", parsed["omitted"]);
        assert_eq!(listed + omitted_research, 60 + 40 + 40 + 20, "{unit}");
        let proofs = parsed["proofs"].as_array().unwrap().len();
        let omitted_proofs = parsed["omitted"]["proofs"].as_u64().unwrap_or(0) as usize;
        assert!(omitted_proofs > 0, "{unit}: {}", parsed["omitted"]);
        assert_eq!(proofs + omitted_proofs, 50, "{unit}");
    }
}
