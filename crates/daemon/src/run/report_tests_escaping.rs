//! Fix round 1 (ruling T16-fix): hostile-input tests for the escaping layer
//! (`report_escape.rs`), plus M1/M2's content assertions the original suite was
//! missing.
//!
//! Fix round 2 (ruling T16-R2, re-review 1): round 1's `continuation_indent` only
//! neutralised free text used as a *plain* line (`merged without approval`, a review
//! summary); the three list-item call sites it also protected (Notes, review
//! Findings, History) stayed exploitable, since a list item only needs 2 columns of
//! indentation to stay inside it, leaving 2 of the 4 indented columns still read as a
//! block start by a real CommonMark parser. `Run.goal`, `ProofRecord.test` and
//! `Finding.file` were not routed through any escaping at all. `pulldown_cmark` (a new
//! dev-only dependency — none of the existing ones parse Markdown) verifies the fixed
//! output structurally rather than by string-matching indentation, which is what let
//! round 1's own tests pass over a still-broken mechanism.

use proto::{AgentRole, Finding, Runtime, Severity, Verdict};
use pulldown_cmark::{Event, Options, Parser, Tag};

use super::super::*;
use super::{base_run, round};
use crate::run::model::{CheckRecord, LogEntry, ProofRecord, ReviewRecord, TaskEvent};
use crate::run::test_support::task;

/// Every block-start hazard T16-R2 names, concatenated as one poisoned field: a
/// heading, a blockquote, a bullet, an ordered marker, a fence (backtick and tilde),
/// a thematic break and a setext underline — each on its own line.
const HOSTILE_LINES: &str = "plain first line\n\
# forged heading\n\
> forged quote\n\
- forged bullet\n\
+ forged bullet\n\
* forged bullet\n\
1. forged ordered\n\
42) forged ordered\n\
```\n\
forged fence\n\
```\n\
~~~\n\
forged tilde fence\n\
~~~\n\
---\n\
===\n\
| fake | table |\n\
|---|---|";

/// Counts of `Start` events per kind pulldown-cmark emits for `text`, with GFM tables
/// enabled (the report's own `## Tasks` table needs it to parse as a table at all).
/// `html` counts both block and inline HTML nodes (NB3: a raw HTML block absorbing a
/// note's own continuation lines).
#[derive(Debug, Default, PartialEq, Eq)]
struct NodeCounts {
    headings: usize,
    lists: usize,
    tables: usize,
    fences: usize,
    html: usize,
}

fn node_counts(text: &str) -> NodeCounts {
    let parser = Parser::new_ext(text, Options::ENABLE_TABLES);
    let mut counts = NodeCounts::default();
    for event in parser {
        match event {
            Event::Start(Tag::Heading { .. }) => counts.headings += 1,
            Event::Start(Tag::List(_)) => counts.lists += 1,
            Event::Start(Tag::Table(_)) => counts.tables += 1,
            Event::Start(Tag::CodeBlock(_)) => counts.fences += 1,
            Event::Html(_) | Event::InlineHtml(_) => counts.html += 1,
            _ => {}
        }
    }
    counts
}

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
fn notes_findings_and_history_do_not_forge_real_markdown_structure() {
    // Round 1's mistake, reproduced: 4-space `continuation_indent` inside a `"- "`
    // list item leaves only 2 spare columns once the item's own 2-column content
    // indent is subtracted, and CommonMark still reads a block start at up to 3
    // columns. A real parser, not a string match on "is it indented", is what catches
    // this — re-review 1 found it exactly because round 1's own string-matching test
    // could not tell the difference.
    let mut run = base_run();
    let baseline = node_counts(&render(&run, 2_000));
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.notes.push(format!("note first line\n{HOSTILE_LINES}"));
    t.reviews.push(ReviewRecord {
        round: 1,
        route: t.route.clone(),
        base: "a".repeat(40),
        head: "b".repeat(40),
        verdict: Some(Verdict::Changes),
        summary: "ok".to_string(),
        findings: vec![Finding {
            severity: Severity::Important,
            file: Some("# evil.rs".to_string()),
            line: Some(1),
            input: None,
            text: format!("finding first line\n{HOSTILE_LINES}"),
        }],
    });
    t.history.push(TaskEvent {
        at: 1_800,
        text: format!("history first line\n{HOSTILE_LINES}"),
    });
    let out = render(&run, 2_000);

    // Three more list items (one each for the poisoned note, finding and history
    // entry) are the only structural change a real parser should see: no new
    // headings, no new fences, no forged table, and only those 3 extra lists.
    let counts = node_counts(&out);
    assert_eq!(
        counts.headings, baseline.headings,
        "a poisoned line forged a heading:\n{out}"
    );
    assert_eq!(
        counts.tables, baseline.tables,
        "a poisoned line forged a table:\n{out}"
    );
    assert_eq!(
        counts.fences, baseline.fences,
        "a poisoned line forged a fence:\n{out}"
    );
    assert_eq!(
        counts.lists,
        baseline.lists + 3,
        "expected exactly 3 new list items (notes, findings, history), not forged nested ones:\n{out}"
    );
}

