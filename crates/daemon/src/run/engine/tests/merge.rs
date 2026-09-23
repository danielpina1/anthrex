//! M8a.14: decision 36's merge queue — one candidate at a time in arrival order, the
//! merged task's clean-up, the conflict hand-back and its re-queue, the second conflict
//! and a red candidate. The ref guard, completion, `finish` and `cancel` are in
//! `merge_complete.rs`. Every sequence ends with the liveness check, extended to the
//! merge queue and the run's own states.

use proto::{AgentRole, BlockReason, TaskState};
use serde_json::json;

use super::dispatch::{replies, task_path};
use super::fixture::*;
use super::gates::check_result;
use super::holds::delivers;
use super::turns_fixes::assert_alive;
use crate::run::contract::{DONE_ACCEPTED, candidate_red_message, conflict_message};
use crate::run::engine::{AgentSignal, Effect, OpKind, OpResult};
use crate::run::model::{CheckRecord, OpId};

/// A check-mode task's extra lines.
const CHECK_MODE: &str = "test_mode = \"check\"\ntest_mode_reason = \"glue code\"";

/// `review.small = "off"`: an S task that is not a hub is not reviewed, so a check-mode
/// task's only gate before the merge queue is the check.
pub(super) fn config() -> config::Orchestrator {
    config::Orchestrator {
        review_small: false,
        ..config::Orchestrator::default()
    }
}

/// A check-mode S task owning `docs/<id>/**`, with `extra` TOML lines.
pub(super) fn doc_task(id: &str, extra: &str) -> String {
    task_toml(
        id,
        "S",
        &format!("[\"docs/{id}/**\"]"),
        &format!("{CHECK_MODE}\n{extra}"),
    )
}

/// A running run of `tasks` on `profile`, every dispatched worker launched.
pub(super) fn start_on(profile: &str, tasks: &[String]) -> (Fixture, Vec<(String, u32)>) {
    let plan = plan_with(profile, tasks);
    let mut fx = Fixture::with_config(&plan, config());
    fx.ready(true);
    let windows = fx.launch_all();
    (fx, windows)
}

pub(super) fn start(tasks: &[String]) -> (Fixture, Vec<(String, u32)>) {
    start_on(PROFILE, tasks)
}

/// The window of `id` among `windows`.
pub(super) fn window_of(windows: &[(String, u32)], id: &str) -> u32 {
    windows
        .iter()
        .find(|(t, _)| t == id)
        .unwrap_or_else(|| panic!("no window for {id}: {windows:?}"))
        .1
}

/// The commit task `id` claims (40 characters, distinct per task).
pub(super) fn head_of(id: &str) -> String {
    format!("{id}{}", "c".repeat(40 - id.len()))
}

/// A merge commit, distinct per `n`.
pub(super) fn commit(n: u32) -> String {
    format!("{n}{}", "e".repeat(40 - n.to_string().len()))
}

/// The pending ops of `name` for `task` (`None`: run-level ops).
pub(super) fn pending(fx: &Fixture, name: &str, task: Option<&str>) -> Vec<(OpId, OpKind)> {
    fx.run()
        .pending_ops
        .values()
        .filter(|p| op_name(&p.kind) == name && p.task_id.as_deref() == task)
        .map(|p| (p.op, p.kind.clone()))
        .collect()
}

/// The only pending op of `name` for `task`.
pub(super) fn pending_one(fx: &Fixture, name: &str, task: Option<&str>) -> (OpId, OpKind) {
    let ops = pending(fx, name, task);
    assert_eq!(ops.len(), 1, "one pending {name} for {task:?}: {ops:#?}");
    ops[0].clone()
}

/// `id` claims done with `head` from `window`, and the claim is accepted; the worker
/// ends its turn. Returns the effects of the accepting step.
pub(super) fn claim(fx: &mut Fixture, id: &str, window: u32, head: &str) -> Vec<Effect> {
    let args = json!({"summary": "done"});
    let effects = fx.tool_as(AgentRole::Worker, window, id, "task_done", args);
    let (op, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let mut result = fx.clean_check(id);
    if let OpResult::DoneChecked { head: h, .. } = &mut result {
        *h = head.to_string();
    }
    let effects = fx.done(op, result);
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);
    fx.turn_completed(window);
    effects
}

/// `id` claims done with its own head and its check passes: it reaches the merge queue.
/// Returns the effects of the check's step.
pub(super) fn to_queue(fx: &mut Fixture, id: &str, window: u32) -> Vec<Effect> {
    claim(fx, id, window, &head_of(id));
    let (op, _) = pending_one(fx, "Check", Some(id));
    fx.done(op, check_result(true))
}

