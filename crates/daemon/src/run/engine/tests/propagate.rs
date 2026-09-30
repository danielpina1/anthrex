//! Milestone 9.1 task M9.1.17: each stage's head flows into the stage above it through
//! a propagate (decisions 49 and 50), a conflicted propagate becomes a `sync` fix task
//! whose worktree holds the merge (decision 51), a red propagate waits for the user or
//! the orchestrator (decision 52), and a propagate is journaled and restarted like a
//! merge candidate (decision 53). On the engine fixture; ops are answered with
//! `Fixture::done`.

use std::collections::BTreeSet;

use proto::{AgentRole, RouteSpec, Size, TaskOrigin, TaskState, TestMode};

use super::bisect::with_orchestrator;
use super::control::resume;
use super::control_restore::restart;
use super::fixture::*;
use super::full::{attention, later, merge_tiered, outcome, profile};
use super::merge::{commit, doc_task, merge, pending, pending_one, start_on, to_queue, window_of};
use super::wake_notes::notes;
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{OpKind, OpResult};
use crate::run::model::{FixOf, OpId, PropagateSpec, StageMerge, SyncState};

/// The conflicted tree a propagate's `merge-tree` wrote.
const TREE: &str = "7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a";

fn stage_branch(n: u16) -> String {
    format!("anthrex/{RUN_ID}/stage-{n}")
}

/// A running `Multi` run of `tasks` on `profile`, every stage created that can be, and
/// every dispatched worker launched.
fn stages_on(profile: &str, tasks: &[String]) -> (Fixture, Vec<(String, u32)>) {
    let (mut fx, mut windows) = start_on(profile, tasks);
    while let Some((op, _)) = pending(&fx, "CreateStageBranch", None).first().cloned() {
        fx.done(op, OpResult::StageCreated);
    }
    windows.extend(fx.launch_all());
    (fx, windows)
}

/// `t1` in stage 1, `t2` in stage 2 and, with `three`, `t3` in stage 3; all created.
fn stages(three: bool) -> (Fixture, Vec<(String, u32)>) {
    let mut tasks = vec![doc_task("t1", ""), doc_task("t2", "stage = 2")];
    if three {
        tasks.push(doc_task("t3", "stage = 3"));
    }
    let (fx, windows) = stages_on(PROFILE, &tasks);
    assert_eq!(fx.run().stages.len(), tasks.len(), "{:#?}", fx.run().stages);
    (fx, windows)
}

/// The pending propagates, as `(op, spec)`.
fn propagates(fx: &Fixture) -> Vec<(OpId, PropagateSpec)> {
    pending(fx, "Propagate", None)
        .into_iter()
        .map(|(op, kind)| match kind {
            OpKind::Propagate(spec) => (op, *spec),
            _ => unreachable!(),
        })
        .collect()
}

/// The one pending propagate.
fn propagate(fx: &Fixture) -> (OpId, PropagateSpec) {
    let all = propagates(fx);
    assert_eq!(all.len(), 1, "one pending propagate: {all:#?}");
    all[0].clone()
}

/// Answers every pending propagate as landed, each at a commit of its own.
pub(super) fn land_propagates(fx: &mut Fixture) {
    while let Some((op, _)) = propagates(fx).first().cloned() {
        let at = format!("{op}{}", "a".repeat(40 - op.to_string().len()));
        fx.done(op, merged_at(&at));
    }
}

fn merged_at(at: &str) -> OpResult {
    OpResult::Merged {
        commit: at.into(),
        tier: None,
    }
}

/// `t1` merges into stage 1 at `commit(1)`.
fn merge_t1(fx: &mut Fixture, windows: &[(String, u32)]) {
    to_queue(fx, "t1", window_of(windows, "t1"));
    merge(fx, "t1", &commit(1));
}