#[test]
fn run_goal_with_hostile_lines_forges_nothing_n1() {
    let mut run = base_run();
    let baseline = node_counts(&render(&run, 2_000));
    run.goal = format!("Evil goal\n{HOSTILE_LINES}");
    let out = render(&run, 2_000);
    assert_eq!(
        node_counts(&out),
        baseline,
        "Run.goal forged Markdown structure:\n{out}"
    );
    assert!(out.contains("Goal: Evil goal"));
}

#[test]
fn proof_test_field_with_a_newline_does_not_split_its_line_n2() {
    let mut run = base_run();
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.proofs.push(ProofRecord {
        at: 1_600,
        test: "evil::test\n# forged heading".to_string(),
        red: "a".repeat(40),
        head: "b".repeat(40),
        red_failed: true,
        head_passed: true,
        matched: true,
        red_tail: String::new(),
        head_tail: String::new(),
    });
    let out = render(&run, 2_000);
    let proof_line = out
        .lines()
        .find(|l| l.starts_with("Proof 1:"))
        .expect("a single-line Proof entry");
    assert!(proof_line.contains("test=evil::test # forged heading"));
    assert!(proof_line.trim_end().ends_with(')'));
    assert!(!out.lines().any(|l| l == "# forged heading"));
}

#[test]
fn finding_file_with_a_leading_hash_does_not_forge_a_heading_n3() {
    let mut run = base_run();
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.reviews.push(ReviewRecord {
        round: 1,
        route: t.route.clone(),
        base: "a".repeat(40),
        head: "b".repeat(40),
        verdict: Some(Verdict::Changes),
        summary: "ok".to_string(),
        findings: vec![Finding {
            severity: Severity::Important,
            file: Some("# evil.rs\nsecond line".to_string()),
            line: Some(1),
            input: None,
            text: "unrelated finding text".to_string(),
        }],
    });
    let out = render(&run, 2_000);
    let counts = node_counts(&out);
    assert_eq!(counts.headings, 4, "Finding.file forged a heading:\n{out}");
    assert!(!out.lines().any(|l| l == "# evil.rs"));
    assert!(!out.lines().any(|l| l == "second line"));
}

