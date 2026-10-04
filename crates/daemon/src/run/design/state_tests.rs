//! Task M9.6.5: the design state's pure half: numbering, file names, lookups and the
//! write `store` asks for. The files themselves are `driver/design_io_tests.rs`'.

use proto::{DocAuthor, DocKind};

use super::{DesignState, NewDoc, VERSIONS_FILE, store, store_findings};
use crate::run::engine::Effect;
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

fn run(design: bool) -> crate::run::model::Run {
    let mut run = run_ok(&plan_with(PROFILE, &[task_toml("t1", "S", "[\"a\"]", "")]));
    if design {
        run.orch.design = Some(DesignState::default());
    }
    run
}

fn draft(label: &str) -> NewDoc {
    let author = DocAuthor::Brainstormer {
        label: label.into(),
    };
    NewDoc::new(DocKind::BrainstormDraft, author, "draft", "text")
}

#[test]
fn store_records_the_version_and_asks_for_its_write() {
    let mut run = run(true);
    let text = "# T\n\n## Requirements\nR1 one\nR2 two\n";
    let doc = NewDoc::new(DocKind::Spec, DocAuthor::User, "your edit", text);
    let (version, effect) = store(&mut run, doc, 7).unwrap();
    assert_eq!(
        (version.n, version.at, version.bytes),
        (1, 7, text.len() as u64)
    );
    assert_eq!(version.requirements, ["R1", "R2"]);
    assert_eq!(version.sha256.len(), 64);
    let design = run.orch.design.as_ref().unwrap();
    assert_eq!(design.versions, [version]);
    let dir = run.data_dir.join("design");
    let Effect::WriteDoc {
        path,
        text: t,
        index,
    } = effect
    else {
        panic!("{effect:?}")
    };
    assert_eq!((path, t.as_str()), (dir.join("spec-v1.md"), text));
    let (index, json) = index.expect("the index is rewritten");
    assert_eq!(index, dir.join(VERSIONS_FILE));
    assert_eq!(
        serde_json::from_str::<Vec<super::DocVersion>>(&json).unwrap(),
        design.versions
    );
}

#[test]
fn store_refuses_a_run_without_the_design_flow_and_an_unnamed_draft() {
    let mut plain = run(false);
    let doc = NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, "r", "t");
    let refusal = store(&mut plain, doc, 1).unwrap_err();
    assert_eq!(
        refusal,
        format!("run {} does not use the design flow", plain.id)
    );
    assert!(
        plain.orch.design.is_none(),
        "a plain run stays without design state"
    );

    let mut run = run(true);
    let anonymous = NewDoc::new(DocKind::BrainstormDraft, DocAuthor::Orchestrator, "r", "t");
    // Fix round 1, m6: only the four labels the daemon gives, exactly.
    let others = ["../x", "", "a b", "C", "Claude", "claude2", "a", "codex-1"].map(draft);
    for doc in std::iter::once(anonymous).chain(others) {
        let refusal = store(&mut run, doc, 1).unwrap_err();
        assert_eq!(refusal, "a brainstorm draft needs its brainstormer's label");
    }
    assert!(run.orch.design.as_ref().unwrap().versions.is_empty());
    for label in ["claude", "codex", "A", "B"] {
        store(&mut run, draft(label), 1).unwrap();
    }
}

#[test]
fn versions_are_found_by_number_latest_label_and_previous() {
    let mut run = run(true);
    for label in ["A", "B", "A"] {
        store(&mut run, draft(label), 1).unwrap();
    }
    for text in ["one", "two"] {
        let doc = NewDoc::new(DocKind::Brainstorm, DocAuthor::Orchestrator, "r", text);
        store(&mut run, doc, 1).unwrap();
    }
    let design = run.orch.design.as_ref().unwrap();
    assert_eq!(design.find(DocKind::Brainstorm, None).unwrap().n, 2);
    assert_eq!(design.find(DocKind::Brainstorm, Some(1)).unwrap().n, 1);
    assert!(design.find(DocKind::Brainstorm, Some(3)).is_none());
    assert!(design.find(DocKind::Spec, None).is_none());
    assert_eq!(design.next_n(DocKind::Brainstorm), 3);
    assert_eq!(design.next_n(DocKind::Plan), 1);

    let a = design.draft_from("A").unwrap();
    assert_eq!(a.n, 3);
    assert!(design.draft_from("C").is_none());
    // A draft's previous is the same brainstormer's; the report's, its v1.
    assert_eq!(design.previous(a).unwrap().n, 1);
    let b = design.draft_from("B").unwrap();
    assert!(design.previous(b).is_none());
    let v2 = design.find(DocKind::Brainstorm, Some(2)).unwrap();
    assert_eq!(design.previous(v2).unwrap().n, 1);
    assert_eq!(design.file_name(v2), "brainstorm-v2.md");
}