/// The pending `MergeCandidate`'s op and task head.
pub(super) fn candidate(fx: &Fixture, id: &str) -> (OpId, String) {
    match pending_one(fx, "MergeCandidate", Some(id)) {
        (op, OpKind::MergeCandidate { task_head, .. }) => (op, task_head),
        _ => unreachable!(),
    }
}

/// The tasks of every `MergeCandidate` among `effects`, in order.
fn candidates_in(fx: &Fixture, effects: &[Effect]) -> Vec<String> {
    ops_in(effects, "MergeCandidate")
        .iter()
        .map(|(_, kind)| match kind {
            OpKind::MergeCandidate { task_head, .. } => fx
                .run()
                .tasks
                .iter()
                .find(|t| t.head.as_deref() == Some(task_head))
                .map(|t| t.spec.id.clone())
                .unwrap_or_default(),
            _ => unreachable!(),
        })
        .collect()
}

/// Merges `id` at `at`, then completes every worktree removal that follows.
pub(super) fn merge(fx: &mut Fixture, id: &str, at: &str) -> Vec<Effect> {
    let (op, _) = candidate(fx, id);
    let mut effects = fx.done(op, OpResult::Merged { commit: at.into() });
    for (op, _) in pending(fx, "RemoveWorktree", Some(id)) {
        effects.extend(fx.done(op, OpResult::Removed { salvage_ref: None }));
    }
    effects
}

#[test]
fn merges_are_one_at_a_time_in_arrival_order() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", ""), doc_task("t3", "")]);
    assert_eq!(windows.len(), 3);
    let effects = to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    let int = task_path("integration");
    let merges = ops_in(&effects, "MergeCandidate");
    assert_eq!(merges.len(), 1, "{effects:#?}");
    assert_eq!(
        merges[0].1,
        OpKind::MergeCandidate {
            root: "/tmp/x".into(),
            integration: int.clone(),
            run_branch: format!("anthrex/{RUN_ID}/integration"),
            expected_run_head: BASE.into(),
            base_branch: "main".into(),
            expected_base: BASE.into(),
            task_head: head_of("t2"),
            message: "anthrex: merge t2: Title t2".into(),
            check: Some("cargo test".into()),
            timeout_secs: 1800,
            env: vec![("TARGET".into(), format!("{}/target", int.display()))],
        }
    );
    // Width 1: the others wait, in the order they arrived.
    let effects = to_queue(&mut fx, "t3", window_of(&windows, "t3"));
    assert!(
        ops_in(&effects, "MergeCandidate").is_empty(),
        "{effects:#?}"
    );
    let effects = to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    assert!(
        ops_in(&effects, "MergeCandidate").is_empty(),
        "{effects:#?}"
    );
    assert_eq!(fx.run().merge_queue, vec!["t2", "t3", "t1"]);
    assert_alive(&fx);

    let effects = merge(&mut fx, "t2", &commit(2));
    assert_eq!(candidates_in(&fx, &effects), vec!["t3"]);
    let (op, _) = candidate(&fx, "t3");
    let OpKind::MergeCandidate {
        expected_run_head, ..
    } = pending_one(&fx, "MergeCandidate", Some("t3")).1
    else {
        unreachable!()
    };
    assert_eq!(expected_run_head, commit(2));
    assert_alive(&fx);
    let effects = fx.done(op, OpResult::Merged { commit: commit(3) });
    assert_eq!(candidates_in(&fx, &effects), vec!["t1"]);
    assert_eq!(fx.run().merge_queue, vec!["t1"]);
    assert_alive(&fx);
}

