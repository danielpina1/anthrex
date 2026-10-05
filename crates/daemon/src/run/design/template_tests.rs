//! Task M9.6.4: DF §3.3, §3.4 and §4.1's templates, checked mechanically, with the exact
//! refusal texts of "Messages (exact)"; and Review focus 4's hostile text.

use proto::DocKind;

use super::{TemplateCtx, admit, cap_bytes, check, clean};

pub(crate) const DRAFT: &str = "\
## Understanding
Reset passwords by email.

## Assumptions
- (assumed) mail is configured

## Constraints found
- crates/auth/src/lib.rs:12 has no token store

## Approaches
### 1. Signed tokens
Stateless.
### 2. Stored tokens
A table.

## Recommendation
Signed tokens.

## Questions for you
- How long should a link live?
";

pub(crate) const REPORT: &str = "\
## Where they agree
Both want tokens.

## Where they disagree
claude: stateless. codex: stored. Judgment: stored.

## Approaches
### 1. Signed tokens [claude]
Stateless.
### 2. Stored tokens [both]
A table.

## Recommendation
Go with Stored tokens, because links must be revocable.

## Questions for you
- How long should a link live?
";

pub(crate) const SPEC: &str = "\
# Password reset

## Goal and success criteria
Users reset their password.

## Non-goals
SSO.

## Approach
Stored tokens.

## Design
A table of tokens.

## Requirements
R1 Tokens expire after an hour. Check: a clock test.
R2 Links are single use. Check: a reuse test.

## Interfaces
`reset(token)`.

## Errors and edge cases
Expired tokens.

## Testing
```
cargo test # TODO inside a code block is fine
```

## Risks
Mail delays.

## Open questions
";

pub(crate) fn two() -> TemplateCtx {
    TemplateCtx {
        labels: vec!["claude".into(), "codex".into()],
        failed: None,
        ready: false,
    }
}

fn without(text: &str, heading: &str) -> String {
    text.lines()
        .filter(|l| *l != heading)
        .map(|l| format!("{l}\n"))
        .collect()
}

#[test]
fn the_fixtures_pass() {
    assert_eq!(check(DocKind::BrainstormDraft, DRAFT, &two()), Ok(()));
    assert_eq!(check(DocKind::Brainstorm, REPORT, &two()), Ok(()));
    assert_eq!(check(DocKind::Spec, SPEC, &two()), Ok(()));
    let ready = TemplateCtx {
        ready: true,
        ..two()
    };
    assert_eq!(check(DocKind::Spec, SPEC, &ready), Ok(()));
}

#[test]
fn each_template_section_is_required() {
    let cases: [(DocKind, &str, &str, &[&str]); 3] = [
        (
            DocKind::BrainstormDraft,
            DRAFT,
            "brainstorm draft",
            &[
                "## Understanding",
                "## Assumptions",
                "## Constraints found",
                "## Approaches",
                "## Recommendation",
                "## Questions for you",
            ],
        ),
        (
            DocKind::Brainstorm,
            REPORT,
            "brainstorm",
            &[
                "## Where they agree",
                "## Where they disagree",
                "## Approaches",
                "## Recommendation",
                "## Questions for you",
            ],
        ),
        (
            DocKind::Spec,
            SPEC,
            "spec",
            &[
                "## Goal and success criteria",
                "## Non-goals",
                "## Approach",
                "## Design",
                "## Requirements",
                "## Interfaces",
                "## Errors and edge cases",
                "## Testing",
                "## Risks",
                "## Open questions",
            ],
        ),
    ];
    for (kind, text, name, headings) in cases {
        for heading in headings {
            let text = without(text, heading);
            // Without the Approaches heading the report's approaches are gone too, but
            // the missing section is named first.
            assert_eq!(
                check(kind, &text, &two()),
                Err(format!("the {name} is missing the section \"{heading}\"")),
                "{name} without {heading}"
            );
        }
    }
    let untitled = SPEC.replacen("# Password reset\n", "", 1);
    assert_eq!(
        check(DocKind::Spec, &untitled, &two()),
        Err("the spec is missing the section \"# <title>\"".to_string())
    );
    let empty_title = SPEC.replacen("# Password reset\n", "#  \n", 1);
    assert_eq!(
        check(DocKind::Spec, &empty_title, &two()),
        Err("the spec is missing the section \"# <title>\"".to_string())
    );
}

