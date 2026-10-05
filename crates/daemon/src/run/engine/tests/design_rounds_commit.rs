//! Milestone 9.6 task M9.6.15 (decision 24, DF §8.1): round k's documents commit. On
//! the approval of round k's plan, the amendment is appended to the committed spec
//! under `## Round <k> amendment` and the round's plan goes to `…-round<k>.md`; the
//! commit is the round's first new stage (its stage branch is created at the commit),
//! so stage 1's record and ref never move, the run head and `integration` follow the
//! new top stage, and nothing of the round branches before the reply. And an
//! orchestrator-started round stops at every gate, `--yes` or not.

use proto::{DocGateAction, DocGateKind, PrState, RunState};
use serde_json::json;

use super::delivery_land::pr_record;
use super::delivery_open::pr_mode;
use super::design_commit::{approve, commits};
use super::design_fixture::*;
use super::design_rounds::round_task;
use super::design_rounds_fixture::*;
use super::fixture::*;
use super::kinds_integration::C1;
use super::orch::{answer, edit_plan};
use crate::run::delivery::StageDelivery;
use crate::run::design::commit::DocsCommitSpec;
use crate::run::engine::{Effect, EventKind, OpResult};

/// Round 2's documents commit in these tests.
pub(super) const ROUND_DOCS: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
/// Round 1's committed spec (`design_commit::committed`).
const SPEC_PATH: &str = "docs/anthrex/specs/1970-01-01-password-reset.md";
const ROUND_PLAN: &str = "docs/anthrex/plans/1970-01-01-password-reset-round2.md";

/// Round 2 at its plan gate (`t2` in stage 2 covering R2 and R3), approved: the
/// widened run creates stage 1 from the run head, then asks for round 2's commit as
/// stage 2. Returns the commit's op and spec.
pub(super) fn approved_round(fx: Fixture) -> (Fixture, u64, DocsCommitSpec) {
    let mut fx = plan_gate_of(fx, json!([round_task("t2", &["R2", "R3"])]));
    let effects = approve(&mut fx);
    assert!(
        commits(&effects).is_empty(),
        "stage 1 comes first: {effects:?}"
    );
    let (op, kind) = fx.op("CreateStageBranch");
    let branch = format!("anthrex/{RUN_ID}/stage-1");
    assert!(format!("{kind:?}").contains(&branch), "{kind:?}");
    let effects = fx.done(op, OpResult::StageCreated);
    assert!(
        ops_in(&effects, "CreateStageBranch").is_empty(),
        "{effects:?}"
    );
    assert!(
        ops_in(&effects, "PrepareWorktree").is_empty(),
        "{effects:?}"
    );
    let mut asked = commits(&effects);
    assert_eq!(asked.len(), 1, "{effects:?}");
    let (op, spec) = asked.remove(0);
    (fx, op, spec)
}

/// Decision 24, round k: the spec is the committed file, the amendment appended under
/// its heading; the plan is the round's own file beside round 1's; on the round's first
/// new stage, at the head of the stage below.
fn assert_round_spec(fx: &Fixture, spec: &DocsCommitSpec) {
    let run = fx.run();
    assert_eq!(
        spec.stage_branch.as_deref(),
        Some(run.stage_branch(2).as_str())
    );
    assert_eq!(spec.branch, run.run_branch());
    assert_eq!(spec.expected_head, run.stage(1).unwrap().head);
    let files: Vec<(Option<&str>, Option<&str>, String)> = (spec.files.iter())
        .map(|f| {
            let name = f.path.file_name().unwrap().to_string_lossy().into_owned();
            (f.repo_path.as_deref(), f.append.as_deref(), name)
        })
        .collect();
    assert_eq!(
        files,
        vec![
            (
                Some(SPEC_PATH),
                Some("## Round 2 amendment"),
                "spec-v2.md".into()
            ),
            (Some(ROUND_PLAN), None, "plan-v2.md".into()),
        ]
    );
}

