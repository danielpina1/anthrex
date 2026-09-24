//! Fix round 1 (ruling T16-fix): hostile-input tests for the escaping layer
//! (`report_escape.rs`), plus M1/M2's content assertions the original suite was
//! missing.

use proto::{AgentRole, Finding, Runtime, Severity, Verdict};

use super::super::*;
use super::{base_run, round};
use crate::run::model::{CheckRecord, ReviewRecord, TaskEvent};
use crate::run::test_support::task;

#[test]
fn title_with_pipe_and_newline_does_not_break_the_table() {
    let mut run = base_run();
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.spec.title = "Evil | pipe\ntitle across two lines".to_string();
    let out = render(&run, 2_000);

    // The table row is exactly one line: find the line that starts the t1 row and
    // confirm it also holds the row's last cell (`merge commit`, `-` here), which
    // could only be true if the embedded newline did not split the row.
    let row = out
        .lines()
        .find(|l| l.starts_with("| t1 |"))
        .expect("a single-line t1 row");
    assert!(row.contains("Evil \\| pipe title across two lines"));
    assert!(row.trim_end().ends_with("| - |"));
    // The raw (unescaped, newline-split) title never appears as its own bare
    // continuation line, in the table or in the task's own heading below it.
    assert!(!out.lines().any(|l| l == "title across two lines"));
}

#[test]
fn title_starting_with_hash_does_not_become_a_heading_break() {
    let mut run = base_run();
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.spec.title = "# not a heading\nsecond line".to_string();
    let out = render(&run, 2_000);

    let heading = out
        .lines()
        .find(|l| l.starts_with("## t1:"))
        .expect("a single-line task heading");
    assert_eq!(heading, "## t1: \\# not a heading second line");
    // The raw, unescaped continuation never appears as its own bare line.
    assert!(!out.contains("\n# not a heading\n"));
}

#[test]
fn check_tail_with_a_backtick_fence_uses_a_longer_fence() {
    let mut run = base_run();
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.checks.push(CheckRecord {
        at: 1_700,
        ok: false,
        code: Some(1),
        timed_out: false,
        tail: "some output\n```\nnot really the end\n```\nmore text".to_string(),
        secs: 5,
        on_candidate: false,
    });
    let out = render(&run, 2_000);

    // The longest run in the tail is 3, so the fence must be 4 backticks: a naive
    // 3-backtick fence would close on the tail's own first ``` line. The tail's own
    // bare ``` lines are expected to survive verbatim between the two 4-backtick
    // fences, so the fence lines are found by their neighbours, not by a blanket
    // "no bare ``` line anywhere" check.
    assert!(out.lines().any(|l| l == "````"));
    let open = out
        .find("````\nsome output")
        .expect("the 4-backtick fence opens the block");
    let close = out[open..]
        .find("more text\n````")
        .expect("the 4-backtick fence closes after the tail");
    // Everything between is the tail verbatim, including its own embedded ``` lines.
    let body = &out[open..open + close + "more text\n````".len()];
    assert!(body.contains("not really the end"));
    assert!(
        body.contains("\n```\n"),
        "the tail's own fence survives verbatim"
    );
}

#[test]
fn notes_findings_and_history_with_newlines_and_hash_indent_their_continuation() {
    let mut run = base_run();
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.notes
        .push("note with a newline\n# looks like a heading".to_string());
    t.reviews.push(ReviewRecord {
        round: 1,
        route: t.route.clone(),
        base: "a".repeat(40),
        head: "b".repeat(40),
        verdict: Some(Verdict::Changes),
        summary: "ok".to_string(),
        findings: vec![Finding {
            severity: Severity::Important,
            file: None,
            line: None,
            input: None,
            text: "finding with a newline\n# looks like a heading".to_string(),
        }],
    });
    t.history.push(TaskEvent {
        at: 1_800,
        text: "history with a newline\n# looks like a heading".to_string(),
    });
    let out = render(&run, 2_000);

    // Every continuation line is indented under its item, never a bare line.
    assert!(!out.contains("\n# looks like a heading"));
    assert_eq!(
        out.matches("\n    # looks like a heading").count(),
        3,
        "notes, findings and history must each indent their continuation:\n{out}"
    );
}

#[test]
fn salvage_ref_content_is_asserted_m1() {
    let mut run = base_run();
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.salvage_refs
        .push("refs/anthrex/salvage/add-password-reset-3f9a/t1/1".to_string());
    let out = render(&run, 2_000);
    assert!(out.contains("refs/anthrex/salvage/add-password-reset-3f9a/t1/1"));
}

#[test]
fn route_and_review_route_content_is_asserted_m2() {
    let run = base_run();
    let t = task(&run, "t1");
    let out = render(&run, 2_000);
    assert!(out.contains("Route: claude claude-sonnet-5 (standard/high)"));
    assert!(
        t.review_route.is_some(),
        "the fixture task must be reviewed"
    );
    // Whatever route the roster picked, its rendered line must name that exact
    // runtime/model/strength/effort, not a placeholder.
    let rr = t.review_route.as_ref().unwrap();
    let expected = format!(
        "Review route: {} {} ({}/{})",
        match rr.runtime {
            Runtime::Claude => "claude",
            Runtime::Codex => "codex",
            Runtime::Shell => "shell",
        },
        rr.model,
        match rr.strength {
            proto::Strength::Fast => "fast",
            proto::Strength::Standard => "standard",
            proto::Strength::Frontier => "frontier",
        },
        match rr.effort {
            proto::Effort::Low => "low",
            proto::Effort::Medium => "medium",
            proto::Effort::High => "high",
        },
    );
    assert!(out.contains(&expected), "missing {expected:?} in:\n{out}");
}

#[test]
fn round_usage_line_survives_alongside_escaping() {
    // A guard against the escaping pass accidentally touching the per-round usage
    // line (it carries no model-authored free text, only counters).
    let mut run = base_run();
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.rounds.push(round(AgentRole::Worker, 1, 2, 3, 0));
    let out = render(&run, 2_000);
    assert!(out.contains("2 turns, 3 tool calls"));
}