#[test]
fn head_move_on_stage_k_queues_propagate_to_k_plus_1_lowest_first() {
    let (mut fx, windows) = stages(true);
    assert!(propagates(&fx).is_empty(), "no head has moved");
    // t1 and t2 both queued: t1's candidate first (FIFO), t2's waits behind it.
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t1", &commit(1));
    // Stage 1's head moved: stage 2 is due, and its propagate goes before t2's merge.
    assert!(pending(&fx, "MergeCandidate", Some("t2")).is_empty());
    let (op, spec) = propagate(&fx);
    let run = fx.run();
    assert_eq!((spec.from, spec.to), (1, 2));
    assert_eq!(spec.from_head, commit(1));
    assert_eq!(spec.to_branch, stage_branch(2));
    assert_eq!(spec.expected_to_head, BASE);
    assert!(!spec.also_integration, "stage 3 is the highest");
    assert_eq!(spec.message, "anthrex: propagate stage-1 into stage-2");
    assert_eq!(spec.tasks, BTreeSet::from(["t1".to_string()]));
    assert_eq!(spec.guarded, crate::run::engine::stages::guard_list(run));
    assert_eq!(
        spec.check, run.profile.check,
        "an untiered profile runs check"
    );
    assert_eq!(spec.tier, None);
    assert_eq!(spec.base_branch, run.base_branch);
    assert_eq!(spec.expected_base, run.base_sha);
    assert_eq!(spec.integration, run.integration_path());
    assert!(
        run.propagate_due.is_empty(),
        "taken: {:?}",
        run.propagate_due
    );
    // It lands: stage 2 holds t1, and stage 3 is due in turn, before t2's merge.
    fx.done(op, merged_at(&commit(21)));
    let stage = fx.run().stage(2).unwrap();
    assert_eq!(stage.head, commit(21));
    assert!(stage.tasks_in.contains("t1"));
    assert_eq!(stage.synced_from, Some(commit(1)));
    assert_eq!(
        stage.merges,
        [StageMerge::Propagate {
            from: 1,
            commit: commit(21)
        }]
    );
    assert_eq!(stage.last_green_candidate, Some(commit(21)));
    let (op, spec) = propagate(&fx);
    assert_eq!((spec.from, spec.to), (2, 3));
    assert_eq!(spec.from_head, commit(21));
    assert_eq!(spec.expected_to_head, BASE);
    assert!(spec.also_integration, "into the highest stage");
    assert!(pending(&fx, "MergeCandidate", Some("t2")).is_empty());
    fx.done(op, merged_at(&commit(32)));
    assert_eq!(fx.run().stage_head(3), Some(commit(32).as_str()));
    assert_eq!(
        fx.run().run_head,
        commit(32),
        "integration moved with stage 3"
    );
    assert!(fx.run().stage(3).unwrap().tasks_in.contains("t1"));
    // Nothing more is due: t2's candidate runs now.
    assert!(propagates(&fx).is_empty());
    let (op, _) = pending_one(&fx, "MergeCandidate", Some("t2"));
    fx.done(op, merged_at(&commit(4)));
    let (op, spec) = propagate(&fx);
    assert_eq!(
        (spec.from, spec.to, spec.from_head.as_str()),
        (2, 3, commit(4).as_str())
    );
    fx.done(op, merged_at(&commit(43)));

    // Two heads moved at once (stage 2's first): the lowest due stage goes first.
    let run = fx.run_mut();
    set_stage_head(run, 2, &commit(5));
    set_stage_head(run, 1, &commit(6));
    assert_eq!(run.propagate_due, BTreeSet::from([2, 3]));
    fx.tick();
    let (op, spec) = propagate(&fx);
    assert_eq!((spec.from, spec.to), (1, 2));
    fx.done(op, merged_at(&commit(7)));
    let (_, spec) = propagate(&fx);
    assert_eq!(
        (spec.from, spec.to, spec.from_head.as_str()),
        (2, 3, commit(7).as_str())
    );
}

