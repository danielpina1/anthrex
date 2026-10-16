//! Milestone 9.17 decision 6: `parse_review` reads a finding's `target`; a `brief`
//! finding needs no location, a `code` one still does. Pure.

use proto::{Finding, FindingTarget, Severity, Verdict};
use serde_json::{Value, json};

use super::parse_review;

fn review(finding: Value) -> Value {
    json!({"verdict": "changes", "summary": "s", "findings": [finding]})
}

#[test]
fn brief_finding_needs_no_location() {
    let args =
        review(json!({"severity": "critical", "target": "brief", "text": "the brief is wrong"}));
    let (verdict, _, findings) = parse_review(&args).expect("accepted");
    assert_eq!(verdict, Verdict::Changes);
    assert_eq!(
        findings,
        vec![Finding {
            severity: Severity::Critical,
            file: None,
            line: None,
            input: None,
            text: "the brief is wrong".into(),
            target: FindingTarget::Brief,
        }]
    );
}

#[test]
fn code_finding_still_needs_one() {
    let located = ": a critical or important finding needs file and line, or input";
    for finding in [
        json!({"severity": "important", "target": "code", "text": "t"}),
        json!({"severity": "important", "text": "t"}),
    ] {
        assert_eq!(
            parse_review(&review(finding.clone())).unwrap_err(),
            format!("findings[0]{located}"),
            "{finding}"
        );
    }
    let args =
        review(json!({"severity": "important", "target": "code", "input": "x", "text": "t"}));
    let (_, _, findings) = parse_review(&args).expect("accepted");
    assert_eq!(findings[0].target, FindingTarget::Code);
}

#[test]
fn unknown_target_is_refused() {
    for target in [json!("plan"), json!(1), json!(null)] {
        let finding = json!({"severity": "minor", "target": target, "text": "t"});
        assert_eq!(
            parse_review(&review(finding)).unwrap_err(),
            "findings[0].target: must be one of code, brief"
        );
    }
}
