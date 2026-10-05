//! Milestone 9.6 task M9.6.12, engine side: the documents commit (decision 23, DF
//! §5.3). The plan gate's approval asks for one `CommitDesignDocs` of the approved
//! versions; nothing branches from the run head, no worktree is prepared and no stage
//! is created until its reply (Review focus 3), across a restart too; ruling T1-O2 skips
//! the plan gate's pre-warm when the documents will be committed; the new head is the
//! run head, so the bottom stage holds it in every layout; a failure halts and `run
//! resume` retries it. The git side is `driver/design_commit_tests.rs` and
//! `tests/run_git_docs.rs`.

use proto::{DocGateAction, DocGateKind, RunState};
use serde_json::json;

use super::delivery_open::pr_mode;
use super::design_fixture::*;
use super::design_plan_fixture::covering;
use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use crate::run::design::commit::DocsCommitSpec;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult};
use crate::run::model::StageLayout;

/// The commit's new head in these tests.
pub(super) const DOCS: &str = "dddddddddddddddddddddddddddddddddddddddd";
const SPEC_PATH: &str = "docs/anthrex/specs/1970-01-01-password-reset.md";

pub(super) fn commits(effects: &[Effect]) -> Vec<(u64, DocsCommitSpec)> {
    ops_in(effects, "CommitDesignDocs")
        .into_iter()
        .map(|(op, kind)| match kind {
            OpKind::CommitDesignDocs(spec) => (op, *spec),
            _ => unreachable!(),
        })
        .collect()
}

/// The pending `CommitDesignDocs`.
pub(super) fn pending_commit(fx: &Fixture) -> (u64, DocsCommitSpec) {
    let mut pending: Vec<(u64, DocsCommitSpec)> = (fx.run().pending_ops.values())
        .filter_map(|p| match &p.kind {
            OpKind::CommitDesignDocs(spec) => Some((p.op, (**spec).clone())),
            _ => None,
        })
        .collect();
    assert_eq!(pending.len(), 1, "one commit in flight");
    pending.remove(0)
}

pub(super) fn committed(fx: &mut Fixture, op: u64) -> Vec<Effect> {
    let result = OpResult::DocsCommitted {
        head: DOCS.into(),
        spec: SPEC_PATH.into(),
    };
    fx.done(op, result)
}

/// Every effect that would branch from the run head: a worktree, a stage branch, a
/// review's checkout.
fn branching(effects: &[Effect]) -> Vec<&'static str> {
    let names = ["PrepareWorktree", "CreateStageBranch", "PrepareReview"];
    (names.into_iter())
        .filter(|name| !ops_in(effects, name).is_empty())
        .collect()
}

/// [`at_plan_gate`], with `edit` applied to the run while it is at the spec gate, and
/// `tasks` (each covering both requirements) as the plan.
pub(super) fn plan_gate_with(
    edit: impl FnOnce(&mut crate::run::model::Run),
    tasks: &[serde_json::Value],
) -> Fixture {
    let mut fx = at_spec_gate(false);
    edit(fx.run_mut());
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    super::design_plan_fixture::read_back(&mut fx, 1, SPEC);
    let args = json!({"edits": tasks, "submit": true});
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", args);
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    super::design_plan_fixture::plan_reviewed(&mut fx, json!([]));
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert_eq!(gate(&fx).map(|g| g.0), Some(DocGateKind::Plan));
    fx
}

fn one_task() -> Vec<serde_json::Value> {
    vec![covering("t1", &["R1", "R2"])]
}

pub(super) fn approve(fx: &mut Fixture) -> Vec<Effect> {
    let reply = fx.reply();
    let effects = fx.next(EventKind::DocGate {
        reply,
        run_id: RUN_ID.into(),
        kind: DocGateKind::Plan,
        action: DocGateAction::APPROVE,
    });
    assert_eq!(
        replies(&effects),
        vec![Ok(format!("run {RUN_ID} approved"))]
    );
    effects
}