#[test]
fn findings_are_written_beside_their_version_only() {
    let mut run = run(true);
    let doc = NewDoc::new(DocKind::Plan, DocAuthor::Engine, "rendered", "# Plan");
    store(&mut run, doc, 1).unwrap();
    let effect = store_findings(&run, DocKind::Plan, 1, &[]).unwrap();
    let Effect::WriteDoc { path, text, index } = effect else {
        panic!("{effect:?}")
    };
    assert_eq!(path, run.data_dir.join("design/findings-plan-v1.json"));
    assert_eq!((text.as_str(), index), ("[]", None));
    assert_eq!(
        store_findings(&run, DocKind::Plan, 2, &[]).unwrap_err(),
        format!("run {} has no plan v2", run.id)
    );
    // Fix round 1, m7: the kind as a refusal names it.
    assert_eq!(
        store_findings(&run, DocKind::BrainstormDraft, 1, &[]).unwrap_err(),
        format!("run {} has no brainstorm draft v1", run.id)
    );
}

/// Ruling T5-1's seam (task M9.6.6): a spec's review drafts are stored with `n = 0`
/// (task M9.6.10), so `get_doc { kind: "spec" }` with no version reads the latest gate
/// version, or, before there is one, the latest draft.
#[test]
fn the_latest_spec_is_the_last_gate_version_else_the_last_draft() {
    let mut run = run(true);
    let mut stored = |text: &str, n: u32| {
        let doc = NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, "r", text);
        store(&mut run, doc, 1).unwrap();
        let design = run.orch.design.as_mut().unwrap();
        design.versions.last_mut().unwrap().n = n;
        let latest = design.find(DocKind::Spec, None).unwrap();
        (latest.n, latest.bytes)
    };
    assert_eq!(stored("d1", 0), (0, 2));
    assert_eq!(stored("draft 2", 0), (0, 7), "the last draft");
    assert_eq!(stored("gate v1", 1), (1, 7));
    assert_eq!(
        stored("d3", 0),
        (1, 7),
        "a gate version wins over a later draft"
    );
    assert_eq!(stored("the gate v2", 2), (2, 11));
}

/// The driver's caller check (task M9.6.6): a design agent of `role` whose session is
/// running in `window`; not one that ended, nor another role's.
#[test]
fn a_live_agent_runs_in_its_window_in_its_role() {
    use super::{DesignAgent, DesignAgentState};
    use proto::{AgentRole, Effort, Route, Runtime, Strength};
    let agent = |role, window, state| DesignAgent {
        label: "x".into(),
        role,
        route: Route {
            runtime: Runtime::Claude,
            model: "m".into(),
            strength: Strength::Frontier,
            effort: Effort::High,
        },
        session: 1,
        window_id: Some(window),
        state,
        calls: 0,
        tokens: 0,
        started: None,
    };
    let design = DesignState {
        brainstormers: vec![
            agent(AgentRole::Brainstormer, 7, DesignAgentState::Running),
            agent(AgentRole::Brainstormer, 8, DesignAgentState::Done),
        ],
        reviewer: Some(agent(AgentRole::DocReviewer, 9, DesignAgentState::Running)),
        ..DesignState::default()
    };
    assert!(design.live_agent(AgentRole::Brainstormer, 7).is_some());
    assert!(design.live_agent(AgentRole::Brainstormer, 8).is_none());
    assert!(design.live_agent(AgentRole::DocReviewer, 9).is_some());
    assert!(design.live_agent(AgentRole::DocReviewer, 7).is_none());
    assert!(design.live_agent(AgentRole::Brainstormer, 9).is_none());
}