#[test]
fn a_heading_inside_a_code_block_is_not_a_section() {
    let fenced = SPEC.replace("## Risks\n", "```\n## Risks\n```\n");
    assert_eq!(
        check(DocKind::Spec, &fenced, &two()),
        Err("the spec is missing the section \"## Risks\"".to_string())
    );
}

#[test]
fn caps_are_enforced() {
    for (kind, kib, name, base) in [
        (DocKind::BrainstormDraft, 12, "brainstorm draft", DRAFT),
        (DocKind::Brainstorm, 32, "brainstorm", REPORT),
        (DocKind::Spec, 64, "spec", SPEC),
    ] {
        assert_eq!(cap_bytes(kind), kib * 1024);
        // Exactly at the cap passes; one byte over is refused.
        let pad = kib * 1024 - base.len();
        let at = format!("{base}{}", "x".repeat(pad));
        assert_eq!(at.len(), kib * 1024);
        assert_eq!(
            admit(kind, &at, &two()),
            Ok(at.clone()),
            "{name} at its cap"
        );
        let over = format!("{at}x");
        let refused = format!("the {name} is over its {kib} KiB cap");
        assert_eq!(admit(kind, &over, &two()), Err(refused.clone()));
        assert_eq!(check(kind, &over, &two()), Err(refused.clone()));
        assert_eq!(clean(kind, &over), Err(refused));
    }
}

#[test]
fn tbd_outside_code_blocks_is_refused_and_inside_is_not() {
    // The fixture already has a TODO inside a fenced block, and passes.
    let line = SPEC.lines().position(|l| l == "Stored tokens.").unwrap() + 1;
    let tbd = SPEC.replace("Stored tokens.\n", "Stored tokens, TBD.\n");
    assert_eq!(
        check(DocKind::Spec, &tbd, &two()),
        Err(format!("the spec still says TBD or TODO at line {line}"))
    );
    let todo = SPEC.replace("Stored tokens.\n", "TODO: pick one\n");
    assert_eq!(
        check(DocKind::Spec, &todo, &two()),
        Err(format!("the spec still says TBD or TODO at line {line}"))
    );
    // Inline code, other words and lower case are not placeholders.
    for fine in [
        "Stored tokens, see `TODO` handling.\n",
        "Stored TODOS and TBDs.\n",
        "Stored tokens, todo later.\n",
    ] {
        let text = SPEC.replace("Stored tokens.\n", fine);
        assert_eq!(check(DocKind::Spec, &text, &two()), Ok(()), "{fine}");
    }
    // Tilde fences are code blocks too.
    let tilde = SPEC.replace("## Risks\n", "~~~\nTBD\n~~~\n## Risks\n");
    assert_eq!(check(DocKind::Spec, &tilde, &two()), Ok(()));
}

#[test]
fn open_questions_must_be_empty_when_ready() {
    let asked = format!("{SPEC}- Should links be emailed twice?\n");
    assert_eq!(check(DocKind::Spec, &asked, &two()), Ok(()), "not ready");
    let ready = TemplateCtx {
        ready: true,
        ..two()
    };
    assert_eq!(
        check(DocKind::Spec, &asked, &ready),
        Err("Open questions must be empty when ready is true".to_string())
    );
    // Blank lines are empty; the next section ends it.
    let blank = SPEC.replace("## Open questions\n", "## Open questions\n\n   \n");
    assert_eq!(check(DocKind::Spec, &blank, &ready), Ok(()));
    let then = format!("{SPEC}\n## Appendix\nNotes.\n");
    assert_eq!(check(DocKind::Spec, &then, &ready), Ok(()));
}