/// Decisions 23 and 24: the approval asks for one commit of the approved spec and plan,
/// read back from their stored files with their recorded sizes and SHA-256, on the run
/// branch at the run head, with the run's start date and decision 24's message.
#[test]
fn approval_asks_for_the_commit_of_the_approved_versions() {
    let mut fx = at_plan_gate(false);
    let effects = approve(&mut fx);
    let mut asked = commits(&effects);
    assert_eq!(asked.len(), 1, "{effects:?}");
    let (_, spec) = asked.remove(0);
    let run = fx.run();
    let design = run.orch.design.as_ref().unwrap();
    assert!(design.commit_due);
    assert_eq!(spec.branch, format!("anthrex/{RUN_ID}/integration"));
    assert_eq!(spec.expected_head, run.run_head);
    assert_eq!(spec.integration, run.integration_path());
    assert_eq!(spec.root, run.root);
    assert_eq!(spec.docs_dir, "docs/anthrex");
    assert_eq!(spec.date, "1970-01-01");
    assert_eq!(spec.run_id, RUN_ID);
    let head = crate::run::design::plan_md::goal_head(&run.goal);
    assert_eq!(spec.message, format!("docs: spec and plan for {head}"));
    let sources: Vec<(String, String)> = (spec.files.iter())
        .map(|f| (f.folder.clone(), f.path.display().to_string()))
        .collect();
    let dir = format!("/tmp/data/runs/{RUN_ID}/design");
    assert_eq!(
        sources,
        vec![
            ("specs".to_string(), format!("{dir}/spec-v1.md")),
            ("plans".to_string(), format!("{dir}/plan-v1.md")),
        ]
    );
    let stored = design.find(proto::DocKind::Spec, Some(1)).unwrap();
    assert_eq!(
        (spec.files[0].bytes, spec.files[0].sha256.as_str()),
        (stored.bytes, stored.sha256.as_str())
    );
    let plan = design.find(proto::DocKind::Plan, Some(1)).unwrap();
    assert_eq!(
        (spec.files[1].bytes, spec.files[1].sha256.as_str()),
        (plan.bytes, plan.sha256.as_str())
    );
}

/// `commit_brainstorm` adds the approved brainstorm report, its appendix included (the
/// stored bytes are what is committed).
#[test]
fn commit_brainstorm_adds_the_report() {
    let mut fx = plan_gate_with(
        |run| run.limits.orch.design.commit_brainstorm = true,
        &one_task(),
    );
    let (_, spec) = commits(&approve(&mut fx)).remove(0);
    let folders: Vec<&str> = spec.files.iter().map(|f| f.folder.as_str()).collect();
    assert_eq!(folders, ["specs", "plans", "brainstorms"]);
    let design = fx.run().orch.design.as_ref().unwrap();
    let report = design.find(proto::DocKind::Brainstorm, None).unwrap();
    assert_eq!(spec.files[2].sha256, report.sha256);
    assert!(spec.files[2].path.ends_with("brainstorm-v1.md"));
}

/// Review focus 3 and ruling T1-O2: with documents to commit, the plan gate pre-warms
/// nothing, and after the approval nothing branches from the run head, ticks included,
/// until the commit's reply; then the work starts from the commit.
#[test]
fn no_worktree_is_prepared_before_the_commit() {
    let mut fx = at_plan_gate(false);
    assert_eq!(
        branching(&fx.log),
        Vec::<&str>::new(),
        "no pre-warm at the gate"
    );
    let mut effects = approve(&mut fx);
    effects.extend(fx.tick());
    effects.extend(fx.tick());
    assert_eq!(branching(&effects), Vec::<&str>::new());
    assert_eq!(commits(&effects).len(), 1, "asked once: {effects:?}");
    assert_eq!(fx.run().state, RunState::Running);

    let (op, _) = pending_commit(&fx);
    let effects = committed(&mut fx, op);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(!design.commit_due);
    assert_eq!(design.committed.as_deref(), Some(DOCS));
    assert_eq!(design.spec_path.as_deref(), Some(SPEC_PATH));
    assert_eq!(fx.run().run_head, DOCS);
    let prepared = ops_in(&effects, "PrepareWorktree");
    assert_eq!(prepared.len(), 1, "{effects:?}");
    match &prepared[0].1 {
        OpKind::PrepareWorktree { from, .. } => assert_eq!(from, DOCS),
        _ => unreachable!(),
    }
    let log = log_lines(&fx);
    assert!(
        log.contains(&format!(
            "committed the spec and plan as ddddddd on anthrex/{RUN_ID}/integration"
        )),
        "{log:?}"
    );
}

