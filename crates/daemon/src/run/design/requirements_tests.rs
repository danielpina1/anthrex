//! Task M9.6.4: decision 14's requirement parsing, with the exact refusal texts.

use super::{GOAL_CAP, Requirement, TEXT_CAP, goal_section, parse, parse_amendment, scan};

fn req(id: &str, text: &str) -> Requirement {
    Requirement {
        id: id.to_string(),
        text: text.to_string(),
    }
}

/// A spec whose Requirements section holds `body`, between two other sections.
fn spec(body: &str) -> String {
    format!(
        "# Title\n\n## Design\n\nR9 is not in the section.\n\n## Requirements\n\n{body}\n## Interfaces\n\nR8 neither.\n"
    )
}

#[test]
fn requirements_parse_numbered_lines_to_the_next() {
    let text = spec(
        "Intro text before the first requirement.\n\
         R1 The token expires after one hour.\n\
         Acceptance: a test advances the clock.\n\
         \n\
         R2: Reset links are single use.\n\
         ### A sub-heading ends the text\n\
         this line belongs to no requirement\n\
         R3. Errors are logged\n\
         R30x is not a requirement line, it continues R3\n\
         Rx neither\n",
    );
    let parsed = parse(&text).expect("a numbered list parses");
    assert_eq!(
        parsed,
        vec![
            req(
                "R1",
                "The token expires after one hour.\nAcceptance: a test advances the clock."
            ),
            req("R2", "Reset links are single use."),
            req(
                "R3",
                "Errors are logged\nR30x is not a requirement line, it continues R3\nRx neither"
            ),
        ]
    );
}

#[test]
fn a_requirement_line_inside_a_code_block_neither_starts_nor_ends_one() {
    let text = spec("R1 First.\n```\nR2 inside a fence\n## not a heading\n```\nR2 Second.\n");
    let parsed = parse(&text).expect("parses");
    assert_eq!(
        parsed,
        vec![
            req(
                "R1",
                "First.\n```\nR2 inside a fence\n## not a heading\n```"
            ),
            req("R2", "Second."),
        ]
    );
}

#[test]
fn a_gap_or_a_repeat_is_refused_exactly() {
    assert_eq!(
        parse(&spec("R1 a\nR2 b\nR4 c\n")),
        Err("requirements must be numbered R1 to R3 without gaps; found R1, R2, R4".to_string())
    );
    assert_eq!(
        parse(&spec("R1 a\nR2 b\nR2 c\n")),
        Err("requirements must be numbered R1 to R3 without gaps; found R1, R2, R2".to_string())
    );
    assert_eq!(
        parse(&spec("R2 a\nR3 b\n")),
        Err("requirements must be numbered R1 to R2 without gaps; found R2, R3".to_string())
    );
    assert_eq!(
        parse(&spec("R2 a\nR1 b\n")),
        Err("requirements must be numbered R1 to R2 without gaps; found R2, R1".to_string())
    );
    assert_eq!(
        parse(&spec("R0 a\n")),
        Err("requirements must be numbered R1 to R1 without gaps; found R0".to_string())
    );
    // A number too large for u32 is shown as written and refused, never wrapped.
    assert_eq!(
        parse(&spec("R1 a\nR99999999999999999999 b\n")),
        Err(
            "requirements must be numbered R1 to R2 without gaps; found R1, R99999999999999999999"
                .to_string()
        )
    );
}

/// Ruling T4-1: a spec without a single R-line in its Requirements section.
#[test]
fn a_spec_without_requirements_is_refused_exactly() {
    let refused = Err("the spec has no requirements; write them as lines R1, R2, …".to_string());
    assert_eq!(parse(&spec("prose only\n")), refused);
    assert_eq!(parse("# Title\n\n## Design\n\nR1 outside\n"), refused);
    assert_eq!(parse(&spec("```\nR1 fenced\n```\n")), refused);
}

/// Ruling T4-2: ids are kept as written, never normalised, so `R01` is not `R1`.
#[test]
fn ids_are_kept_as_written_and_a_leading_zero_fails_the_numbering() {
    assert_eq!(
        parse(&spec("R01 a\nR2 b\n")),
        Err("requirements must be numbered R1 to R2 without gaps; found R01, R2".to_string())
    );
    assert_eq!(
        parse_amendment(
            "## Requirements\n\nR02 changed\n",
            &[req("R1", "a"), req("R2", "b")]
        ),
        Err("the amendment must continue from R3".to_string())
    );
    assert_eq!(
        parse_amendment(
            "## Requirements\n\nR03 new\n",
            &[req("R1", "a"), req("R2", "b")]
        ),
        Err("the amendment must continue from R3".to_string())
    );
    assert_eq!(scan(&spec("R01 a\n")), vec![req("R01", "a")]);
}