#[test]
fn log_entries_do_not_forge_markdown_structure() {
    let mut run = base_run();
    let baseline = node_counts(&render(&run, 2_000));
    run.log.push(LogEntry {
        at: 1_900,
        text: format!("log first line\n{HOSTILE_LINES}"),
    });
    let out = render(&run, 2_000);
    let counts = node_counts(&out);
    assert_eq!(
        counts.headings, baseline.headings,
        "a log entry forged a heading:\n{out}"
    );
    assert_eq!(
        counts.tables, baseline.tables,
        "a log entry forged a table:\n{out}"
    );
    assert_eq!(
        counts.fences, baseline.fences,
        "a log entry forged a fence:\n{out}"
    );
    assert_eq!(
        counts.lists,
        baseline.lists + 1,
        "expected exactly 1 new log list item:\n{out}"
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

// --- Fix round 3 (ruling T16-R3, re-review 2): NB1-NB3 and a punctuation fuzz -----

#[test]
fn review_summary_first_line_does_not_setext_promote_the_round_header_nb1() {
    let mut run = base_run();
    let baseline = node_counts(&render(&run, 2_000));
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.reviews.push(ReviewRecord {
        round: 1,
        route: t.route.clone(),
        base: "a".repeat(40),
        head: "b".repeat(40),
        verdict: Some(Verdict::Changes),
        summary: "===\nrest of summary".to_string(),
        findings: vec![],
    });
    let out = render(&run, 2_000);
    assert_eq!(
        node_counts(&out).headings,
        baseline.headings,
        "the review round's own header line was setext-promoted into a heading:\n{out}"
    );
    assert!(out.contains("\\===\nrest of summary") || out.contains("\\===\n    rest of summary"));
}

#[test]
fn a_leading_tab_does_not_bypass_the_escape_nb2() {
    let mut run = base_run();
    let baseline = node_counts(&render(&run, 2_000));
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.notes
        .push("first\n\t# tab-indented heading attempt".to_string());
    let out = render(&run, 2_000);
    assert_eq!(
        node_counts(&out).headings,
        baseline.headings,
        "a leading tab bypassed escape_block_start's column counting:\n{out}"
    );
}

#[test]
fn a_leading_angle_bracket_does_not_open_an_html_block_nb3() {
    let mut run = base_run();
    let baseline = node_counts(&render(&run, 2_000));
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.notes
        .push("note first line\n<div>\nswallowed?\n</div>\nafter".to_string());
    let out = render(&run, 2_000);
    let counts = node_counts(&out);
    assert_eq!(
        baseline.html, 0,
        "the baseline must have no HTML nodes to compare against"
    );
    assert_eq!(
        counts.html, 0,
        "a leading `<` opened a real HTML block:\n{out}"
    );
}

/// Every ASCII punctuation character, with no leading whitespace, a leading space, or
/// a leading tab — the reviewer's own checklist ("with and without leading spaces and
/// tabs") — planted as a continuation line (or, for the review summary, the first
/// line, since that is NB1's exact shape) in every field T16-R3 names. One render per
/// `(char, whitespace)` pair; each must forge no heading, table, fence or HTML node
/// beyond baseline, and exactly the 4 list items (notes, finding, history, log) the
/// fixture itself adds.
#[test]
fn every_ascii_punctuation_character_at_line_start_forges_nothing() {
    let baseline = node_counts(&render(&base_run(), 2_000));
    let punctuation: Vec<char> = (0x21u8..=0x7e)
        .map(char::from)
        .filter(char::is_ascii_punctuation)
        .collect();
    assert_eq!(punctuation.len(), 32, "the full ASCII punctuation set");

    for &c in &punctuation {
        for ws in ["", " ", "\t"] {
            let hostile = format!("{ws}{c} forged");
            let mut run = base_run();
            run.goal = format!("Evil goal\n{hostile}");
            let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
            t.notes.push(format!("note\n{hostile}"));
            t.proofs.push(ProofRecord {
                at: 1_600,
                test: format!("test\n{hostile}"),
                red: "a".repeat(40),
                head: "b".repeat(40),
                red_failed: true,
                head_passed: true,
                matched: true,
                red_tail: String::new(),
                head_tail: String::new(),
            });
            t.checks.push(CheckRecord {
                at: 1_700,
                ok: false,
                code: Some(1),
                timed_out: false,
                tail: format!("tail\n{hostile}\n```\nend"),
                secs: 5,
                on_candidate: false,
            });
            t.reviews.push(ReviewRecord {
                round: 1,
                route: t.route.clone(),
                base: "a".repeat(40),
                head: "b".repeat(40),
                verdict: Some(Verdict::Changes),
                // The first line, on purpose: NB1's exact shape.
                summary: format!("{hostile}\nrest"),
                findings: vec![Finding {
                    severity: Severity::Important,
                    file: Some(hostile.clone()),
                    line: Some(1),
                    input: None,
                    text: format!("finding\n{hostile}"),
                }],
            });
            t.history.push(TaskEvent {
                at: 1_800,
                text: format!("hist\n{hostile}"),
            });
            run.log.push(LogEntry {
                at: 1_900,
                text: format!("log\n{hostile}"),
            });

            let out = render(&run, 2_000);
            let counts = node_counts(&out);
            assert_eq!(
                counts.headings, baseline.headings,
                "{c:?} (ws {ws:?}) forged a heading:\n{out}"
            );
            assert_eq!(
                counts.tables, baseline.tables,
                "{c:?} (ws {ws:?}) forged a table:\n{out}"
            );
            assert_eq!(
                counts.html, 0,
                "{c:?} (ws {ws:?}) opened an HTML block:\n{out}"
            );
            // The check's own tail must still be exactly one fenced block: raw tool
            // output is never escaped, only fenced, and the fence must not be broken
            // by punctuation inside it either.
            assert_eq!(
                counts.fences,
                baseline.fences + 1,
                "{c:?} (ws {ws:?}) broke the check's fence:\n{out}"
            );
            assert_eq!(
                counts.lists,
                baseline.lists + 4,
                "{c:?} (ws {ws:?}): expected exactly 4 new list items (notes, finding, history, log):\n{out}"
            );
        }
    }
}