/// Ruling T1-O2: with `docs_dir = ""` the plan gate pre-warms as in 9.5.
#[test]
fn docs_dir_empty_keeps_the_plan_gate_prewarm() {
    let fx = plan_gate_with(|run| run.limits.orch.design.docs_dir.clear(), &one_task());
    let prewarmed = ops_in(&fx.log, "PrepareWorktree");
    assert_eq!(prewarmed.len(), 1, "{:?}", fx.log);
    match &prewarmed[0].1 {
        OpKind::PrepareWorktree { from, .. } => assert_eq!(from, BASE),
        _ => unreachable!(),
    }
}

/// Decision 23: with `docs_dir = ""` nothing is committed, and the run starts at once.
#[test]
fn docs_dir_empty_commits_nothing() {
    let mut fx = plan_gate_with(|run| run.limits.orch.design.docs_dir.clear(), &one_task());
    fx.complete_prepares();
    let mut effects = approve(&mut fx);
    effects.extend(fx.tick());
    assert!(commits(&effects).is_empty(), "{effects:?}");
    assert!(!fx.run().orch.design.as_ref().unwrap().commit_due);
    assert_eq!(ops_in(&effects, "CreateWindow").len(), 1, "{effects:?}");
}

/// DF §8 and decision 23: the commit is the bottom stage's in `pr` mode. A `Single` run's
/// one stage (`integration`) moves to it; a `Multi` run's stage 1 is created from it, and
/// not before.
#[test]
fn in_pr_mode_the_commit_is_the_bottom_stage_in_single_and_multi_layouts() {
    let mut fx = plan_gate_with(pr_mode, &one_task());
    let effects = approve(&mut fx);
    assert_eq!(fx.run().stage_layout, StageLayout::Single);
    assert_eq!(branching(&effects), Vec::<&str>::new());
    let (op, _) = pending_commit(&fx);
    committed(&mut fx, op);
    assert_eq!(fx.run().stage(1).map(|s| s.head.as_str()), Some(DOCS));
    // Bisect and tier 3 measure the stage's merges from the commit (ruling C-28 (1)).
    assert_eq!(
        fx.run().stage(1).and_then(|s| s.floor.as_deref()),
        Some(DOCS)
    );
    assert_eq!(fx.run().run_head, DOCS);

    let mut t2 = covering("t2", &["R2"]);
    t2["task"]["stage"] = json!(2);
    t2["task"]["deps"] = json!(["t1"]);
    let mut fx = plan_gate_with(pr_mode, &[covering("t1", &["R1"]), t2]);
    let mut effects = approve(&mut fx);
    effects.extend(fx.tick());
    assert_eq!(fx.run().stage_layout, StageLayout::Multi);
    assert_eq!(branching(&effects), Vec::<&str>::new(), "{effects:?}");
    let (op, _) = pending_commit(&fx);
    let effects = committed(&mut fx, op);
    let created = ops_in(&effects, "CreateStageBranch");
    assert_eq!(created.len(), 1, "{effects:?}");
    match &created[0].1 {
        OpKind::CreateStageBranch { branch, from, .. } => {
            assert_eq!(branch, &format!("anthrex/{RUN_ID}/stage-1"));
            assert_eq!(from, DOCS);
        }
        _ => unreachable!(),
    }
}