#[test]
fn a_propagate_already_held_is_dropped_without_an_op() {
    let (mut fx, windows) = stages(false);
    merge_t1(&mut fx, &windows);
    let (op, _) = propagate(&fx);
    fx.done(op, merged_at(&commit(2)));
    assert!(propagates(&fx).is_empty());
    // Stage 2 already holds stage 1's head.
    fx.run_mut().propagate_due.insert(2);
    fx.tick();
    assert!(propagates(&fx).is_empty());
    assert!(fx.run().propagate_due.is_empty());
}

/// Decision 50: a tiered profile's propagate runs tier 2, on the stage it lands in.
#[test]
fn a_tiered_propagate_carries_tier2_and_no_check() {
    let tasks = [doc_task("t1", ""), doc_task("t2", "stage = 2")];
    let (mut fx, windows) = stages_on(&profile(), &tasks);
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    let (_, spec) = propagate(&fx);
    assert_eq!(spec.check, None);
    let tier = spec.tier.expect("tier 2");
    assert_eq!((tier.tier, tier.stage), (2, 2));
    assert_eq!(tier.diff_base, BASE, "what the merge adds to stage 2");
    assert_eq!(tier.dir, fx.run().integration_path());
}

/// Conflicts `t1`'s propagate into stage 2 on `docs/shared.md`: the sync task `fix1`.
fn conflicted() -> (Fixture, Vec<(String, u32)>) {
    let (mut fx, windows) = stages(false);
    with_orchestrator(&mut fx);
    merge_t1(&mut fx, &windows);
    let (op, _) = propagate(&fx);
    let files = vec!["docs/shared.md".to_string()];
    fx.done(
        op,
        OpResult::Conflict {
            files,
            tree: Some(TREE.into()),
        },
    );
    (fx, windows)
}

