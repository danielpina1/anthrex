//! Milestone 9.6 task M9.6.6: `get_doc`, answered off the engine by the driver's read
//! path (Interfaces "MCP tools"), from the orchestrator and from the run's document
//! reviewer, and the design agents' caller check. Through a real daemon socket with no
//! agent (`orch_read_rig.rs`); the documents are real files under the rig's data dir.

use proto::{AgentRole, DocAuthor, DocKind, Effort, Route, Runtime, Strength};
use serde_json::json;

use super::read_rig::{ANSWER, Rig};
use crate::run::design::state::{self, DesignAgent, DesignAgentState, DesignState, NewDoc};
use crate::run::driver::design_io::{DOC_READ_CAP, write_new};
use crate::run::engine::Effect;
use crate::run::model::Run;

/// The document reviewer's window and a brainstormer's.
const REVIEWER: u32 = 903;
const BRAINSTORMER: u32 = 904;

fn agent(label: &str, role: AgentRole, window: u32) -> DesignAgent {
    DesignAgent {
        label: label.into(),
        role,
        route: Route {
            runtime: Runtime::Codex,
            model: "gpt-5.5".into(),
            strength: Strength::Frontier,
            effort: Effort::HIGH,
        },
        session: 1,
        window_id: Some(window),
        state: DesignAgentState::Running,
        calls: 0,
        tokens: 0,
        started: Some(1_000),
        listed: false,
        unsubmitted: false,
        round: 1,
    }
}

/// A design run with a live reviewer and brainstormer, paused: the engine refuses every
/// tool call of a paused run, so an answer can only come from the read path.
fn design_run(run: &mut Run, _: &std::path::Path) {
    run.design_mode = proto::DesignMode::Full;
    run.state = proto::RunState::Paused;
    run.orch.design = Some(DesignState {
        reviewer: Some(agent("spec-r1", AgentRole::DocReviewer, REVIEWER)),
        brainstormers: vec![agent("codex", AgentRole::Brainstormer, BRAINSTORMER)],
        ..DesignState::default()
    });
}

/// Stores `text` as the run's next spec and writes its file, as the engine and the
/// driver's `WriteDoc` do.
fn spec(rig: &Rig, text: &str) {
    let effect = {
        let mut engine = crate::lock(&rig.runs.state);
        let run = engine.runs.get_mut(&rig.run_id).expect("the run");
        let doc = NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, "ready", text);
        state::store(run, doc, 2_000).expect("stored").1
    };
    let Effect::WriteDoc { path, text, .. } = effect else {
        panic!("not a write: {effect:?}");
    };
    write_new(&path, &text).expect("written");
}

/// `tool` from `window` in `role`, as `anthrex mcp` forwards it: `(ok, raw text)`.
async fn raw(rig: &Rig, role: AgentRole, window: u32, args: serde_json::Value) -> (bool, String) {
    let opts = rig.opts(role, window, None);
    tokio::time::timeout(ANSWER, mcp::forward(&opts, "get_doc", args))
        .await
        .expect("answered")
}

fn error(text: &str) -> String {
    let value: serde_json::Value =
        serde_json::from_str(text).unwrap_or_else(|_| panic!("not a refusal: {text}"));
    value["error"].as_str().expect("an error").to_string()
}

