//! Milestone 9.17 decision 12: how the inspector writes a `brief` finding, and that
//! `most_severe` (what is being fixed) never picks one. Pure.

use proto::{FindingTarget, Severity};

use super::{finding_text, most_severe};
use crate::tree::run_fixtures::finding;

fn brief(severity: Severity, text: &str) -> proto::Finding {
    proto::Finding {
        target: FindingTarget::Brief,
        ..finding(severity, "crates/a/src/x.rs", 3, text)
    }
}

#[test]
fn brief_finding_text_and_most_severe_skips_it() {
    let wrong = brief(Severity::Critical, "the brief is wrong");
    assert_eq!(finding_text(&wrong), "brief critical — the brief is wrong");
    let code = finding(Severity::Minor, "crates/a/src/y.rs", 9, "a nit");
    assert_eq!(finding_text(&code), "y.rs:9 minor — a nit");
    let findings = vec![wrong.clone(), code.clone()];
    assert_eq!(most_severe(&findings), Some(&code));
    assert_eq!(most_severe(&[wrong]), None);
}
