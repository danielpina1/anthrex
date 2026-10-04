//! Task M9.6.5: a run's design fields in its snapshot (`snapshot_design.rs`). A run
//! without the design flow keeps 9.5's snapshot byte for byte; a design run shows its
//! mode, its open gate and its documents, and a task its `covers`.

use proto::{DesignMode, DocAuthor, DocFinding, DocGateInfo, DocGateKind, DocKind, DocSeverity};

use super::fixture::*;
use super::race::racing;
use crate::run::design::state::{DesignState, DocGate, NewDoc, store};
use crate::run::snapshot::snapshot;

/// The RunInfo milestone 9.5's code wrote for `racing()`'s run, captured before any 9.6
/// change (commit 3deaafef).
const M95_RUN_INFO: &str = include_str!("../../../../../proto/src/m95_run_info.json");

#[test]
fn a_non_design_run_has_no_design_state_and_an_unchanged_snapshot() {
    let (fx, _, _) = racing();
    assert_eq!(fx.run().design_mode, DesignMode::Off);
    assert_eq!(fx.run().orch.design, None);
    let snap = snapshot(&fx.state, fx.now);
    let info = &snap.runs[0];
    assert_eq!((info.design, &info.doc_gate), (DesignMode::Off, &None));
    assert!(info.docs.is_empty());
    let written = serde_json::to_string_pretty(info).expect("a RunInfo is JSON");
    assert_eq!(written.trim_end(), M95_RUN_INFO.trim_end());
}

#[test]
fn a_design_run_shows_its_mode_gate_and_documents() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "M", "a", "")]));
    fx.ready(false);
    fx.task_mut("t1").spec.covers = vec!["R1".into(), "R2".into()];
    let disputed = DocFinding {
        id: "F2".into(),
        severity: DocSeverity::Minor,
        place: "## Risks".into(),
        text: "Name a risk.".into(),
    };
    let run = fx.run_mut();
    run.design_mode = DesignMode::Full;
    run.orch.design = Some(DesignState::default());
    let brainstorm = NewDoc::new(DocKind::Brainstorm, DocAuthor::Orchestrator, "merged", "b");
    store(run, brainstorm, 2_000).unwrap();
    let spec = "# S\n\n## Requirements\nR1 one\nR2 two\n";
    let spec = NewDoc {
        disputed: vec![disputed.clone()],
        not_reviewed: None,
        changes: vec!["+ R2".into()],
        same_runtime: true,
        ..NewDoc::new(DocKind::Spec, DocAuthor::User, "your edit", spec)
    };
    store(run, spec, 2_001).unwrap();
    run.orch.design.as_mut().unwrap().gate = Some(DocGate {
        kind: DocGateKind::Spec,
        version: 1,
        opened_at: 2_001,
        revising: Some("tighter".into()),
        review: false,
    });

    let snap = snapshot(&fx.state, fx.now);
    let info = &snap.runs[0];
    assert_eq!(info.design, DesignMode::Full);
    assert_eq!(
        info.doc_gate,
        Some(DocGateInfo {
            kind: DocGateKind::Spec,
            version: 1,
            revising: Some("tighter".into()),
            disputed: vec![disputed],
            not_reviewed: None,
            changes_summary: vec!["+ R2".into()],
            same_runtime: true,
        })
    );
    let docs: Vec<_> = (info.docs.iter())
        .map(|d| (d.kind, d.version, d.author.clone(), d.requirements.clone()))
        .collect();
    assert_eq!(
        docs,
        [
            (DocKind::Brainstorm, 1, DocAuthor::Orchestrator, vec![]),
            (
                DocKind::Spec,
                1,
                DocAuthor::User,
                vec!["R1".to_string(), "R2".to_string()]
            ),
        ]
    );
    assert_eq!(info.docs[1].reason, "your edit");
    assert_eq!(info.tasks[0].covers, ["R1", "R2"]);

    // Its JSON names them; a gate whose version is missing shows the gate alone.
    let json = serde_json::to_value(info).unwrap();
    assert_eq!(json["design"], "full");
    assert_eq!(json["tasks"][0]["covers"][1], "R2");
    let design = fx.run_mut().orch.design.as_mut().unwrap();
    design.gate.as_mut().unwrap().version = 9;
    let snap = snapshot(&fx.state, fx.now);
    let gate = snap.runs[0].doc_gate.clone().unwrap();
    assert_eq!(
        (gate.version, gate.disputed.len(), gate.same_runtime),
        (9, 0, false)
    );
}
