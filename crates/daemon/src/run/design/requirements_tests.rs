//! Task M9.6.4: decision 14's requirement parsing, with the exact refusal texts.

use super::{Requirement, TEXT_CAP, parse, parse_amendment, scan};

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
    // No requirement at all.
    assert_eq!(
        parse(&spec("prose only\n")),
        Err("requirements must be numbered R1 to R1 without gaps; found none".to_string())
    );
    assert_eq!(
        parse("# Title\n\n## Design\n\nR1 outside\n"),
        Err("requirements must be numbered R1 to R1 without gaps; found none".to_string())
    );
}

#[test]
fn leading_zeros_are_read_as_their_number() {
    assert_eq!(
        parse(&spec("R01 a\nR2 b\n")),
        Ok(vec![req("R1", "a"), req("R2", "b")])
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
