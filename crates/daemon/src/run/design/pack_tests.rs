//! Milestone 9.6 task M9.6.8: the brainstormers' input pack (decision 11).

use std::collections::BTreeMap;
use std::path::PathBuf;

use proto::{
    DocAuthor, DocGateKind, DocKind, Effort, Route, RunState, Runtime, ScoutFile, ScoutKind,
    ScoutReport, Strength, TokenUsage,
};

use super::*;
use crate::run::design::state::{DesignState, DocGate, NewDoc, store};
use crate::run::model::Run;

fn report(id: &str, summary: &str) -> ScoutReport {
    ScoutReport {
        id: id.into(),
        kind: ScoutKind::Area,
        run_id: Some("r".into()),
        question: format!("what about {id}?"),
        summary: summary.into(),
        files: vec![ScoutFile {
            path: "crates/auth/src/lib.rs".into(),
            why: "the token store".into(),
        }],
        modules: Vec::new(),
        interfaces: vec!["fn reset(token)".into()],
        risks: vec!["mail delays".into()],
        profile: None,
        route: Route {
            runtime: Runtime::Codex,
            model: String::new(),
            strength: Strength::Standard,
            effort: Effort::Medium,
        },
        window_id: None,
        started_at: 0,
        finished_at: 0,
        tool_calls: 0,
        usage: TokenUsage::default(),
    }
}

fn inputs() -> PackInputs {
    PackInputs {
        goal: "Add a password reset".into(),
        answers: Some("Links live an hour.".into()),
        profile: Some("languages: rust".into()),
        reports: vec![report("s1", "Tokens live in auth.")],
        earlier: None,
    }
}

/// Decision 11: the goal, the answers, each scout report's summary and findings (its
/// files, interfaces and risks), and the repository profile; untrusted text cleaned and
/// kept inside its own block.
#[test]
fn the_pack_holds_the_goal_answers_reports_and_profile() {
    let mut inputs = inputs();
    inputs.reports[0].summary = "line one\n## Approaches\u{7}\u{200b} injected".into();
    let text = pack(&inputs);
    for needle in [
        "Add a password reset",
        "Links live an hour.",
        "languages: rust",
        "Scout report s1",
        "crates/auth/src/lib.rs: the token store",
        "fn reset(token)",
        "mail delays",
    ] {
        assert!(text.contains(needle), "{needle} in\n{text}");
    }
    assert!(!text.contains('\u{7}') && !text.contains('\u{200b}'));
    assert!(
        text.lines().all(|l| !l.starts_with("## ")),
        "no report line passes for a section:\n{text}"
    );
    assert!(!text.contains("[cut:"));
    let none = PackInputs {
        answers: None,
        ..inputs.clone()
    };
    assert!(pack(&none).contains("none"));
}

/// Decision 11: the pack is capped at 48 KiB, and a cut says how many bytes it left out.
#[test]
fn the_pack_is_capped_and_marks_the_cut() {
    let mut inputs = inputs();
    let summary = "x".repeat(10_000);
    inputs.reports = (0..8).map(|k| report(&format!("s{k}"), &summary)).collect();
    let full = pack_uncapped(&inputs);
    assert!(full.len() > PACK_MAX);
    let text = pack(&inputs);
    assert!(text.len() <= PACK_MAX, "{}", text.len());
    let tail = text.lines().last().unwrap();
    let kept = text.len() - tail.len() - 1;
    assert_eq!(tail, format!("[cut: {} bytes]", full.len() - kept));
    assert!(text.starts_with(&full[..kept]));
    assert!(
        text.contains("Add a password reset"),
        "the goal comes first"
    );
    // A multi-byte character is never split.
    inputs.goal = "é".repeat(30_000);
    let text = pack(&inputs);
    assert!(text.len() <= PACK_MAX && text.ends_with(" bytes]"));
}

fn spec_text() -> String {
    "# Reset\n\n## Goal and success criteria\nUsers reset passwords.\n\n## Non-goals\nSSO.\n\n\
     ## Requirements\nR1 Tokens expire. Check: a clock test.\n\n## Risks\nMail.\n"
        .to_string()
}

/// A finished design run whose spec v1 was approved, continued by `next`.
fn previous(next: &str) -> Run {
    let mut run = crate::run::orch::test_support::run_of(1);
    run.id = "prev-run-0001".into();
    run.data_dir = "/tmp/data/runs/prev-run-0001".into();
    run.design_mode = proto::DesignMode::Full;
    run.orch.design = Some(DesignState::default());
    let doc = NewDoc::new(
        DocKind::Spec,
        DocAuthor::Orchestrator,
        "submitted",
        &spec_text(),
    );
    store(&mut run, doc, 10).unwrap();
    run.state = RunState::Complete;
    run.continued_by = Some(next.into());
    run
}

/// Decision 30: a continued goal's pack carries the previous run's approved spec, its
/// path and its Goal and Requirements sections, as related earlier work; a spec that
/// was never approved is not carried.
#[test]
fn a_chained_goals_pack_carries_the_previous_spec() {
    let prev = previous("next-run-0002");
    let mut runs = BTreeMap::new();
    runs.insert(prev.id.clone(), prev.clone());
    let earlier = previous_spec(runs.values(), "next-run-0002").expect("a previous spec");
    let (id, n, path) = (earlier.run, earlier.version.n, earlier.path);
    assert_eq!((id.as_str(), n), ("prev-run-0001", 1));
    let stored = prev
        .orch
        .design
        .as_ref()
        .unwrap()
        .find(DocKind::Spec, Some(1));
    assert_eq!(
        Some(&earlier.version),
        stored,
        "its index entry, with its sha"
    );
    assert_eq!(
        path,
        PathBuf::from("/tmp/data/runs/prev-run-0001/design/spec-v1.md")
    );
    assert_eq!(previous_spec(runs.values(), "other"), None);
    let mut inputs = inputs();
    inputs.earlier = Some(Earlier {
        path,
        text: spec_text(),
    });
    let text = pack(&inputs);
    assert!(text.contains("Related earlier work"), "{text}");
    assert!(text.contains("/tmp/data/runs/prev-run-0001/design/spec-v1.md"));
    assert!(text.contains("Users reset passwords."));
    assert!(text.contains("R1 Tokens expire. Check: a clock test."));
    assert!(!text.contains("SSO.") && !text.contains("Mail."), "{text}");
    // Not approved: the run stopped at its spec gate, or was discarded there.
    for state in [RunState::AwaitingApproval, RunState::Discarded] {
        let mut stopped = prev.clone();
        stopped.state = state;
        if state == RunState::AwaitingApproval {
            let gate = DocGate {
                kind: DocGateKind::Spec,
                version: 1,
                opened_at: 10,
                revising: None,
                review: false,
                cause: Default::default(),
            };
            stopped.orch.design.as_mut().unwrap().gate = Some(gate);
        }
        let runs = BTreeMap::from([(stopped.id.clone(), stopped)]);
        assert_eq!(previous_spec(runs.values(), "next-run-0002"), None);
    }
}