#[test]
fn propagate_conflict_adds_a_sync_task_with_the_conflicted_tree() {
    let (mut fx, _) = conflicted();
    let fix = fx.task("fix1").clone();
    assert_eq!(fix.origin, TaskOrigin::Sync);
    assert_eq!(
        fix.fixes,
        Some(FixOf::Propagate {
            from: 1,
            to: 2,
            head: commit(1)
        })
    );
    assert_eq!(fix.spec.owns, ["docs/shared.md"]);
    assert_eq!(fix.stage(), 2);
    assert_eq!(fix.size, Size::M);
    assert_eq!(fix.test_mode, TestMode::Check);
    assert_eq!(
        fix.spec.test_mode_reason.as_deref(),
        Some("a merge resolution; the tiers check it")
    );
    assert_eq!(fix.spec.route, RouteSpec::default());
    assert_eq!(fix.spec.priority, 100);
    assert!(fix.spec.deps.is_empty());
    assert_eq!(fix.spec.epic, None);
    assert_eq!(fix.spec.title, "Resolve the merge of stage 1 into stage 2");
    assert_eq!(
        fix.spec.acceptance,
        [
            "no conflict markers remain",
            "both stages' changes are kept"
        ]
    );
    assert_eq!(
        fix.spec.brief,
        "[anthrex] Fix task fix1: merging stage 1 into stage 2 conflicted. Your worktree already holds that merge, with conflict markers in:\n\
         - docs/shared.md\n\
         Resolve every conflict so that both stages' work is kept, commit, and call task_done. Change nothing else."
    );
    assert_eq!(
        fix.sync,
        Some(SyncState {
            onto: commit(1),
            base_tree: TREE.into(),
            tasks: BTreeSet::from(["t1".to_string()]),
            handed_back: false,
            to_head: BASE.into(),
        })
    );
    assert_eq!(
        notes(&fx),
        ["propagate of stage 1 into stage 2 conflicted: added fix task fix1"]
    );
    assert_eq!(fx.run().stage(2).unwrap().head, BASE, "nothing landed");

    // Its worktree starts at the stage-2 head the conflict was found on, even when a
    // merge moved stage 2 meanwhile, and before its first session the engine hands the
    // stage-1 head back into it (M8a's hand-back).
    set_stage_head(fx.run_mut(), 2, &commit(9));
    fx.tick();
    let (op, kind) = pending_one(&fx, "PrepareWorktree", Some("fix1"));
    let OpKind::PrepareWorktree { from, .. } = kind else {
        unreachable!()
    };
    assert_eq!(from, BASE);
    let effects = fx.done(op, OpResult::Worktree { head: BASE.into() });
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    let (op, kind) = pending_one(&fx, "HandBack", Some("fix1"));
    assert_eq!(
        kind,
        OpKind::HandBack {
            worktree: super::dispatch::task_path("fix1"),
            run_head: commit(1),
            task_head: Some(BASE.into()),
            list_merged: false,
        }
    );
    let effects = fx.done(
        op,
        OpResult::HandedBack {
            files: vec!["docs/shared.md".into()],
            head: Some(BASE.into()),
            onto: Some(BASE.into()),
            merged: Vec::new(),
            merged_total: 0,
        },
    );
    let windows = ops_in(&effects, "CreateWindow");
    assert_eq!(windows.len(), 1, "{effects:#?}");
    let OpKind::CreateWindow { first_turn, .. } = &windows[0].1 else {
        unreachable!()
    };
    assert!(first_turn.contains("- docs/shared.md"), "{first_turn}");
    let fix = fx.task("fix1");
    assert!(fix.sync.as_ref().unwrap().handed_back);
    assert_eq!(fix.start_commit.as_deref(), Some(BASE));

    // Its claim's spill check is against the conflicted tree.
    let window = fx.launch_all();
    let window = window_of(&window, "fix1");
    let args = serde_json::json!({"summary": "resolved"});
    let effects = fx.tool_as(AgentRole::Worker, window, "fix1", "task_done", args);
    let (op, kind) = ops_in(&effects, "VerifyDone")[0].clone();
    let OpKind::VerifyDone { spill_base, .. } = kind else {
        unreachable!()
    };
    assert_eq!(spill_base.as_deref(), Some(TREE));
    let mut result = fx.clean_check("fix1");
    if let OpResult::DoneChecked { head, .. } = &mut result {
        *head = head_of_fix();
    }
    fx.done(op, result);
    fx.turn_completed(window);

    // Stage 1 moves on meanwhile: stage 2 is due, but waits for its sync task.
    set_stage_head(fx.run_mut(), 1, &commit(3));
    later(&mut fx, 1_000);
    assert!(propagates(&fx).is_empty(), "{:#?}", fx.run().pending_ops);

    // The sync task merges: stage 2 holds what stage 1 held at the conflict, and the
    // newer stage-1 head propagates.
    super::kinds_integration::merge_real(&mut fx, "fix1", &commit(4));
    assert_eq!(fx.task("fix1").state, TaskState::Merged);
    let stage = fx.run().stage(2).unwrap();
    assert!(stage.tasks_in.contains("t1"), "{:?}", stage.tasks_in);
    let (_, spec) = propagate(&fx);
    assert_eq!(
        (spec.from_head.as_str(), spec.expected_to_head.as_str()),
        (commit(3).as_str(), commit(4).as_str())
    );
    assert_eq!(fx.run().stage(2).unwrap().synced_from, Some(commit(1)));
}

fn head_of_fix() -> String {
    format!("f1{}", "d".repeat(38))
}

/// A sync task whose hand-back is lost in a restart gets it again when the run resumes.
#[test]
fn a_lost_sync_hand_back_is_sent_again_after_the_restart() {
    let (mut fx, _) = conflicted();
    fx.tick();
    let (op, _) = pending_one(&fx, "PrepareWorktree", Some("fix1"));
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    pending_one(&fx, "HandBack", Some("fix1"));
    restart(&mut fx, Vec::new());
    assert!(pending(&fx, "HandBack", Some("fix1")).is_empty());
    resume(&mut fx);
    let (_, kind) = pending_one(&fx, "HandBack", Some("fix1"));
    let OpKind::HandBack { run_head, .. } = kind else {
        unreachable!()
    };
    assert_eq!(run_head, commit(1));
    assert!(!fx.task("fix1").sync.as_ref().unwrap().handed_back);
}