#[test]
fn brainstorm_tags_and_recommendation_are_checked() {
    let untagged = REPORT.replace("### 1. Signed tokens [claude]", "### 1. Signed tokens");
    assert_eq!(
        check(DocKind::Brainstorm, &untagged, &two()),
        Err("approach \"1. Signed tokens\" has no [claude], [codex] or [both] tag".to_string())
    );
    // A tag naming no brainstormer of this run is no tag.
    let alien = REPORT.replace("[claude]", "[gemini]");
    assert_eq!(
        check(DocKind::Brainstorm, &alien, &two()),
        Err(
            "approach \"1. Signed tokens [gemini]\" has no [claude], [codex] or [both] tag"
                .to_string()
        )
    );
    // One runtime: the lenses A and B.
    let lenses = TemplateCtx {
        labels: vec!["A".into(), "B".into()],
        ..two()
    };
    let ab = REPORT.replace("[claude]", "[A]");
    assert_eq!(check(DocKind::Brainstorm, &ab, &lenses), Ok(()));
    assert_eq!(
        check(DocKind::Brainstorm, REPORT, &lenses),
        Err("approach \"1. Signed tokens [claude]\" has no [A], [B] or [both] tag".to_string())
    );

    let unnamed = REPORT.replace(
        "Go with Stored tokens, because links must be revocable.",
        "Go with something else.",
    );
    let refused = Err("the recommendation must name one of the listed approaches".to_string());
    assert_eq!(check(DocKind::Brainstorm, &unnamed, &two()), refused);
    // Named without its number, in another case: still named.
    let lower = REPORT.replace("Go with Stored tokens", "go with stored TOKENS");
    assert_eq!(check(DocKind::Brainstorm, &lower, &two()), Ok(()));
    // No approach headings at all: nothing to name.
    let none = REPORT
        .replace("### 1. Signed tokens [claude]\n", "")
        .replace("### 2. Stored tokens [both]\n", "");
    assert_eq!(check(DocKind::Brainstorm, &none, &two()), refused);
}

#[test]
fn a_single_brainstorm_line_is_required_when_one_failed() {
    let single = TemplateCtx {
        failed: Some(("codex".into(), "launch failed".into())),
        ..two()
    };
    assert_eq!(
        check(DocKind::Brainstorm, REPORT, &single),
        Err(
            "a single brainstorm must begin \"single brainstorm: codex failed: launch failed\""
                .to_string()
        )
    );
    let begun = format!("\nsingle brainstorm: codex failed: it crashed on start\n\n{REPORT}");
    assert_eq!(check(DocKind::Brainstorm, &begun, &single), Ok(()));
    // The line must come first, and must name the failed label.
    let late = format!("{REPORT}\nsingle brainstorm: codex failed: launch failed\n");
    assert!(check(DocKind::Brainstorm, &late, &single).is_err());
    let wrong = format!("single brainstorm: claude failed: launch failed\n{REPORT}");
    assert!(check(DocKind::Brainstorm, &wrong, &single).is_err());
    // Without a failure the line is not required.
    assert_eq!(check(DocKind::Brainstorm, REPORT, &two()), Ok(()));
}

#[test]
fn hostile_text_is_cleaned_and_capped_before_the_check() {
    // Control characters, an ESC sequence, CRLF line ends, a hidden format character.
    let hostile = SPEC
        .replace(
            "Users reset their password.",
            "Users\x07 reset \x1b[2J\x1b]0;pwn\x07their\u{200b} password.\x00",
        )
        .replace('\n', "\r\n");
    let cleaned = admit(DocKind::Spec, &hostile, &two()).expect("clean text passes");
    assert!(
        !cleaned.chars().any(|c| c.is_control() && c != '\n'),
        "no control character but newline survives: {cleaned:?}"
    );
    assert!(!cleaned.contains('\r') && !cleaned.contains('\u{200b}'));
    assert!(cleaned.contains("Users  reset  [2J ]0;pwn their password. \n"));
    assert_eq!(
        clean(DocKind::Spec, &hostile).as_deref(),
        Ok(cleaned.as_str())
    );

    // A CR-only document still has its sections once cleaned.
    let cr = SPEC.replace('\n', "\r");
    assert_eq!(admit(DocKind::Spec, &cr, &two()), Ok(SPEC.to_string()));

    // A 10 MB text is refused by its raw size, for every kind.
    let huge = "\x1b".repeat(10 * 1024 * 1024);
    for (kind, kib, name) in [
        (DocKind::BrainstormDraft, 12, "brainstorm draft"),
        (DocKind::Brainstorm, 32, "brainstorm"),
        (DocKind::Spec, 64, "spec"),
    ] {
        assert_eq!(
            admit(kind, &huge, &two()),
            Err(format!("the {name} is over its {kib} KiB cap"))
        );
    }
}
