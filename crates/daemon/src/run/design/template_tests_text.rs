//! Task M9.6.4 fix round 1: fences, inline code, the cap's order and the
//! recommendation's whole-word match (review m1, m2, m7; ruling T4-3).

use proto::DocKind;

use super::tests::{REPORT, SPEC, two};
use super::{admit, check, clean};

/// Review m1: an opener's info string has no backtick, a closer is a bare marker, and
/// an inline triple-backtick span opens no fence.
#[test]
fn fences_open_and_close_only_on_real_fence_lines() {
    // Inline spans of three backticks, mid-line and at the start: no fence opens, so
    // the TBD after them is prose.
    for inline in ["Run ```cargo test``` first\n", "```cargo test``` first\n"] {
        let text = SPEC.replace("Stored tokens.\n", &format!("{inline}TBD\n"));
        let line = text.lines().position(|l| l == "TBD").unwrap() + 1;
        assert_eq!(
            check(DocKind::Spec, &text, &two()),
            Err(format!("the spec still says TBD or TODO at line {line}")),
            "{inline}"
        );
    }
    // A marker with an info string inside an open fence does not close it.
    let inner = SPEC.replace(
        "Stored tokens.\n",
        "```markdown\n```rust\nTBD\n```\nStored tokens.\n",
    );
    assert_eq!(check(DocKind::Spec, &inner, &two()), Ok(()));
    // A closer may be longer than the opener, never shorter; a tilde never closes a
    // backtick fence.
    let longer = SPEC.replace("Stored tokens.\n", "```\nTBD\n`````\nStored tokens.\n");
    assert_eq!(check(DocKind::Spec, &longer, &two()), Ok(()));
    let shorter = SPEC.replace("Stored tokens.\n", "````\n```\nTBD\n````\nStored tokens.\n");
    assert_eq!(check(DocKind::Spec, &shorter, &two()), Ok(()));
    let tilde = SPEC.replace("Stored tokens.\n", "```\n~~~\nTBD\n```\nStored tokens.\n");
    assert_eq!(check(DocKind::Spec, &tilde, &two()), Ok(()));
    // A tilde fence's info string may hold backticks.
    let tilde_info = SPEC.replace("Stored tokens.\n", "~~~ `x`\nTBD\n~~~\nStored tokens.\n");
    assert_eq!(check(DocKind::Spec, &tilde_info, &two()), Ok(()));
}

/// Review m2: the raw text is over the cap and the cleaned text under it. Capping the
/// cleaned text instead would let it through; capping the raw bytes refuses it.
#[test]
fn the_cap_is_on_the_raw_bytes_not_the_cleaned_text() {
    let pad = "\u{200b}".repeat(64 * 1024 / 3 + 1);
    let raw = format!("{SPEC}{pad}");
    assert!(raw.len() > 64 * 1024);
    assert!(proto::safe_text::multi_line(&raw).len() < 64 * 1024);
    let refused = Err("the spec is over its 64 KiB cap".to_string());
    assert_eq!(admit(DocKind::Spec, &raw, &two()), refused);
    assert_eq!(clean(DocKind::Spec, &raw), refused);
}

/// Review m7: an inline code span is a backtick run closed by a run of the same length.
#[test]
fn inline_code_is_masked_by_runs_of_equal_length() {
    let line = SPEC.lines().position(|l| l == "Stored tokens.").unwrap() + 1;
    let refused = Err(format!("the spec still says TBD or TODO at line {line}"));
    for masked in [
        "See ``TODO`` handling.\n",
        "See ``a ` TODO`` handling.\n",
        "See `TBD` and `x`.\n",
    ] {
        let text = SPEC.replace("Stored tokens.\n", masked);
        assert_eq!(check(DocKind::Spec, &text, &two()), Ok(()), "{masked}");
    }
    for prose in [
        "A lone ` backtick, then TODO.\n",
        "``TODO` is not closed by one.\n",
        "`x` then TBD.\n",
    ] {
        let text = SPEC.replace("Stored tokens.\n", prose);
        assert_eq!(check(DocKind::Spec, &text, &two()), refused, "{prose}");
    }
}

/// Ruling T4-3: the recommendation names an approach as a whole word or phrase,
/// case-insensitively; a name under three characters must match exactly.
#[test]
fn a_recommendation_names_an_approach_as_a_whole_word() {
    let short = REPORT
        .replace("### 1. Signed tokens [claude]", "### A [both]")
        .replace("### 2. Stored tokens [both]", "### B [codex]");
    let with = |rec: &str| {
        short.replace(
            "Go with Stored tokens, because links must be revocable.",
            rec,
        )
    };
    let refused = Err("the recommendation must name one of the listed approaches".to_string());
    for named in ["Go with A.", "B, because it is safer.", "Pick (A)"] {
        assert_eq!(
            check(DocKind::Brainstorm, &with(named), &two()),
            Ok(()),
            "{named}"
        );
    }
    for unnamed in [
        "Go with a table.",
        "Approach Bravo.",
        "Go with b.",
        "AB testing",
    ] {
        assert_eq!(
            check(DocKind::Brainstorm, &with(unnamed), &two()),
            refused,
            "{unnamed}"
        );
    }
    // Three characters or more: case-insensitive, but still a whole word or phrase.
    let long = |rec: &str| {
        REPORT.replace(
            "Go with Stored tokens, because links must be revocable.",
            rec,
        )
    };
    assert_eq!(
        check(DocKind::Brainstorm, &long("STORED TOKENS!"), &two()),
        Ok(())
    );
    assert_eq!(
        check(DocKind::Brainstorm, &long("Prestored tokens."), &two()),
        refused
    );
    assert_eq!(
        check(DocKind::Brainstorm, &long("Stored tokensets."), &two()),
        refused
    );
}