/// The orchestrator and the run's document reviewer read a version's text, capped at
/// 64 KiB with the cut marked, while the run is paused (the engine would refuse); a
/// missing version and an unknown kind are refused with the read helper's text.
#[tokio::test(flavor = "multi_thread")]
async fn get_doc_is_answered_off_engine_and_capped() {
    let rig = Rig::new(design_run).await;
    let head = "# Reset\n\n## Requirements\nR1 A user can ask for a \"reset\" link.\n";
    spec(&rig, head);
    let long = format!("{head}{}", "line \"quoted\" \\ text\n".repeat(8_000));
    assert!(long.len() > 2 * DOC_READ_CAP);
    spec(&rig, &long);

    for (role, window) in [
        (AgentRole::Orchestrator, super::read_rig::ORCH),
        (AgentRole::DocReviewer, REVIEWER),
    ] {
        // The latest version: v2, cut within the cap.
        let (ok, text) = raw(&rig, role, window, json!({"kind": "spec"})).await;
        assert!(ok, "{role:?}: {text}");
        assert!(text.len() <= DOC_READ_CAP, "{role:?}: {} bytes", text.len());
        assert!(text.starts_with(head), "{role:?}");
        let cut = text.lines().last().unwrap();
        assert!(
            cut.starts_with("[cut: ") && cut.ends_with(" bytes]"),
            "{role:?}: {cut}"
        );
        // Only the text: no diff and no findings with it.
        assert!(!text.contains("\"findings\""), "{role:?}");
        // v1, whole.
        let (ok, text) = raw(&rig, role, window, json!({"kind": "spec", "version": 1})).await;
        assert_eq!((ok, text.as_str()), (true, head), "{role:?}");
        let (ok, text) = raw(&rig, role, window, json!({"kind": "spec", "version": 3})).await;
        assert!(!ok);
        assert_eq!(error(&text), format!("run {} has no spec v3", rig.run_id));
        let (ok, text) = raw(&rig, role, window, json!({"kind": "plan"})).await;
        assert!(!ok);
        assert_eq!(error(&text), format!("run {} has no plan yet", rig.run_id));
    }
    // The engine saw none of it: the run is still paused and unchanged.
    assert!(rig.run(|run| run.state == proto::RunState::Paused));
}

/// A design agent's read is checked against the run's live design agents: a window
/// that is not the run's document reviewer is refused, a brainstormer may not read
/// (its role has no `get_doc`), and a run without the design flow says so.
#[tokio::test(flavor = "multi_thread")]
async fn design_agents_are_checked_by_window_and_role() {
    let rig = Rig::new(design_run).await;
    spec(&rig, "# Reset\n\n## Requirements\nR1 one\n");
    let (ok, text) = raw(&rig, AgentRole::DocReviewer, 905, json!({"kind": "spec"})).await;
    assert!(!ok);
    assert_eq!(
        error(&text),
        format!(
            "this window is not the document reviewer of run {}",
            rig.run_id
        )
    );
    let (ok, text) = raw(
        &rig,
        AgentRole::Brainstormer,
        BRAINSTORMER,
        json!({"kind": "spec"}),
    )
    .await;
    assert!(!ok);
    assert_eq!(
        error(&text),
        "tool get_doc is not available to the brainstormer role"
    );
    let (ok, text) = raw(&rig, AgentRole::Brainstormer, 906, json!({"kind": "spec"})).await;
    assert!(!ok);
    assert_eq!(
        error(&text),
        format!("this window is not a brainstormer of run {}", rig.run_id)
    );
    // A reviewer whose session ended is no longer a caller.
    {
        let mut engine = crate::lock(&rig.runs.state);
        let run = engine.runs.get_mut(&rig.run_id).unwrap();
        let design = run.orch.design.as_mut().unwrap();
        design.reviewer.as_mut().unwrap().state = DesignAgentState::Done;
    }
    let (ok, text) = raw(
        &rig,
        AgentRole::DocReviewer,
        REVIEWER,
        json!({"kind": "spec"}),
    )
    .await;
    assert!(!ok);
    assert_eq!(
        error(&text),
        format!(
            "this window is not the document reviewer of run {}",
            rig.run_id
        )
    );

    let plain = Rig::new(|_, _| {}).await;
    let (ok, text) = raw(
        &plain,
        AgentRole::Orchestrator,
        super::read_rig::ORCH,
        json!({"kind": "spec"}),
    )
    .await;
    assert!(!ok);
    assert_eq!(
        error(&text),
        format!("run {} does not use the design flow", plain.run_id)
    );
}

