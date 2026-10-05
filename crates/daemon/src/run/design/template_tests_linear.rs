//! Task M9.6.9's carries from task 4's review: the template checks run in the engine
//! reducer, so they stay linear in the capped input; and they follow CommonMark on
//! indentation (a fence or a heading has 0 to 3 leading spaces), escaped backticks and
//! an unclosed fence.

use std::time::{Duration, Instant};

use proto::DocKind;

use super::tests::{REPORT, SPEC, two};
use super::{cap_bytes, check};

/// Separates the one-pass tag removal from the old remove-and-search-again loop on a
/// 32 KiB heading (`docs/timing-budgets.md`, "Recorded, from M9.6.9").
const LINEAR_BOUND: Duration = Duration::from_millis(40);

/// A merged report whose second approach heading is `[both]` repeated to fill the
/// 32 KiB cap.
fn both_heading_report() -> String {
    let old = "### 2. Stored tokens [both]";
    let room = cap_bytes(DocKind::Brainstorm) - (REPORT.len() - old.len());
    let tags = "[both]".repeat((room - "### 2. Stored tokens ".len()) / "[both]".len());
    REPORT.replace(old, &format!("### 2. Stored tokens {tags}"))
}

/// The carry: the tag removal lower-cases once and rebuilds the name in one pass, so a
/// heading of thousands of tags is read in linear time.
#[test]
fn tag_removal_is_linear_on_a_32_kib_heading() {
    let text = both_heading_report();
    assert!(text.len() <= cap_bytes(DocKind::Brainstorm));
    assert!(
        text.len() > cap_bytes(DocKind::Brainstorm) - 16,
        "fills the cap"
    );
    let start = Instant::now();
    let checked = check(DocKind::Brainstorm, &text, &two());
    let took = start.elapsed();
    assert_eq!(checked, Ok(()), "the name is still Stored tokens");
    assert!(took < LINEAR_BOUND, "took {took:?}");
}

/// The name a heading's tags leave: removed in one pass, case aside, whatever their
/// number; a tag that the removal of another would join is left as written.
#[test]
fn tags_are_removed_in_one_pass() {
    let tags = ["[claude]".to_string(), "[both]".to_string()];
    assert_eq!(
        super::approach_name("2. Stored [BOTH][both] tokens [Claude]", &tags),
        "Stored  tokens"
    );
    assert_eq!(super::approach_name("[bo[both]th] x", &tags), "[both] x");
    assert_eq!(
        super::approach_name("Ünïcode [both] é", &tags),
        "Ünïcode  é"
    );
}

/// CommonMark: a heading or a fence sits after at most three spaces; four or more make
/// an indented code line, which is neither.
#[test]
fn a_fence_or_heading_indented_four_spaces_is_neither() {
    let three = SPEC.replace("## Risks\n", "   ## Risks\n");
    assert_eq!(check(DocKind::Spec, &three, &two()), Ok(()));
    let four = SPEC.replace("## Risks\n", "    ## Risks\n");
    assert_eq!(
        check(DocKind::Spec, &four, &two()),
        Err("the spec is missing the section \"## Risks\"".to_string())
    );
    // A fence after three spaces hides the TBD inside it; after four it is text.
    let fenced = |indent: &str| {
        SPEC.replace(
            "Stored tokens.\n",
            &format!("{indent}```\nTBD\n{indent}```\nStored tokens.\n"),
        )
    };
    assert_eq!(check(DocKind::Spec, &fenced("   "), &two()), Ok(()));
    let text = fenced("    ");
    let line = text.lines().position(|l| l == "TBD").unwrap() + 1;
    assert_eq!(
        check(DocKind::Spec, &text, &two()),
        Err(format!("the spec still says TBD or TODO at line {line}"))
    );
    // A closer indented four spaces does not close: the fence runs on, so the next
    // section's heading is code.
    let unclosed = SPEC.replace("## Testing\n", "```\nx\n    ```\n## Testing\n");
    assert_eq!(
        check(DocKind::Spec, &unclosed, &two()),
        Err("the spec is missing the section \"## Testing\"".to_string())
    );
}

/// CommonMark: a backslash before a backtick outside a span makes that backtick text;
/// the rest of its run is still a run. Inside a span a backslash is text, so it never
/// escapes a closer (CommonMark's `` `foo\`bar` `` is the span `foo\`).
#[test]
fn an_escaped_backtick_is_not_a_code_span_delimiter() {
    let line = SPEC.lines().position(|l| l == "Stored tokens.").unwrap() + 1;
    let refused = Err(format!("the spec still says TBD or TODO at line {line}"));
    for prose in [
        // The escaped opener is text; the closer has no partner.
        "See \\`TODO` here.\n",
        // The span is `x\`, closed by the second backtick; TBD follows it.
        "See `x\\` TBD.\n",
    ] {
        let text = SPEC.replace("Stored tokens.\n", prose);
        assert_eq!(check(DocKind::Spec, &text, &two()), refused, "{prose}");
    }
    for masked in [
        // An escaped backslash escapes nothing: the backtick opens the span.
        "See \\\\`TODO` here.\n",
        // The first backtick of the run is text; the second opens a one-backtick span.
        "See \\``TODO` here.\n",
    ] {
        let text = SPEC.replace("Stored tokens.\n", masked);
        assert_eq!(check(DocKind::Spec, &text, &two()), Ok(()), "{masked}");
    }
}

/// Pinned: a fence that never closes runs to the end of the document, so every heading
/// after it is code and every placeholder after it is hidden.
#[test]
fn an_unclosed_fence_runs_to_the_end_of_the_document() {
    let text = SPEC.replace("## Risks\n", "```\n## Risks\n");
    assert_eq!(
        check(DocKind::Spec, &text, &two()),
        Err("the spec is missing the section \"## Risks\"".to_string())
    );
    let text = format!("{SPEC}~~~\nTBD\n");
    assert_eq!(check(DocKind::Spec, &text, &two()), Ok(()));
    let lines = super::lines(&text);
    assert!(lines.last().unwrap().code);
    assert_eq!(super::open_fence(&text).as_deref(), Some("~~~"));
    assert_eq!(super::open_fence(SPEC), None);
}