#[test]
fn merged_updates_run_head_cleans_up_and_retires_the_worker() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "deps = [\"t1\"]")]);
    let window = window_of(&windows, "t1");
    to_queue(&mut fx, "t1", window);
    assert_eq!(fx.task("t2").state, TaskState::Pending);
    let (op, task_head) = candidate(&fx, "t1");
    assert_eq!(task_head, head_of("t1"));
    let effects = fx.done(op, OpResult::Merged { commit: commit(1) });
    let run = fx.run();
    assert_eq!(run.run_head, commit(1));
    assert_eq!(run.last_green_candidate, Some(commit(1)));
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.merge_commit, Some(commit(1)));
    assert!(fx.run().merge_queue.is_empty());

    let salvage = |n: u32| format!("refs/anthrex/salvage/{RUN_ID}/t1/{n}");
    let removals: Vec<(std::path::PathBuf, String)> = ops_in(&effects, "RemoveWorktree")
        .into_iter()
        .map(|(_, k)| match k {
            OpKind::RemoveWorktree {
                root,
                path,
                salvage_ref,
            } => {
                assert_eq!(root, std::path::PathBuf::from("/tmp/x"));
                (path, salvage_ref)
            }
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        removals,
        vec![
            (task_path("t1"), salvage(1)),
            (task_path("t1.review"), salvage(2)),
            (task_path("t1.proof"), salvage(3)),
        ]
    );
    let position = |pred: &dyn Fn(&Effect) -> bool| effects.iter().position(pred);
    let unwatch = position(&|e| {
        *e == Effect::UnwatchWorktree {
            root: task_path("t1"),
        }
    })
    .expect("the task worktree is unwatched");
    let first_removal = position(&|e| {
        matches!(
            e,
            Effect::Op {
                kind: OpKind::RemoveWorktree { .. },
                ..
            }
        )
    })
    .unwrap();
    assert!(unwatch < first_removal, "{effects:#?}");
    assert!(effects.contains(&Effect::RetireWindow { window_id: window }));
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
    assert!(fx.task("t1").rounds[0].retiring);

    // Its dependent is runnable, and dispatched from the new run head.
    assert_eq!(fx.task("t2").state, TaskState::Preparing);
    let prepares = ops_in(&effects, "PrepareWorktree");
    assert!(
        prepares
            .iter()
            .any(|(_, k)| matches!(k, OpKind::PrepareWorktree { from, .. } if *from == commit(1))),
        "{effects:#?}"
    );
    // Only the task's own worktree clears `worktree_live`.
    for (op, kind) in pending(&fx, "RemoveWorktree", Some("t1")) {
        if matches!(&kind, OpKind::RemoveWorktree { path, .. } if *path != task_path("t1")) {
            fx.done(op, OpResult::Removed { salvage_ref: None });
        }
    }
    assert!(fx.task("t1").worktree_live);
    let (op, _) = pending_one(&fx, "RemoveWorktree", Some("t1"));
    fx.done(
        op,
        OpResult::Removed {
            salvage_ref: Some(salvage(1)),
        },
    );
    assert!(!fx.task("t1").worktree_live);
    assert_eq!(fx.task("t1").salvage_refs, vec![salvage(1)]);
    assert_alive(&fx);
}

/// Decision 20's `<seq>`: worktrees removed together take consecutive numbers and only
/// the dirty ones record theirs, so the next number follows the highest one recorded.
#[test]
fn salvage_numbers_follow_the_highest_recorded_ref() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let salvage = |n: u32| format!("refs/anthrex/salvage/{RUN_ID}/t1/{n}");
    fx.task_mut("t1").salvage_refs = vec![salvage(3)];
    let (op, _) = candidate(&fx, "t1");
    let effects = fx.done(op, OpResult::Merged { commit: commit(1) });
    let refs: Vec<String> = ops_in(&effects, "RemoveWorktree")
        .into_iter()
        .map(|(_, k)| match k {
            OpKind::RemoveWorktree { salvage_ref, .. } => salvage_ref,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(refs, vec![salvage(4), salvage(5), salvage(6)]);
}

/// The first conflict hands the run head back; the worker's merge and its next
/// accepted `task_done` go straight back to the merge queue, with no gate op.
#[test]
fn first_conflict_hands_back_then_requeues_after_task_done() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    let window = window_of(&windows, "t1");
    to_queue(&mut fx, "t1", window);
    let (op, _) = candidate(&fx, "t1");
    let files = vec!["docs/t1/a.md".to_string(), "docs/t1/ü b.md".to_string()];
    let effects = fx.done(
        op,
        OpResult::Conflict {
            files: files.clone(),
        },
    );
    let hand_backs = ops_in(&effects, "HandBack");
    assert_eq!(
        hand_backs
            .iter()
            .map(|(_, k)| k.clone())
            .collect::<Vec<_>>(),
        vec![OpKind::HandBack {
            worktree: task_path("t1"),
            run_head: BASE.into(),
            task_head: Some(head_of("t1")),
        }]
    );
    let t1 = fx.task("t1");
    assert_eq!((t1.conflicts, t1.failures), (1, 0));
    assert_eq!(t1.state, TaskState::MergeQueue);
    assert!(fx.run().merge_queue.is_empty());
    assert!(delivers(&effects).is_empty());
    assert_alive(&fx);

    let effects = fx.done(
        hand_backs[0].0,
        OpResult::HandedBack {
            files: files.clone(),
            head: None,
            onto: Some(head_of("t1")),
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(delivers(&effects), vec![conflict_message(&files)]);
    assert_alive(&fx);

    // The worker commits the merge and claims again: straight to the merge queue.
    let merged_head = head_of("t1m");
    let effects = claim(&mut fx, "t1", window, &merged_head);
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    for gate in ["Proof", "Check", "PrepareReview"] {
        assert!(ops_in(&effects, gate).is_empty(), "no {gate}: {effects:#?}");
    }
    assert_eq!(candidate(&fx, "t1").1, merged_head);
    assert_eq!(fx.task("t1").failures, 0);
    assert_alive(&fx);
}

#[test]
fn a_clean_hand_back_requeues_without_the_worker() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (op, _) = candidate(&fx, "t1");
    fx.done(
        op,
        OpResult::Conflict {
            files: vec!["docs/t1/a.md".into()],
        },
    );
    let (hand_back, _) = pending_one(&fx, "HandBack", Some("t1"));
    let merged_head = head_of("t1m");
    let effects = fx.done(
        hand_back,
        OpResult::HandedBack {
            files: vec![],
            head: Some(merged_head.clone()),
            onto: Some(head_of("t1")),
        },
    );
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    assert!(fx.run().outbox.is_empty());
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::MergeQueue);
    assert_eq!(t1.head, Some(merged_head.clone()));
    assert!(!t1.handed_back);
    assert_eq!(candidates_in(&fx, &effects), vec!["t1"]);
    assert_eq!(candidate(&fx, "t1").1, merged_head);
    assert_alive(&fx);
}