/// Ruling T5-1 (task M9.6.10): the reviewer reads its review draft by `draft`; a draft
/// that does not exist is refused; `get_doc { kind: "spec" }` with no version reads the
/// latest draft while no gate version exists.
#[tokio::test(flavor = "multi_thread")]
async fn the_reviewer_reads_its_review_draft() {
    let rig = Rig::new(design_run).await;
    for (k, text) in [(1, "# Draft one\n"), (2, "# Draft two\n")] {
        let effect = {
            let mut engine = crate::lock(&rig.runs.state);
            let run = engine.runs.get_mut(&rig.run_id).expect("the run");
            let doc = NewDoc {
                draft_review: Some(k),
                ..NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, "draft", text)
            };
            state::store(run, doc, 2_000).expect("stored").1
        };
        let Effect::WriteDoc { path, text, .. } = effect else {
            panic!("not a write: {effect:?}");
        };
        assert!(path.ends_with(format!("spec-draft-r{k}.md")), "{path:?}");
        write_new(&path, &text).expect("written");
    }
    let read = |args| raw(&rig, AgentRole::DocReviewer, REVIEWER, args);
    let (ok, text) = read(json!({"kind": "spec", "draft": 1})).await;
    assert_eq!((ok, text.as_str()), (true, "# Draft one\n"));
    let (ok, text) = read(json!({"kind": "spec"})).await;
    assert_eq!((ok, text.as_str()), (true, "# Draft two\n"));
    let (ok, text) = read(json!({"kind": "spec", "draft": 3})).await;
    assert!(!ok);
    assert_eq!(
        error(&text),
        format!("run {} has no spec draft for review 3", rig.run_id)
    );
}

/// Task 6's carry (e) for task M9.6.11: the plan review's draft is stored before its
/// reviewer starts, so the reviewer's `get_doc { kind: "plan" }` reads it while the
/// plan has no gate version.
#[tokio::test(flavor = "multi_thread")]
async fn the_plan_reviewer_reads_the_plan_sent_to_it() {
    let rig = Rig::new(design_run).await;
    let effect = {
        let mut engine = crate::lock(&rig.runs.state);
        let run = engine.runs.get_mut(&rig.run_id).expect("the run");
        let doc = NewDoc {
            draft_review: Some(1),
            ..NewDoc::new(
                DocKind::Plan,
                DocAuthor::Orchestrator,
                "draft",
                "# Plan: x\n",
            )
        };
        state::store(run, doc, 2_000).expect("stored").1
    };
    let Effect::WriteDoc { path, text, .. } = effect else {
        panic!("not a write: {effect:?}");
    };
    assert!(path.ends_with("plan-draft-r1.md"), "{path:?}");
    write_new(&path, &text).expect("written");
    let read = |args| raw(&rig, AgentRole::DocReviewer, REVIEWER, args);
    let (ok, text) = read(json!({"kind": "plan"})).await;
    assert_eq!((ok, text.as_str()), (true, "# Plan: x\n"));
}