/// Review m6: two-digit ids, an indented R-line and CRLF line ends.
#[test]
fn ten_requirements_an_indented_line_and_crlf() {
    let ten: String = (1..=10).map(|n| format!("R{n} text {n}\n")).collect();
    let parsed = parse(&spec(&ten)).expect("R1 to R10 parse");
    assert_eq!(parsed.len(), 10);
    assert_eq!(parsed[9], req("R10", "text 10"));
    assert_eq!(
        parse(&spec("R1 a\nR10 b\n")),
        Err("requirements must be numbered R1 to R2 without gaps; found R1, R10".to_string())
    );
    // An indented R-line does not match `^R(\d+)\b`: it continues the one before.
    assert_eq!(
        parse(&spec("R1 a\n  R2 indented\n")),
        Ok(vec![req("R1", "a\n  R2 indented")])
    );
    let crlf = spec("R1 a\nmore of a\nR2 b\n").replace('\n', "\r\n");
    assert_eq!(
        parse(&crlf),
        Ok(vec![req("R1", "a\nmore of a"), req("R2", "b")])
    );
}

#[test]
fn requirement_text_is_capped_at_2_kib() {
    // Multi-byte characters straddle the cap: the cut lands on a char boundary.
    let long = "é".repeat(TEXT_CAP);
    let parsed = parse(&spec(&format!("R1 {long}\nR2 short\n"))).expect("parses");
    assert_eq!(TEXT_CAP, 2 * 1024);
    assert_eq!(parsed[0].text.len(), TEXT_CAP);
    assert!(parsed[0].text.chars().all(|c| c == 'é'));
    assert_eq!(parsed[1], req("R2", "short"));

    let odd = format!("a{}", "é".repeat(TEXT_CAP));
    let parsed = parse(&spec(&format!("R1 {odd}\n"))).expect("parses");
    assert_eq!(
        parsed[0].text.len(),
        TEXT_CAP - 1,
        "never cuts inside a char"
    );
}

#[test]
fn an_amendment_continues_the_numbering() {
    let approved = vec![req("R1", "a"), req("R2", "b"), req("R3", "c")];
    let ok = "## Requirements\n\nR2 b, now stricter (changed in round 2)\nR4 new one\nR5 another\n";
    assert_eq!(
        parse_amendment(ok, &approved),
        Ok(vec![
            req("R2", "b, now stricter (changed in round 2)"),
            req("R4", "new one"),
            req("R5", "another"),
        ])
    );
    let refused = "the amendment must continue from R4".to_string();
    for bad in [
        "## Requirements\n\nR5 skips R4\n",
        "## Requirements\n\nR4 a\nR6 gap\n",
        "## Requirements\n\nR4 a\nR4 repeat\n",
        "## Requirements\n\nR5 a\nR4 out of order\n",
        "## Requirements\n\nR2 changed\nR2 changed twice\n",
        "## Requirements\n\nR0 zero\n",
        "## Requirements\n\nno requirement\n",
    ] {
        assert_eq!(
            parse_amendment(bad, &approved),
            Err(refused.clone()),
            "{bad}"
        );
    }
    // An approved spec without requirements continues from R1.
    assert_eq!(
        parse_amendment("## Requirements\n\nR2 a\n", &[]),
        Err("the amendment must continue from R1".to_string())
    );
}

#[test]
fn scan_is_lenient_and_keeps_the_first_of_a_repeat() {
    let text = spec("R3 c\nR1 a\nR3 again\n");
    assert_eq!(scan(&text), vec![req("R3", "c"), req("R1", "a")]);
}

/// Decision 12 (task M9.6.10): the Goal section the approved spec's requirements travel
/// with, trimmed, outside code, and cut to 8 KiB on a character.
#[test]
fn the_goal_section_is_read_trimmed_and_capped() {
    let text = "# T\n\n## Goal and success criteria\n\n  Users reset.\nDone when mailed.\n\n## Non-goals\nSSO.\n";
    assert_eq!(goal_section(text), "Users reset.\nDone when mailed.");
    assert_eq!(goal_section("# T\n\n## Design\nx\n"), "");
    let long = format!(
        "## Goal and success criteria\n{}é\n",
        "a".repeat(GOAL_CAP - 1)
    );
    let cut = goal_section(&long);
    assert_eq!(cut.len(), GOAL_CAP - 1, "the é would cross the cap");
}