#[test]
fn red_propagate_raises_attention_and_wakes() {
    let tasks = [doc_task("t1", ""), doc_task("t2", "stage = 2")];
    let (mut fx, windows) = stages_on(&profile(), &tasks);
    with_orchestrator(&mut fx);
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    let (op, _) = propagate(&fx);
    let count = fx.run().tasks.len();
    fx.done(
        op,
        OpResult::CandidateRed {
            code: Some(101),
            timed_out: false,
            tail: "FAILED".into(),
            secs: 3,
            tier: Some(Box::new(outcome(2, &["a::works", "b::works"]))),
        },
    );
    let stage = fx.run().stage(2).unwrap();
    assert_eq!(stage.propagate_red, Some(commit(1)));
    assert_eq!(stage.head, BASE, "nothing landed");
    let line = "propagate of stage 1 into stage 2 is red: a::works, b::works";
    assert!(
        attention(&fx).iter().any(|l| l == line),
        "{:?}",
        attention(&fx)
    );
    assert_eq!(
        notes(&fx),
        [
            "propagate of stage 1 into stage 2 is red: a::works, b::works; plan a fix in stage 1 or 2"
        ]
    );
    assert_eq!(fx.run().tasks.len(), count, "no fix task");
    // It is not retried on the same heads, even when they are read again (a
    // rebaseline that finds them where they were).
    later(&mut fx, 1_000);
    set_stage_head(fx.run_mut(), 1, &commit(1));
    later(&mut fx, 1_000);
    assert!(propagates(&fx).is_empty());
    // Stage 1's head moves: the propagate is due again, and the line goes.
    set_stage_head(fx.run_mut(), 1, &commit(3));
    fx.tick();
    let (_, spec) = propagate(&fx);
    assert_eq!(spec.from_head, commit(3));
    assert!(
        !attention(&fx).iter().any(|l| l == line),
        "{:?}",
        attention(&fx)
    );
}

/// Decision 52 for an untiered profile: M8a's check red on the propagate, which names
/// no tests (the text is invented).
#[test]
fn an_untiered_red_propagate_names_the_check() {
    let (mut fx, windows) = stages(false);
    merge_t1(&mut fx, &windows);
    let (op, _) = propagate(&fx);
    fx.done(
        op,
        OpResult::CandidateRed {
            code: Some(1),
            timed_out: false,
            tail: "FAILED".into(),
            secs: 3,
            tier: None,
        },
    );
    let line = "propagate of stage 1 into stage 2 is red: the check failed";
    assert!(
        attention(&fx).iter().any(|l| l == line),
        "{:?}",
        attention(&fx)
    );
    // A merge into stage 2 moves its head: the propagate is due again.
    set_stage_head(fx.run_mut(), 2, &commit(5));
    fx.tick();
    let (_, spec) = propagate(&fx);
    assert_eq!(spec.expected_to_head, commit(5));
}

