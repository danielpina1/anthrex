//! Milestone 9.6 task M9.6.8: the input pack the driver reads off the engine for a
//! brainstormer's first turn (decision 11): the stored scout reports, and a continued
//! goal's previous approved spec read through its index's check (decision 30). Real
//! files under the read rig's data dir; no agent.

use proto::{
    DocAuthor, DocKind, Effort, Route, RunState, Runtime, ScoutFile, ScoutKind, ScoutReport,
    Strength, TokenUsage,
};

use super::read_rig::Rig;
use crate::run::design::pack::{freeze, previous_spec};
use crate::run::design::state::{self, DesignState, NewDoc};
use crate::run::driver::design_io::write_new;
use crate::run::engine::Effect;
use crate::run::model::Run;

const SPEC: &str = "# Reset\n\n## Goal and success criteria\nUsers reset passwords.\n\n\
                    ## Requirements\nR1 Tokens expire. Check: a clock test.\n\n## Risks\nMail.\n";

fn report(id: &str) -> ScoutReport {
    ScoutReport {
        id: id.into(),
        kind: ScoutKind::Area,
        run_id: None,
        question: "Where are tokens kept?".into(),
        summary: "In the auth crate.".into(),
        files: vec![ScoutFile {
            path: "crates/auth/src/lib.rs".into(),
            why: "the store".into(),
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

/// A design run with the user's answers and one stored run scout report.
fn brainstorming(run: &mut Run, dir: &std::path::Path) {
    run.design_mode = proto::DesignMode::Full;
    run.state = RunState::Brainstorming;
    run.repo_dir = dir.join("repo-data");
    std::fs::create_dir_all(&run.repo_dir).unwrap();
    run.scout_reports = vec!["s1".into()];
    run.orch.design = Some(DesignState {
        answers: Some("Links live an hour.".into()),
        pack: Some(freeze(run, None)),
        ..DesignState::default()
    });
    let scouts = run.data_dir.join("scouts");
    std::fs::create_dir_all(&scouts).unwrap();
    let text = serde_json::to_string(&report("s1")).unwrap();
    std::fs::write(scouts.join("s1.json"), text).unwrap();
}

/// Ruling T8-2: the previous spec frozen into the run's pack, as the engine freezes it
/// when the brainstormers are queued.
fn freeze_previous(rig: &Rig) {
    let mut engine = crate::lock(&rig.runs.state);
    let earlier = previous_spec(engine.runs.values(), &rig.run_id);
    let run = engine.runs.get_mut(&rig.run_id).unwrap();
    run.orch.design.as_mut().unwrap().pack = Some(freeze(run, earlier));
}

/// The previous run of a chain, finished, its spec v1 stored and written.
fn previous(rig: &Rig) {
    let mut prev = crate::run::orch::test_support::run_of(1);
    prev.id = "prev-run-0001".into();
    prev.data_dir = {
        let engine = crate::lock(&rig.runs.state);
        engine.runs[&rig.run_id].data_dir.join("prev")
    };
    prev.design_mode = proto::DesignMode::Full;
    prev.orch.design = Some(DesignState::default());
    prev.state = RunState::Complete;
    prev.continued_by = Some(rig.run_id.clone());
    let doc = NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, "submitted", SPEC);
    let (_, effect) = state::store(&mut prev, doc, 10).unwrap();
    let Effect::WriteDoc { path, text, .. } = effect else {
        panic!("not a write");
    };
    write_new(&path, &text).unwrap();
    crate::lock(&rig.runs.state)
        .runs
        .insert(prev.id.clone(), prev);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pack_reads_the_reports_and_the_previous_spec_off_the_engine() {
    let rig = Rig::new(brainstorming).await;
    let pack = rig.runs.brainstorm_pack(&rig.run_id).await;
    for needle in [
        "Links live an hour.",
        "Scout report s1 (Where are tokens kept?)",
        "crates/auth/src/lib.rs: the store",
        "fn reset(token)",
        "mail delays",
    ] {
        assert!(pack.contains(needle), "{needle} in\n{pack}");
    }
    assert!(!pack.contains("Related earlier work"));
    previous(&rig);
    freeze_previous(&rig);
    let pack = rig.runs.brainstorm_pack(&rig.run_id).await;
    assert!(pack.contains("Related earlier work"), "{pack}");
    assert!(pack.contains("prev/design/spec-v1.md"), "{pack}");
    assert!(pack.contains("R1 Tokens expire. Check: a clock test."));
    assert!(
        !pack.contains("Mail."),
        "only the Goal and Requirements sections"
    );
    // A spec file changed since it was stored is never carried (task 5's I-1).
    let path = {
        let engine = crate::lock(&rig.runs.state);
        let prev = &engine.runs["prev-run-0001"];
        let design = prev.orch.design.as_ref().unwrap();
        let v = design.find(DocKind::Spec, Some(1)).unwrap();
        state::design_dir(prev).join(design.file_name(v))
    };
    std::fs::write(
        &path,
        SPEC.replace("an hour", "a day").replace("expire", "lapse"),
    )
    .unwrap();
    let pack = rig.runs.brainstorm_pack(&rig.run_id).await;
    assert!(!pack.contains("Related earlier work"), "{pack}");
}

/// Ruling T8-2: the pack reads exactly the inputs frozen when the brainstormers were
/// queued: a run scout's report that lands between the two starts, and a previous spec
/// approved since, change nothing; the two packs are byte-equal.
#[tokio::test(flavor = "multi_thread")]
async fn the_pack_reads_exactly_its_frozen_inputs() {
    let rig = Rig::new(brainstorming).await;
    let first = rig.runs.brainstorm_pack(&rig.run_id).await;
    assert!(first.contains("Scout report s1"), "{first}");
    let scouts = {
        let mut engine = crate::lock(&rig.runs.state);
        let run = engine.runs.get_mut(&rig.run_id).unwrap();
        run.scout_reports.push("s2".into());
        run.data_dir.join("scouts")
    };
    let text = serde_json::to_string(&report("s2")).unwrap();
    std::fs::write(scouts.join("s2.json"), text).unwrap();
    previous(&rig);
    let second = rig.runs.brainstorm_pack(&rig.run_id).await;
    assert_eq!(first, second);
    assert!(!second.contains("Scout report s2"));
}
