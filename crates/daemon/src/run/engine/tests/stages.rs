//! Milestone 9.1 task M9.1.12: stage heads, stage branches, the guard list and the
//! layout fixed at approval (decisions 46–48, 53), on the engine fixture.

use proto::{RunState, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::kinds_integration::merge_real;
use super::merge::window_of;
use super::merge::{commit, config, doc_task, merge, pending, pending_one, start, to_queue};
use super::orch::{add, edit_plan, launched};
use super::planners::{PLANNER, planner_started, spawn, submit_epic, task_in};
use crate::run::engine::stages::{Rebaseline, ready_in_stage, set_stage_head};
use crate::run::engine::{Effect, EngineState, EventKind, OpKind, OpResult};
use crate::run::model::{Run, StageLayout};

fn integration() -> String {
    format!("anthrex/{RUN_ID}/integration")
}

fn stage_branch(n: u16) -> String {
    format!("anthrex/{RUN_ID}/stage-{n}")
}

/// The pending `CreateStageBranch`es, as `(op, branch, from)`.
fn creates(fx: &Fixture) -> Vec<(u64, String, String)> {
    pending(fx, "CreateStageBranch", None)
        .into_iter()
        .map(|(op, kind)| match kind {
            OpKind::CreateStageBranch { branch, from, .. } => (op, branch, from),
            _ => unreachable!(),
        })
        .collect()
}

/// Answers the one pending `CreateStageBranch`, which must be `branch` from `from`.
fn create(fx: &mut Fixture, branch: &str, from: &str) -> Vec<Effect> {
    let pending = creates(fx);
    assert_eq!(
        pending,
        vec![(pending[0].0, branch.to_string(), from.to_string())],
        "{:#?}",
        fx.run().stages
    );
    fx.done(pending[0].0, OpResult::StageCreated)
}

/// A running `--yes` run of `tasks` whose stage 1 is created; every worker launched.
fn multi(tasks: &[String]) -> Fixture {
    let (mut fx, _) = start(tasks);
    assert_eq!(fx.run().stage_layout, StageLayout::Multi);
    assert!(fx.run().stages.is_empty(), "no stage before its branch");
    create(&mut fx, &stage_branch(1), BASE);
    fx
}

#[test]
fn single_stage_run_has_no_stage_refs() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    let run = fx.run();
    assert_eq!(run.stage_layout, StageLayout::Single);
    assert_eq!(run.stage_branch(1), integration());
    assert_eq!(run.stages.len(), 1);
    assert_eq!(run.stages[0].branch, integration());
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (_, kind) = pending_one(&fx, "MergeCandidate", Some("t1"));
    let OpKind::MergeCandidate {
        run_branch,
        guarded,
        also_integration,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(run_branch, integration());
    // M8a's guard: the base and `integration`, nothing else.
    assert_eq!(guarded, vec![(integration(), BASE.to_string())]);
    assert!(!also_integration);
    merge(&mut fx, "t1", &commit(1));
    fx.tick();
    let (_, verify) = pending_one(&fx, "VerifyRefs", None);
    let OpKind::VerifyRefs { guarded, .. } = verify else {
        unreachable!()
    };
    assert_eq!(guarded, vec![(integration(), commit(1))]);
    assert!(fx.ops("CreateStageBranch").is_empty(), "{:#?}", fx.log);
    let run = fx.run();
    assert_eq!(run.run_head, commit(1));
    assert_eq!(run.stages.len(), 1);
    assert_eq!(run.stages[0].head, commit(1));
    assert!(run.stages[0].tasks_in.contains("t1"));
    // `run.json` round-trips with its one stage.
    let back: Run = serde_json::from_str(&serde_json::to_string(run).unwrap()).unwrap();
    assert_eq!(&back, run);
    assert_eq!(back.stages.len(), 1);
}

#[test]
fn set_stage_head_moves_run_head_only_for_the_highest_stage() {
    let mut fx = multi(&[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    create(&mut fx, &stage_branch(2), BASE);
    let run = fx.run_mut();
    set_stage_head(run, 1, &commit(1));
    assert_eq!(run.stage_head(1), Some(commit(1).as_str()));
    assert_eq!(run.run_head, BASE, "stage 1 is not the highest");
    set_stage_head(run, 2, &commit(2));
    assert_eq!(run.stage_head(2), Some(commit(2).as_str()));
    assert_eq!(run.run_head, commit(2));
}

#[test]
fn run_head_is_the_highest_stage_head() {
    let mut fx = multi(&[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    assert_eq!(fx.run().run_head, BASE);
    assert_eq!(fx.run().stage_head(2), None, "not created yet");
    // Stage 2 is created from stage 1's head; it is now the highest, and `run_head`
    // (the `integration` alias) is its head, whatever moves below it.
    create(&mut fx, &stage_branch(2), BASE);
    let run = fx.run_mut();
    assert_eq!(run.stage(2).unwrap().synced_from, Some(BASE.to_string()));
    assert_eq!(run.stage_branch(2), stage_branch(2));
    set_stage_head(run, 2, &commit(2));
    set_stage_head(run, 1, &commit(1));
    assert_eq!(run.run_head, commit(2));
    assert_eq!(run.run_head, run.stage_head(2).unwrap());
}

#[test]
fn stage_branch_is_created_from_the_lower_stage_head_when_its_first_task_is_ready() {
    let (mut fx, windows) = start(&[
        doc_task("t1", ""),
        doc_task("t2", "stage = 2\ndeps = [\"t1\"]"),
    ]);
    // At approval, stage 1 from the run head, and nothing else.
    assert_eq!(
        creates(&fx),
        vec![(creates(&fx)[0].0, stage_branch(1), BASE.into())]
    );
    assert!(windows.is_empty(), "no task starts before its stage exists");
    create(&mut fx, &stage_branch(1), BASE);
    let windows = fx.launch_all();
    assert!(
        creates(&fx).is_empty(),
        "t2 is not ready: its dependency is not merged"
    );
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (_, kind) = pending_one(&fx, "MergeCandidate", Some("t1"));
    let OpKind::MergeCandidate {
        run_branch,
        expected_run_head,
        also_integration,
        guarded,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(run_branch, stage_branch(1));
    assert_eq!(expected_run_head, BASE);
    assert!(also_integration, "stage 1 is the highest stage");
    assert_eq!(
        guarded,
        vec![(stage_branch(1), BASE.into()), (integration(), BASE.into())]
    );
    merge(&mut fx, "t1", &commit(1));
    fx.tick();
    assert_eq!(fx.run().stage(1).unwrap().head, commit(1));
    assert_eq!(fx.run().run_head, commit(1));
    // t2 would run in stage 2 now: stage 2 from stage 1's head, once.
    create(&mut fx, &stage_branch(2), &commit(1));
    fx.tick();
    assert_eq!(fx.ops("CreateStageBranch").len(), 2);
    assert!(fx.run().stage(2).unwrap().tasks_in.contains("t1"));
    let prepares = fx.ops("PrepareWorktree");
    let (_, last) = prepares.last().unwrap();
    assert_eq!(op_task(last), "t2");
    let OpKind::PrepareWorktree { from, .. } = last else {
        unreachable!()
    };
    assert_eq!(from, &commit(1));
}

#[test]
fn tasks_in_a_stage_wait_for_their_branch() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    assert!(windows.is_empty());
    let i = fx.run().tasks.iter().position(|t| t.id() == "t2").unwrap();
    assert!(!ready_in_stage(fx.run(), i));
    assert_eq!(fx.task("t2").state, TaskState::Pending);
    create(&mut fx, &stage_branch(1), BASE);
    // Stage 1 exists; t2 has no dependency, so stage 2 follows at once, from it.
    assert!(!ready_in_stage(fx.run(), i));
    assert!(
        fx.run()
            .pending_ops
            .values()
            .all(|p| p.task_id.as_deref() != Some("t2")),
        "{:#?}",
        fx.run().pending_ops
    );
    create(&mut fx, &stage_branch(2), BASE);
    assert!(ready_in_stage(fx.run(), i));
    let starts = pending(&fx, "PrepareWorktree", Some("t2"));
    assert_eq!(starts.len(), 1, "{:#?}", fx.run().pending_ops);
}

#[test]
fn a_merge_into_a_lower_stage_moves_only_its_own_ref() {
    let mut fx = multi(&[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    create(&mut fx, &stage_branch(2), BASE);
    let windows = fx.launch_all();
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (_, kind) = pending_one(&fx, "MergeCandidate", Some("t1"));
    let OpKind::MergeCandidate {
        run_branch,
        also_integration,
        guarded,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(run_branch, stage_branch(1));
    assert!(!also_integration);
    assert_eq!(
        guarded,
        vec![
            (stage_branch(1), BASE.into()),
            (stage_branch(2), BASE.into()),
            (integration(), BASE.into())
        ]
    );
    merge(&mut fx, "t1", &commit(1));
    let run = fx.run();
    assert_eq!(run.stage_head(1), Some(commit(1).as_str()));
    assert_eq!(run.stage_head(2), Some(BASE));
    assert_eq!(run.run_head, BASE, "integration stays with stage 2");
    assert_eq!(run.last_green_candidate, None);
    assert_eq!(run.stage(1).unwrap().last_green_candidate, Some(commit(1)));
}

#[test]
fn the_layout_is_fixed_at_the_gate_and_a_single_run_keeps_one_stage() {
    // Approved at the gate with a stage-2 task: `Multi`.
    let plan = plan_with(PROFILE, &[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    let mut fx = Fixture::with_config(&plan, config());
    fx.ready(false);
    assert_eq!(fx.run().stage_layout, StageLayout::Single, "not fixed yet");
    assert_eq!(fx.run().stages.len(), 1);
    fx.approve();
    assert_eq!(fx.run().stage_layout, StageLayout::Multi);
    assert_eq!(creates(&fx).len(), 1);

    // Approved with one stage: an edit naming stage 2 is refused.
    let (mut fx, _) = start(&[doc_task("t1", "")]);
    assert_eq!(fx.run().stage_layout, StageLayout::Single);
    let mut t9 = fx.task("t1").spec.clone();
    t9.id = "t9".into();
    t9.owns = vec!["docs/t9/**".into()];
    t9.stage = 2;
    let effects = edit(&mut fx, vec![proto::PlanEdit::AddTask { task: t9 }]);
    let replies = replies(&effects);
    let [Err(text)] = &replies[..] else {
        panic!("{replies:?}")
    };
    assert!(
        text.contains("this run was approved with one stage; its tasks stay in stage 1"),
        "{text}"
    );
    assert!(fx.run().task("t9").is_none());
}

#[test]
fn m8a_run_json_restores_as_a_single_stage() {
    let run: Run = serde_json::from_str(include_str!("m8b_run.json")).unwrap();
    assert!(run.stages.is_empty());
    let head = run.run_head.clone();
    let id = run.id.clone();
    let mut fx = Fixture::new(PROFILE);
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        runs: vec![run],
        replay: Vec::new(),
        held: Vec::new(),
    });
    let run = &fx.state.runs[&id];
    assert_eq!(run.stage_layout, StageLayout::Single);
    assert_eq!(run.stages.len(), 1);
    assert_eq!(run.stages[0].n, 1);
    assert_eq!(run.stages[0].head, head);
    assert_eq!(run.stages[0].branch, run.run_branch());
    assert_eq!(run.stage_head(1), Some(head.as_str()));
}

#[test]
fn a_multi_run_rebaselines_every_stage_head() {
    let mut fx = multi(&[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    create(&mut fx, &stage_branch(2), BASE);
    let run = fx.run_mut();
    run.state = RunState::Halted;
    run.halted_reason = Some("moved".into());
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some(Rebaseline {
            base: BASE.into(),
            head: commit(2),
            stages: vec![(1, commit(1)), (2, commit(2))],
            salvaged: None,
        }),
    });
    let run = fx.run();
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.stage_head(1), Some(commit(1).as_str()));
    assert_eq!(run.stage_head(2), Some(commit(2).as_str()));
    assert_eq!(run.run_head, commit(2));
}

/// Controller ruling C-14 (c): an epic's integration review goes into the highest stage
/// holding one of its tasks, and reviews up to that stage's head.
#[test]
fn an_epic_integration_review_goes_into_its_highest_stage() {
    let mut fx = launched(true);
    let mut t2 = add("t2", "docs");
    t2["task"]["stage"] = json!(2);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth"), t2], "submit": true}),
    );
    assert_eq!(fx.run().stage_layout, StageLayout::Multi);
    spawn(&mut fx, "mail");
    planner_started(&mut fx, PLANNER);
    let mut m2 = task_in("m2", "mail");
    m2["task"]["stage"] = json!(2);
    let effects = submit_epic(&mut fx, json!([task_in("m1", "mail"), m2]));
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    create(&mut fx, &stage_branch(1), BASE);
    create(&mut fx, &stage_branch(2), BASE);
    merge_real(&mut fx, "t1", &commit(1));
    merge_real(&mut fx, "m1", &commit(2));
    merge_real(&mut fx, "t2", &commit(3));
    merge_real(&mut fx, "m2", &commit(4));
    let review = fx
        .run()
        .task("mail-int1")
        .expect("the integration review")
        .clone();
    assert_eq!(review.stage(), 2);
    let target = review.spec.review_target.unwrap();
    assert!(target.ends_with(&format!("..{}", commit(4))), "{target}");
}

/// A stage whose tasks were all cancelled is created empty once a later stage has
/// work, so the stages above it are never stranded.
#[test]
fn a_stage_of_cancelled_tasks_does_not_strand_the_stages_above() {
    let mut fx = multi(&[
        doc_task("t1", ""),
        doc_task("t2", "stage = 2\ndeps = [\"t1\"]"),
        doc_task("t3", "stage = 3"),
    ]);
    assert!(creates(&fx).is_empty(), "t2 waits for t1; t3 for stage 2");
    fx.task_mut("t2").state = TaskState::Cancelled;
    fx.tick();
    create(&mut fx, &stage_branch(2), BASE);
    create(&mut fx, &stage_branch(3), BASE);
    assert_eq!(fx.run().stages.len(), 3);
    assert_eq!(pending(&fx, "PrepareWorktree", Some("t3")).len(), 1);
}

/// The journal carries the new op and result, and an intent journaled before them (no
/// `guarded`, no `also_integration`) still loads, as M8a's guard.
#[test]
fn stage_ops_round_trip_through_the_journal() {
    let create = OpKind::CreateStageBranch {
        root: "/tmp/x".into(),
        branch: stage_branch(2),
        from: BASE.into(),
    };
    let text = serde_json::to_string(&create).unwrap();
    assert_eq!(serde_json::from_str::<OpKind>(&text).unwrap(), create);
    let created = OpResult::StageCreated;
    let text = serde_json::to_string(&created).unwrap();
    assert_eq!(serde_json::from_str::<OpResult>(&text).unwrap(), created);
    let old = serde_json::json!({"VerifyRefs": {
        "root": "/tmp/x", "base_branch": "main", "expected_base": BASE,
        "run_branch": integration(), "expected_run_head": BASE
    }});
    let OpKind::VerifyRefs { guarded, .. } = serde_json::from_value(old).unwrap() else {
        unreachable!()
    };
    assert!(guarded.is_empty());
    let old = serde_json::json!({"MergeCandidate": {
        "root": "/tmp/x", "integration": "/tmp/i", "run_branch": integration(),
        "expected_run_head": BASE, "base_branch": "main", "expected_base": BASE,
        "task_head": HEAD, "message": "m", "check": null, "timeout_secs": 1, "env": []
    }});
    let OpKind::MergeCandidate {
        guarded,
        also_integration,
        ..
    } = serde_json::from_value(old).unwrap()
    else {
        unreachable!()
    };
    assert!(guarded.is_empty() && !also_integration);
}

/// Controller ruling C-15 (I-1): after a rebaseline of a `Multi` run, `run_head` is the
/// highest stage's head, the guard expects every adopted head, and the log names the
/// run head it set, the commit `integration` was moved back from and its salvage ref.
#[test]
fn a_multi_rebaseline_guards_the_adopted_heads_and_logs_them() {
    let mut fx = multi(&[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    create(&mut fx, &stage_branch(2), BASE);
    let run = fx.run_mut();
    run.state = RunState::Halted;
    run.halted_reason = Some("moved".into());
    let x = commit(9);
    let salvage = format!("refs/anthrex/salvage/{RUN_ID}/_integration-7");
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some(Rebaseline {
            base: BASE.into(),
            head: commit(2),
            stages: vec![(1, commit(1)), (2, commit(2))],
            salvaged: Some((x.clone(), salvage.clone())),
        }),
    });
    let run = fx.run();
    assert_eq!(run.run_head, commit(2));
    assert_eq!(
        crate::run::engine::stages::guard_list(run),
        vec![
            (stage_branch(1), commit(1)),
            (stage_branch(2), commit(2)),
            (integration(), commit(2))
        ]
    );
    let line = &run.log.last().unwrap().text;
    assert!(
        line.contains(&format!("run head {}", &commit(2)[..7])),
        "{line}"
    );
    assert!(
        line.contains(&format!(
            "{} moved back from {} to {} ({} kept at {salvage})",
            integration(),
            &x[..7],
            &commit(2)[..7],
            &x[..7]
        )),
        "{line}"
    );
}

/// The review's log finding: the text names the `run_head` the rebaseline set, never
/// the `integration` head the driver read.
#[test]
fn a_multi_rebaseline_logs_the_run_head_it_set() {
    let mut fx = multi(&[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    create(&mut fx, &stage_branch(2), BASE);
    fx.run_mut().state = RunState::Halted;
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some(Rebaseline {
            base: BASE.into(),
            head: commit(8),
            stages: vec![(1, commit(1)), (2, commit(2))],
            salvaged: None,
        }),
    });
    let line = &fx.run().log.last().unwrap().text;
    assert!(
        line.contains(&format!("run head {}", &commit(2)[..7])),
        "{line}"
    );
    assert!(!line.contains(&commit(8)[..7]), "{line}");
}

/// Controller ruling C-15 (M-5): an epic task that finished without merging (here a
/// cancelled stage-2 task) does not pull the integration review into its stage.
#[test]
fn an_epic_review_ignores_a_cancelled_tasks_stage() {
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
    let effects = submit_epic(&mut fx, json!([task_in("m1", "mail"), m2]));
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    create(&mut fx, &stage_branch(1), BASE);
    create(&mut fx, &stage_branch(2), BASE);
    fx.task_mut("m2").state = TaskState::Cancelled;
    merge_real(&mut fx, "t1", &commit(1));
    merge_real(&mut fx, "m1", &commit(2));
    let review = fx.run().task("mail-int1").expect("the review").clone();
    assert_eq!(review.stage(), 1);
    let target = review.spec.review_target.unwrap();
    assert!(target.ends_with(&format!("..{}", commit(2))), "{target}");
}