/// Review focus 3: a restart between the approval and the commit's reply prepares
/// nothing and sends the commit again, as it was; the reply then lets the work start
/// from the commit once the run is resumed.
#[test]
fn a_restart_between_approval_and_the_commit_reply_prepares_nothing_and_resends_the_commit() {
    let mut fx = at_plan_gate(false);
    approve(&mut fx);
    let (lost, asked) = pending_commit(&fx);
    let run = fx.run().clone();

    let mut fx = Fixture::new("");
    let effects = fx.next(EventKind::Restore {
        runs: vec![run],
        replay: Vec::new(),
        held: Vec::new(),
    });
    let again = commits(&effects);
    assert_eq!(again.len(), 1, "{effects:?}");
    assert_ne!(again[0].0, lost, "under a new id");
    assert_eq!(again[0].1, asked, "the same commit");
    assert_eq!(branching(&effects), Vec::<&str>::new());
    assert_eq!(fx.run().state, RunState::Paused);

    let reply = fx.reply();
    let mut effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    });
    effects.extend(fx.tick());
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(branching(&effects), Vec::<&str>::new(), "{effects:?}");
    assert!(commits(&effects).is_empty(), "not asked twice: {effects:?}");

    let effects = committed(&mut fx, again[0].0);
    assert_eq!(fx.run().run_head, DOCS);
    assert_eq!(ops_in(&effects, "PrepareWorktree").len(), 1, "{effects:?}");
}

/// Decision 23: a failed commit halts with its exact text, retryably; `run resume`
/// sends it again, and still nothing is prepared before its reply.
#[test]
fn a_failed_commit_halts_and_resume_retries() {
    let mut fx = at_plan_gate(false);
    approve(&mut fx);
    let (op, asked) = pending_commit(&fx);
    fx.done(
        op,
        OpResult::Failed {
            message: "git commit-tree failed: no identity".into(),
        },
    );
    let run = fx.run();
    assert_eq!(run.state, RunState::Halted);
    let text =
        "design flow: could not commit the spec and plan: git commit-tree failed: no identity";
    assert_eq!(run.halted_reason.as_deref(), Some(text));
    assert!(run.halt_retryable);
    assert!(run.orch.design.as_ref().unwrap().commit_due);

    let reply = fx.reply();
    let effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    });
    assert_eq!(replies(&effects), vec![Ok(format!("run {RUN_ID} resumed"))]);
    let again = commits(&effects);
    assert_eq!(again.len(), 1, "{effects:?}");
    assert_eq!(again[0].1, asked);
    assert_eq!(branching(&effects), Vec::<&str>::new());
    committed(&mut fx, again[0].0);
    assert_eq!(fx.run().run_head, DOCS);
}

/// The task's addendum: a documents folder that goes through a tracked symbolic link
/// halts with its own exact text.
#[test]
fn a_documents_folder_through_a_symlink_halts() {
    let mut fx = at_plan_gate(false);
    approve(&mut fx);
    let (op, _) = pending_commit(&fx);
    fx.done(
        op,
        OpResult::DocsThroughSymlink {
            path: "docs".into(),
        },
    );
    let run = fx.run();
    assert_eq!(run.state, RunState::Halted);
    let text = "design flow: the documents folder docs/anthrex goes through a symlink in the repository; change [orchestrator.design].docs_dir";
    assert_eq!(run.halted_reason.as_deref(), Some(text));
    assert!(run.orch.design.as_ref().unwrap().commit_due);
    assert_eq!(fx.run().run_head, BASE);
    // Ruling T12-1: the frozen docs_dir means a resume cannot succeed.
    assert!(!run.halt_retryable);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    });
    assert!(replies(&effects)[0].is_err(), "{effects:?}");
    assert!(commits(&effects).is_empty());
}

/// The final fix wave's FW-21 (review A's M-4): with `docs_dir = ""` the pre-warm runs
/// at the plan gate only. A back from the plan gate to the spec gate, with a writer slot
/// free there, pre-warms nothing for a plan the orchestrator is about to rewrite.
#[test]
fn no_prewarm_at_the_spec_gate() {
    let no_docs = |run: &mut crate::run::model::Run| {
        run.limits.orch.design.docs_dir.clear();
        run.limits.max_writers = 0;
    };
    let mut fx = plan_gate_with(no_docs, &one_task());
    assert!(ops_in(&fx.log, "PrepareWorktree").is_empty(), "no slot yet");
    let back = DocGateAction::Back { note: "b".into() };
    act(&mut fx, DocGateKind::Plan, back).unwrap();
    assert_eq!(gate(&fx).map(|g| g.0), Some(DocGateKind::Spec));
    fx.run_mut().limits.max_writers = 1;
    let effects = fx.tick();
    assert!(
        ops_in(&effects, "PrepareWorktree").is_empty(),
        "{effects:?}"
    );
}
