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
    for doc in [anonymous, draft("../x"), draft(""), draft("a b")] {
        let refusal = store(&mut run, doc, 1).unwrap_err();
        assert_eq!(refusal, "a brainstorm draft needs its brainstormer's label");
    }
    assert!(run.orch.design.as_ref().unwrap().versions.is_empty());
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
}