/// The commit's reply: stage 2 is recorded at the commit (created from it, holding stage
/// 1's head), the run head follows it, stage 1 is untouched, and round 2's work starts.
pub(super) fn assert_committed(fx: &mut Fixture, op: u64, stage1: &str) -> Vec<Effect> {
    let result = OpResult::DocsCommitted {
        head: ROUND_DOCS.into(),
        spec: SPEC_PATH.into(),
    };
    let mut effects = fx.done(op, result);
    effects.extend(fx.tick());
    let run = fx.run();
    let two = run.stage(2).expect("stage 2 recorded");
    assert_eq!(two.head, ROUND_DOCS);
    assert_eq!(two.created_from, ROUND_DOCS);
    assert_eq!(two.synced_from.as_deref(), Some(stage1));
    assert_eq!(two.round, 2);
    assert_eq!(run.stage(1).unwrap().head, stage1, "stage 1 never moves");
    assert_eq!(run.run_head, ROUND_DOCS);
    let design = run.orch.design.as_ref().unwrap();
    assert!(!design.commit_due);
    assert_eq!(design.committed.as_deref(), Some(ROUND_DOCS));
    assert_eq!(design.committed_round, 2);
    assert_eq!(design.spec_path.as_deref(), Some(SPEC_PATH));
    assert!(
        !ops_in(&effects, "PrepareWorktree").is_empty(),
        "{effects:?}"
    );
    effects
}

/// Decision 24 in local mode: the round's commit, the amendment appended and the
/// round's plan, is its first new stage (the git side is
/// `driver/design_commit_round_tests.rs`).
#[test]
fn the_round_commit_is_the_rounds_first_new_stage() {
    let (mut fx, op, spec) = approved_round(design_complete());
    assert_eq!(fx.run().stage(1).unwrap().head, C1);
    assert_round_spec(&fx, &spec);
    let head = crate::run::design::plan_md::goal_head(&fx.run().goal);
    assert_eq!(
        spec.message,
        format!("docs: round 2 spec amendment and plan for {head}")
    );
    assert_committed(&mut fx, op, C1);
}

/// DF §8.1: in `pr` mode the round's commit goes on the round's first new stage, never
/// on stage 1, whose PR is open.
#[test]
fn in_pr_mode_the_round_commit_goes_on_the_rounds_first_new_stage() {
    let mut fx = design_complete();
    let head = fx.run().run_head.clone();
    let run = fx.run_mut();
    pr_mode(run);
    run.state = RunState::Running;
    let mut pr = pr_record(12, PrState::Open);
    pr.pushed_head = head.clone();
    run.delivery.stages = vec![StageDelivery {
        pr: Some(pr),
        ..StageDelivery::default()
    }];
    fx.tick();
    let (mut fx, op, spec) = approved_round(fx);
    assert_round_spec(&fx, &spec);
    assert_committed(&mut fx, op, &head);
    let pr = fx.run().delivery.pr(1).unwrap();
    assert_eq!(pr.pushed_head, head, "stage 1's PR is untouched");
}

/// Review focus 3 for a round: a restart between the request and the reply sends the
/// same round commit again (its stage branch included), and nothing of the round
/// branches before its reply.
#[test]
fn a_restart_resends_the_round_commit_and_branches_nothing() {
    let (fx, lost, asked) = approved_round(design_complete());
    let run = fx.run().clone();
    let stage1 = run.stage(1).unwrap().head.clone();
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
    let reply = fx.reply();
    let mut effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    });
    effects.extend(fx.tick());
    assert_eq!(fx.run().state, RunState::Running);
    for name in ["PrepareWorktree", "CreateStageBranch", "CommitDesignDocs"] {
        assert!(ops_in(&effects, name).is_empty(), "{name}: {effects:?}");
    }
    assert_committed(&mut fx, again[0].0, &stage1);
}

/// Round 1's commit is task 12's: on the run branch, every file named by the slug, no
/// stage branch made.
#[test]
fn round_one_still_commits_on_the_run_branch() {
    let mut fx = at_plan_gate(false);
    let effects = approve(&mut fx);
    let asked = commits(&effects);
    assert_eq!(asked.len(), 1, "{effects:?}");
    assert_eq!(asked[0].1.stage_branch, None);
    assert!(
        asked[0]
            .1
            .files
            .iter()
            .all(|f| f.repo_path.is_none() && f.append.is_none())
    );
    assert_eq!(fx.run().orch.design.as_ref().unwrap().committed, None);
}