/// Ruling WB-A-I2 (the final fix wave's FW-29), round 2: the plan already has round 1's
/// gate version, and review 2's draft is stored. The plan reviewer's first turn names
/// its draft, and its `get_doc { kind: "plan" }` with no draft and with `draft: 2` both
/// read the draft, never round 1's version; the approved spec still reads as itself.
#[tokio::test(flavor = "multi_thread")]
async fn the_plan_reviewer_reads_its_own_draft_in_round_2() {
    use crate::run::design::state::DocReviewRecord;
    use crate::scout::design_spec::reviewer_first_turn;
    let turn = reviewer_first_turn(DocKind::Plan, 2);
    assert!(turn.contains("kind \"plan\", draft 2"), "{turn}");
    let rig = Rig::new(design_run).await;
    spec(&rig, "# Reset\n\n## Requirements\nR1 x\n");
    let plans = [
        (None, "# Plan: round 1\n", "plan-v1.md"),
        (Some(2), "# Plan: round 2\n", "plan-draft-r2.md"),
    ];
    for (draft, text, name) in plans {
        let effect = {
            let mut engine = crate::lock(&rig.runs.state);
            let run = engine.runs.get_mut(&rig.run_id).expect("the run");
            let doc = NewDoc {
                draft_review: draft,
                ..NewDoc::new(DocKind::Plan, DocAuthor::Orchestrator, "plan", text)
            };
            let effect = state::store(run, doc, 2_000).expect("stored").1;
            let design = run.orch.design.as_mut().unwrap();
            if let Some(k) = draft {
                design.reviews.push(DocReviewRecord {
                    doc: DocKind::Plan,
                    n: k,
                    findings: Vec::new(),
                    failed: None,
                    after: 1,
                    same_runtime: false,
                    dropped: false,
                });
                design.reviewer.as_mut().unwrap().label = format!("plan-r{k}");
            }
            effect
        };
        let Effect::WriteDoc { path, text, .. } = effect else {
            panic!("not a write: {effect:?}");
        };
        assert!(path.ends_with(name), "{path:?}");
        write_new(&path, &text).expect("written");
    }
    let read = |args| raw(&rig, AgentRole::DocReviewer, REVIEWER, args);
    for args in [json!({"kind": "plan"}), json!({"kind": "plan", "draft": 2})] {
        let (ok, text) = read(args.clone()).await;
        assert_eq!((ok, text.as_str()), (true, "# Plan: round 2\n"), "{args}");
    }
    let (ok, text) = read(json!({"kind": "spec"})).await;
    assert_eq!(
        (ok, text.as_str()),
        (true, "# Reset\n\n## Requirements\nR1 x\n")
    );
    // The orchestrator's latest plan is still the gate's.
    let orch = raw(
        &rig,
        AgentRole::Orchestrator,
        super::read_rig::ORCH,
        json!({"kind": "plan"}),
    );
    assert_eq!(orch.await, (true, "# Plan: round 1\n".to_string()));
}

/// Ruling T20-2 (the final fix wave's FW-30): a live brainstormer's `get_doc` never
/// returns a brainstorm draft, of any round, nor the report: every read is refused with
/// the role-admission text, and no text comes back. Round 2's brainstormer, with
/// round 1's and round 2's drafts on disk.
#[tokio::test(flavor = "multi_thread")]
async fn a_brainstormer_can_never_read_a_draft() {
    let rig = Rig::new(design_run).await;
    let secret = "SECRET-DRAFT-TEXT";
    {
        let mut engine = crate::lock(&rig.runs.state);
        let run = engine.runs.get_mut(&rig.run_id).expect("the run");
        let mut writes = Vec::new();
        for (label, round) in [("claude", 1), ("codex", 1), ("claude", 2), ("codex", 2)] {
            let author = DocAuthor::Brainstormer {
                label: label.into(),
            };
            let text = format!("## Understanding\n{secret} {label} {round}\n");
            let doc = NewDoc::new(DocKind::BrainstormDraft, author, "submitted", &text);
            writes.push(state::store(run, doc, 2_000).expect("stored").1);
        }
        let report = NewDoc::new(
            DocKind::Brainstorm,
            DocAuthor::Orchestrator,
            "submitted",
            &format!("## Recommendation\n{secret}\n"),
        );
        writes.push(state::store(run, report, 2_000).expect("stored").1);
        let design = run.orch.design.as_mut().unwrap();
        design.brainstormers[0].round = 2;
        for effect in writes {
            let Effect::WriteDoc { path, text, .. } = effect else {
                panic!("not a write: {effect:?}");
            };
            write_new(&path, &text).expect("written");
        }
    }
    let refused = "tool get_doc is not available to the brainstormer role";
    for args in [
        json!({"kind": "brainstorm_draft"}),
        json!({"kind": "brainstorm_draft", "from": "claude"}),
        json!({"kind": "brainstorm_draft", "from": "codex"}),
        json!({"kind": "brainstorm_draft", "version": 1}),
        json!({"kind": "brainstorm_draft", "version": 3}),
        json!({"kind": "brainstorm"}),
        json!({"kind": "brainstorm", "version": 1}),
    ] {
        let (ok, text) = raw(&rig, AgentRole::Brainstormer, BRAINSTORMER, args.clone()).await;
        assert!(!ok, "{args}");
        assert!(!text.contains(secret), "{args}: {text}");
        assert_eq!(error(&text), refused, "{args}");
    }
}
