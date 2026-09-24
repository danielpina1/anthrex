//! Fix round 4 (ruling T16-R4, re-review 3): per-field hostile-input sweeps.
//!
//! NB4: `str::lines()` splits only on `\n` and `\r\n`, but CommonMark also ends a
//! line at a bare `\r`, so text after a bare `\r` used to reach the parser as a new
//! line at column 0 that `normalize_line` had never seen. Every field is now
//! checked with a bare `\r` separator as well as `\n` and `\r\n`.
//!
//! The punctuation fuzz (moved here from `report_tests_escaping.rs`, which was
//! nearing the 600-line guidance) now plants its hostile line in one field per
//! render and asserts per field, so a failure names the field that forged
//! structure instead of reporting only that the combined document changed.

use proto::{Finding, Severity, Verdict};

use super::super::super::*;
use super::super::base_run;
use super::{NodeCounts, node_counts};
use crate::run::model::{CheckRecord, LogEntry, ProofRecord, ReviewRecord, TaskEvent};

/// One untrusted field of the report: how to plant a value in it, and the
/// structure that planting it legitimately adds on top of the clean baseline.
/// `one_line` marks a field flattened onto a single line (`escape_cell`): anything
/// `<`-led in it lands mid-line, where inline HTML is an accepted risk (T16-R4), so
/// its inline-HTML count is not asserted. Block HTML is asserted for every field.
struct Field {
    name: &'static str,
    plant: fn(&mut Run, String),
    new_lists: usize,
    new_fences: usize,
    one_line: bool,
}

fn t1(run: &mut Run) -> &mut crate::run::model::Task {
    run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap()
}

fn review(run: &mut Run, summary: String, findings: Vec<Finding>) {
    let t = t1(run);
    t.reviews.push(ReviewRecord {
        round: 1,
        route: t.route.clone(),
        base: "a".repeat(40),
        head: "b".repeat(40),
        verdict: Some(Verdict::Changes),
        summary,
        findings,
    });
}

fn finding(file: Option<String>, text: String) -> Finding {
    Finding {
        severity: Severity::Important,
        file,
        line: Some(1),
        input: None,
        text,
    }
}

const FIELDS: &[Field] = &[
    Field {
        name: "Run.goal",
        plant: |run, v| run.goal = v,
        new_lists: 0,
        one_line: false,
        new_fences: 0,
    },
    Field {
        name: "Task.notes",
        plant: |run, v| t1(run).notes.push(v),
        new_lists: 1,
        one_line: false,
        new_fences: 0,
    },
    Field {
        name: "ProofRecord.test",
        plant: |run, v| {
            t1(run).proofs.push(ProofRecord {
                at: 1_600,
                test: v,
                red: "a".repeat(40),
                head: "b".repeat(40),
                red_failed: true,
                head_passed: true,
                matched: true,
                red_tail: String::new(),
                head_tail: String::new(),
            })
        },
        new_lists: 0,
        one_line: true,
        new_fences: 0,
    },
    Field {
        name: "CheckRecord.tail",
        plant: |run, v| {
            t1(run).checks.push(CheckRecord {
                at: 1_700,
                ok: false,
                code: Some(1),
                timed_out: false,
                // A fence inside the tail too: the report's own fence must outlast it.
                tail: format!("{v}\n```\nend"),
                secs: 5,
                on_candidate: false,
            })
        },
        new_lists: 0,
        one_line: false,
        new_fences: 1,
    },
    Field {
        name: "ReviewRecord.summary",
        plant: |run, v| review(run, v, vec![]),
        new_lists: 0,
        one_line: false,
        new_fences: 0,
    },
    Field {
        name: "Finding.text",
        plant: |run, v| review(run, "ok".to_string(), vec![finding(None, v)]),
        new_lists: 1,
        one_line: false,
        new_fences: 0,
    },
    Field {
        name: "Finding.file",
        plant: |run, v| {
            let f = finding(Some(v), "finding".to_string());
            review(run, "ok".to_string(), vec![f])
        },
        new_lists: 1,
        one_line: true,
        new_fences: 0,
    },
    Field {
        name: "Task.history",
        plant: |run, v| t1(run).history.push(TaskEvent { at: 1_800, text: v }),
        new_lists: 1,
        one_line: false,
        new_fences: 0,
    },
    Field {
        name: "Run.log",
        plant: |run, v| run.log.push(LogEntry { at: 1_900, text: v }),
        new_lists: 1,
        one_line: false,
        new_fences: 0,
    },
];

/// Renders the fixture with `value` planted in `field` alone and asserts the parse
/// is exactly the baseline plus what that field legitimately adds, naming the field
/// and `case` on failure.
fn assert_forges_nothing(field: &Field, value: String, baseline: &NodeCounts, case: &str) {
    let mut run = base_run();
    (field.plant)(&mut run, value);
    let out = render(&run, 2_000);
    let mut actual = node_counts(&out);
    if field.one_line {
        actual.inline_html = baseline.inline_html;
    }
    let expected = NodeCounts {
        lists: baseline.lists + field.new_lists,
        fences: baseline.fences + field.new_fences,
        ..*baseline
    };
    assert_eq!(
        actual, expected,
        "{} forged Markdown structure ({case}):\n{out}",
        field.name
    );
}

/// Every block-start hazard, each after a bare `\r` — no `\n` anywhere.
const BARE_CR_HOSTILE: &str = "first\r# forged heading\r> forged quote\r- forged bullet\r\
1. forged ordered\r<div>\rswallowed\r</div>\r| a | b |\r|---|---|\r===";

#[test]
fn a_bare_carriage_return_forges_nothing_in_any_field_nb4() {
    let baseline = node_counts(&render(&base_run(), 2_000));
    assert_eq!(
        (baseline.html, baseline.inline_html),
        (0, 0),
        "the baseline has no HTML to compare against"
    );
    for field in FIELDS {
        assert_forges_nothing(field, BARE_CR_HOSTILE.to_string(), &baseline, "bare \\r");
        // The same payload with every separator as `\r\n`, and mixed with `\n`.
        let crlf = BARE_CR_HOSTILE.replace('\r', "\r\n");
        assert_forges_nothing(field, crlf, &baseline, "\\r\\n");
        let mixed = BARE_CR_HOSTILE.replacen('\r', "\n", 3);
        assert_forges_nothing(field, mixed, &baseline, "mixed \\r and \\n");
    }
}

/// Every ASCII punctuation character, with no leading whitespace, a leading space, or
/// a leading tab, planted in one field at a time — as the field's first line and as
/// a continuation line after each of the three line endings — so a failure names the
/// field, the character, the whitespace and the shape.
#[test]
fn every_ascii_punctuation_character_at_line_start_forges_nothing() {
    let baseline = node_counts(&render(&base_run(), 2_000));
    let punctuation: Vec<char> = (0x21u8..=0x7e)
        .map(char::from)
        .filter(char::is_ascii_punctuation)
        .collect();
    assert_eq!(punctuation.len(), 32, "the full ASCII punctuation set");

    for field in FIELDS {
        for &c in &punctuation {
            for ws in ["", " ", "\t"] {
                let hostile = format!("{ws}{c} forged");
                let shapes = [
                    ("first line", format!("{hostile}\nrest")),
                    ("after \\n", format!("lead\n{hostile}")),
                    ("after \\r\\n", format!("lead\r\n{hostile}")),
                    ("after \\r", format!("lead\r{hostile}")),
                ];
                for (shape, value) in shapes {
                    let case = format!("{c:?}, ws {ws:?}, {shape}");
                    assert_forges_nothing(field, value, &baseline, &case);
                }
            }
        }
    }
}