#[test]
fn second_conflict_blocks_the_task_as_conflict() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    let window = window_of(&windows, "t1");
    to_queue(&mut fx, "t1", window);
    let (op, _) = candidate(&fx, "t1");
    fx.done(
        op,
        OpResult::Conflict {
            files: vec!["docs/t1/a.md".into()],
        },
    );
    let (hand_back, _) = pending_one(&fx, "HandBack", Some("t1"));
    fx.done(
        hand_back,
        OpResult::HandedBack {
            files: vec![],
            head: Some(head_of("t1m")),
            onto: Some(head_of("t1")),
        },
    );
    let (op, _) = candidate(&fx, "t1");
    let effects = fx.done(
        op,
        OpResult::Conflict {
            files: vec!["docs/t1/a.md".into(), "docs/t1/b.md".into()],
        },
    );
    assert!(ops_in(&effects, "HandBack").is_empty(), "{effects:#?}");
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    let block = t1.block.clone().unwrap();
    assert_eq!(block.reason, BlockReason::Conflict);
    assert!(
        block.text.contains("docs/t1/a.md, docs/t1/b.md"),
        "{block:?}"
    );
    // Conflicts are counted, and they are not failures.
    assert_eq!((t1.conflicts, t1.failures, t1.bounces.merge), (2, 0, 0));
    assert_eq!(t1.rung, 0);
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
    assert!(fx.run().merge_queue.is_empty());
    assert_alive(&fx);
}

#[test]
fn red_candidate_is_a_merge_failure() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    let window = window_of(&windows, "t1");
    to_queue(&mut fx, "t1", window);
    let (op, _) = candidate(&fx, "t1");
    let effects = fx.done(
        op,
        OpResult::CandidateRed {
            code: Some(101),
            timed_out: false,
            tail: "compiling\ntest a::works ... FAILED".into(),
            secs: 42,
        },
    );
    let record = CheckRecord {
        at: fx.now,
        ok: false,
        code: Some(101),
        timed_out: false,
        tail: "compiling\ntest a::works ... FAILED".into(),
        secs: 42,
        on_candidate: true,
    };
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Working);
    assert_eq!((t1.rung, t1.failures, t1.bounces.merge), (1, 1, 1));
    assert_eq!(t1.checks.last(), Some(&record));
    assert_eq!(t1.conflicts, 0);
    let text = candidate_red_message("cargo test", &record);
    assert_eq!(
        text,
        "[anthrex] Your branch merged cleanly into the run branch, but the check failed on the merged result (exit 101): cargo test\nLast 40 lines:\ncompiling\ntest a::works ... FAILED\nFix it on your branch, commit, then call task_done again."
    );
    assert_eq!(delivers(&effects), vec![text]);
    assert!(fx.run().merge_queue.is_empty());
    assert_eq!(fx.run().run_head, BASE);
    assert_alive(&fx);
}

/// Ruling T13-R2 (N3): the merge takes the claimed commit that passed the gates. A
/// worker turn after `task_done` (a self-started one, ruling T12-R4) may commit more;
/// the candidate still names the claimed head, and the engine counts nothing for it.
#[test]
fn a_worker_commit_after_task_done_is_not_merged() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    let window = window_of(&windows, "t1");
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    to_queue(&mut fx, "t1", window);
    // t2's merge runs; t1 waits in the queue while its worker starts a turn of its own.
    fx.signal(window, AgentSignal::TurnStarted);
    fx.signal(
        window,
        AgentSignal::ToolUse {
            name: "Bash".into(),
        },
    );
    let effects = fx.turn_completed(window);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").head, Some(head_of("t1")));
    merge(&mut fx, "t2", &commit(2));
    let (_, task_head) = candidate(&fx, "t1");
    assert_eq!(task_head, head_of("t1"));
    assert_ne!(task_head, fx.task("t1").branch);
    assert_alive(&fx);
}