#[test]
fn propagate_is_journaled_and_reconciled() {
    let (mut fx, windows) = stages(false);
    merge_t1(&mut fx, &windows);
    let (_, spec) = propagate(&fx);
    let kind = OpKind::Propagate(Box::new(spec));
    let text = serde_json::to_string(&kind).unwrap();
    assert_eq!(serde_json::from_str::<OpKind>(&text).unwrap(), kind);
    let conflict = OpResult::Conflict {
        files: vec!["a".into()],
        tree: Some(TREE.into()),
    };
    let text = serde_json::to_string(&conflict).unwrap();
    assert_eq!(serde_json::from_str::<OpResult>(&text).unwrap(), conflict);
    let old: OpResult =
        serde_json::from_value(serde_json::json!({"Conflict": {"files": []}})).unwrap();
    assert_eq!(
        old,
        OpResult::Conflict {
            files: vec![],
            tree: None
        }
    );
    // A propagate that reached neither ref (reconciled `NotStarted`) is issued again
    // once the run resumes.
    let before = fx.run().clone();
    restart(&mut fx, Vec::new());
    assert!(propagates(&fx).is_empty());
    assert_eq!(fx.run().propagate_due, BTreeSet::from([2]));
    resume(&mut fx);
    let (_, spec) = propagate(&fx);
    assert_eq!(spec.from_head, commit(1));
    // One whose commit reached both refs is replayed as `Merged`, and lands.
    fx.state.runs.insert(RUN_ID.into(), before);
    let (op, _) = propagate(&fx);
    restart(&mut fx, vec![(op, merged_at(&commit(2)))]);
    assert_eq!(fx.run().stage_head(2), Some(commit(2).as_str()));
    assert_eq!(fx.run().stage(2).unwrap().synced_from, Some(commit(1)));
    assert!(fx.run().propagate_due.is_empty());
    resume(&mut fx);
    assert!(propagates(&fx).is_empty());
}

/// Milestone 9.1 decision 17(b) with a propagate: the queue is not idle while one is
/// due or in flight.
#[test]
fn a_due_or_running_propagate_keeps_the_queue_busy() {
    let tasks = [doc_task("t1", ""), doc_task("t2", "stage = 2")];
    let (mut fx, windows) = stages_on(&profile(), &tasks);
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    propagate(&fx);
    assert_eq!(fx.run().queue_idle_since, None);
    let effects = later(&mut fx, 1_000);
    assert!(super::full::full_jobs(&effects).is_empty(), "{effects:#?}");
}

/// Task M9.1.12's open item: an epic's integration review waits until its highest
/// stage holds every merged task of the epic.
#[test]
fn an_epic_review_waits_for_its_highest_stage_to_hold_every_merge() {
    use super::kinds_integration::merge_real;
    use super::orch::{add, edit_plan, launched};
    use super::planners::{PLANNER, planner_started, spawn, submit_epic, task_in};
    use serde_json::json;
    let mut fx = launched(true);
    let mut t2 = add("t2", "docs");
    t2["task"]["stage"] = json!(2);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth"), t2], "submit": true}),
    );
    spawn(&mut fx, "mail");
    planner_started(&mut fx, PLANNER);
    let mut m2 = task_in("m2", "mail");
    m2["task"]["stage"] = json!(2);
    submit_epic(&mut fx, json!([task_in("m1", "mail"), m2]));
    while let Some((op, _)) = pending(&fx, "CreateStageBranch", None).first().cloned() {
        fx.done(op, OpResult::StageCreated);
    }
    assert_eq!(fx.run().stages.len(), 2);
    merge_real(&mut fx, "m1", &commit(1));
    // m1's propagate into stage 2 is red, so stage 2 does not hold it.
    let (op, _) = propagate(&fx);
    let red = OpResult::CandidateRed {
        code: Some(1),
        timed_out: false,
        tail: String::new(),
        secs: 1,
        tier: None,
    };
    fx.done(op, red);
    merge_real(&mut fx, "m2", &commit(2));
    fx.tick();
    assert!(fx.run().task("mail-int1").is_none(), "stage 2 lacks m1");
    // Stage 1 moves on and propagates: stage 2 now holds m1, and the review comes.
    set_stage_head(fx.run_mut(), 1, &commit(3));
    fx.tick();
    let (op, _) = propagate(&fx);
    fx.done(op, merged_at(&commit(4)));
    let review = fx.run().task("mail-int1").expect("the integration review");
    assert_eq!(review.stage(), 2);
    let target = review.spec.review_target.clone().unwrap();
    assert!(target.ends_with(&format!("..{}", commit(4))), "{target}");
}