/// DF §8.1 and §8.2: a round the orchestrator starts stops at every gate, with `--yes`
/// too: its amendment waits at the spec gate, and its plan at the plan gate.
#[test]
fn an_orchestrator_started_round_or_goal_stops_at_every_gate() {
    let mut fx = design_complete_with(true);
    assert!(fx.run().orch.yes);
    assert!(answer(&edit_plan(&mut fx, json!({"iterate": "Add the app"}))).0);
    assert_eq!(fx.run().state, RunState::Specifying);
    submit_amendment(&mut fx, AMENDMENT).unwrap();
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert_eq!(gate(&fx).map(|g| g.0), Some(DocGateKind::Spec));
    act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).unwrap();
    super::design_plan_fixture::read_back(&mut fx, 2, AMENDMENT);
    round_submit(&mut fx, json!([round_task("t2", &["R2", "R3"])])).unwrap();
    round_reviewed(&mut fx);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert_eq!(gate(&fx).map(|g| g.0), Some(DocGateKind::Plan));
}

/// Round 2 of a `pr` run whose stage 1's PR is open, at its plan gate, approved: the
/// round's commit asked (`approved_round`).
fn pr_round_approved() -> (Fixture, u64, DocsCommitSpec) {
    let mut fx = design_complete();
    let head = fx.run().run_head.clone();
    let run = fx.run_mut();
    pr_mode(run);
    run.state = RunState::Running;
    let mut pr = pr_record(12, PrState::Open);
    pr.pushed_head = head;
    run.delivery.stages = vec![StageDelivery {
        pr: Some(pr),
        ..StageDelivery::default()
    }];
    fx.tick();
    approved_round(fx)
}

/// Ruling T15-6: a base sync due while the round's commit is in flight waits for its
/// reply (its merge would move the stage below the commit, or lose the CAS to it), then
/// starts.
#[test]
fn a_base_sync_waits_for_the_round_commit() {
    let (mut fx, op, _) = pr_round_approved();
    let stage1 = fx.run().stage(1).unwrap().head.clone();
    fx.run_mut()
        .delivery
        .base_sync_due
        .insert(1, "baba".repeat(10));
    let effects = fx.tick();
    assert!(ops_in(&effects, "Propagate").is_empty(), "{effects:?}");
    assert!(fx.run().delivery.base_sync_due.contains_key(&1));
    let effects = assert_committed(&mut fx, op, &stage1);
    let mut started = ops_in(&effects, "Propagate");
    started.extend(ops_in(&fx.tick(), "Propagate"));
    assert_eq!(started.len(), 1, "{effects:?}");
}

/// Ruling T15-7 (m5): a later round's documents commit is built only from an approved
/// amendment, as the round's first stage. A round with no new approved spec, or one
/// whose run is not `Multi`, halts with decision 23's text instead of committing a
/// round-1-shaped commit.
#[test]
fn a_round_commit_is_only_an_amendment_on_its_own_stage() {
    let edits = || json!([round_task("t2", &["R2", "R3"])]);
    let mut fx = round_plan_gate(edits());
    let design = fx.run_mut().orch.design.as_mut().unwrap();
    design.approved_spec = design.round.as_ref().unwrap().spec_before;
    approve(&mut fx);
    let (op, _) = fx.op("CreateStageBranch");
    let effects = fx.done(op, OpResult::StageCreated);
    assert!(commits(&effects).is_empty(), "{effects:?}");
    let reason = "design flow: could not commit the spec and plan: round 2 approved no amendment of the spec";
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(fx.run().halted_reason.as_deref(), Some(reason));
    let mut fx = round_plan_gate(edits());
    fx.run_mut().stage_layout = crate::run::model::StageLayout::Single;
    let mut effects = approve(&mut fx);
    effects.extend(fx.tick());
    assert!(commits(&effects).is_empty(), "{effects:?}");
    let reason = "design flow: could not commit the spec and plan: round 2's documents commit is not a stage of the run";
    assert_eq!(fx.run().halted_reason.as_deref(), Some(reason));
}
