//! Milestone 9.2 task M9.2.11, decisions 35, 37, 43 and 44: following the user's
//! merges (anthrex never lands anything itself). A merged stage's base is fetched,
//! which counts its merge commit's parents; the next stage absorbs the new base, is
//! pushed, then retargeted; a merged branch is deleted (when asked) once no PR uses
//! it. A stage closed without merging cancels its fix tasks and pauses the stages
//! above until it is reopened; a paused stage whose tasks are all cancelled counts as
//! skipped. Each landed stage gets one `stage` history line, with the time its PR
//! waited on a person.

use proto::{
    HISTORY_VERSION, HistoryLine, MergeMethod, PlanEdit, PrState, StageLine, StageOutcome,
    TaskOrigin, TaskState, TestMode,
};

use super::bisect::with_orchestrator;
use super::control_restore::restart;
use super::delivery_open::{answer, host_ops, open_stage, pr_on};
use super::delivery_sync::{base_fetch, base_fetch_op, base_sync, fetched};
use super::delivery_watch::{check, fast, poll_with, view, watched};
use super::delivery_watch_adopt::{poll_stage, stage_op, two_stages, view_of};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::full::{attention, delivery_alerts, later, verify_ok};
use super::merge::{commit, doc_task, merge, pending, to_queue, window_of};
use super::propagate::{land_propagates, merged_at};
use super::wake_notes::notes;
use crate::host::{Conclusion, MergeCommit, PrView};
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult};
use crate::run::model::FixOf;

/// A view of stage PR `number` merged on GitHub at merge commit `oid`.
pub(super) fn merged_view(number: u64, head: &str, oid: &str) -> PrView {
    PrView {
        state: PrState::Merged,
        merged_at: Some(5_000),
        merge_commit: Some(MergeCommit { oid: oid.into() }),
        ..view_of(number, head)
    }
}

pub(super) fn closed_view(number: u64, head: &str) -> PrView {
    PrView {
        state: PrState::Closed,
        ..view_of(number, head)
    }
}

/// A stage PR record in `state`, as task M9.2.7 opens one (against `main`).
pub(super) fn pr_record(number: u64, state: PrState) -> crate::run::delivery::PrRecord {
    crate::run::delivery::PrRecord {
        number,
        url: format!("https://github.com/fake/app/pull/{number}"),
        base: "main".into(),
        opened_at: 1_000,
        pushed_head: commit(1),
        state,
        merged_at: None,
        merge_commit: None,
        merge_method: None,
        next_poll_at: u64::MAX / 2,
        unchanged_views: 0,
        last_view_at: None,
        watermark: Default::default(),
        checks: Vec::new(),
        retargeted_to: None,
        opened_base: Some("main".into()),
        branch_deleted: false,
        confirmed: None,
    }
}

/// Stage `n`'s PR is not polled again in this test.
pub(super) fn park(fx: &mut Fixture, n: u16) {
    let pr = fx.run_mut().delivery.stages[usize::from(n) - 1].pr.as_mut();
    pr.unwrap().next_poll_at = u64::MAX / 2;
}

pub(super) fn head(fx: &Fixture, n: u16) -> String {
    fx.run().stage_head(n).unwrap().to_string()
}

pub(super) fn logged(fx: &Fixture, text: &str) -> bool {
    fx.run().log.iter().any(|l| l.text == text)
}

fn branch(n: u16) -> String {
    format!("anthrex/{RUN_ID}/stage-{n}")
}

/// Merges stage 1 of `two_stages(true)` on GitHub as a squash at `commit(70)`, the
/// base then at `commit(71)`; answers the base fetch and stage 2's base sync, landed at
/// `commit(72)`.
pub(super) fn stage_1_squashed(fx: &mut Fixture) {
    let h1 = head(fx, 1);
    poll_stage(fx, 1, merged_view(11, &h1, &commit(70)));
    fetched(fx, &commit(71), Some(1));
    let (op, _) = base_sync(fx);
    fx.done(op, merged_at(&commit(72)));
}

#[test]
fn merged_stage_syncs_the_next_pushes_then_retargets() {
    let (mut fx, _) = two_stages(true);
    with_orchestrator(&mut fx);
    park(&mut fx, 2);
    let h1 = head(&fx, 1);
    let mut order = Vec::new();
    poll_stage(&mut fx, 1, merged_view(11, &h1, &commit(70)));
    assert!(logged(
        &fx,
        "stage 1 (PR #11): merged on the host at 70eeeee"
    ));
    // 1. The base is fetched, counting the merge commit's parents (ruling R-4).
    let (_, op) = base_fetch(&fx);
    assert_eq!(op, base_fetch_op(Some(commit(70))));
    order.push("fetch");
    assert!(
        !(host_ops(&fx).iter()).any(|(_, o)| matches!(o, HostOp::Retarget { .. })),
        "nothing is retargeted before the sync"
    );
    fetched(&mut fx, &commit(71), Some(1));
    let pr = fx.run().delivery.pr(1).unwrap();
    assert_eq!(pr.merge_method, Some(MergeMethod::SquashOrRebase));
    assert_eq!(
        notes(&fx),
        ["stage 1 PR #11 was merged (squash or rebase); stage 2 is being synced and retargeted"]
    );
    // 2. The next stage absorbs the new base before it is retargeted.
    let (op, spec) = base_sync(&fx);
    assert_eq!((spec.from, spec.to), (0, 2));
    assert_eq!(spec.from_head, commit(71));
    order.push("base sync");
    fx.done(op, merged_at(&commit(72)));
    // 3. It is pushed.
    let (op, push) = stage_op(&fx, 2);
    assert_eq!(
        push,
        HostOp::Push {
            stage: 2,
            sha: commit(72)
        }
    );
    order.push("push");
    answer(
        &mut fx,
        op,
        HostResult::Pushed(crate::host::PushOutcome::Pushed),
    );
    // 4. Then retargeted onto the base branch (`gh pr edit --base`).
    let (op, retarget) = stage_op(&fx, 2);
    assert_eq!(
        retarget,
        HostOp::Retarget {
            stage: 2,
            number: 12,
            base: "main".into()
        }
    );
    order.push("retarget");
    answer(&mut fx, op, HostResult::Retargeted);
    assert_eq!(order, ["fetch", "base sync", "push", "retarget"]);
    let pr = fx.run().delivery.pr(2).unwrap();
    assert_eq!(pr.retargeted_to.as_deref(), Some("main"));
    assert!(logged(&fx, "stage 2 (PR #12): retargeted onto main"));
    // Once: no second retarget, and nothing ever merges.
    later(&mut fx, 30);
    let all: Vec<HostOp> = super::delivery_open::host_ops_in(&fx.log);
    let retargets = all.iter().filter(|o| matches!(o, HostOp::Retarget { .. }));
    assert_eq!(retargets.count(), 1);
    assert_eq!(fx.run().state, proto::RunState::Running);
}

/// Decision 35, the squash case, engine side: the base sync after a squash merges a
/// base that holds stage 1's changes under another commit; 9.1's executor finds it
/// clean and changing no file (real git: `driver/stage_ops_propagate_tests.rs`), and
/// it lands like any base sync: stage 2 holds the squash, and nothing is red or held.
#[test]
fn squash_merged_stage_sync_is_clean_and_changes_no_file() {
    let (mut fx, _) = two_stages(true);
    park(&mut fx, 2);
    stage_1_squashed(&mut fx);
    assert_eq!(head(&fx, 2), commit(72));
    assert_eq!(
        fx.run().delivery.base_synced.as_deref(),
        Some(commit(71).as_str())
    );
    let stage = fx.run().delivery.stage(2).unwrap();
    assert_eq!(stage.sync_red, None);
    assert!(!fx.run().tasks.iter().any(|t| t.origin == TaskOrigin::Sync));
    assert!(attention(&fx).is_empty(), "{:?}", attention(&fx));
}

#[test]
fn delete_merged_branches_deletes_only_a_branch_no_pr_uses() {
    for delete in [true, false] {
        let (mut fx, _) = two_stages(true);
        fx.run_mut().delivery.limits.delete_merged_branches = delete;
        park(&mut fx, 2);
        stage_1_squashed(&mut fx);
        let (op, _) = stage_op(&fx, 2);
        answer(
            &mut fx,
            op,
            HostResult::Pushed(crate::host::PushOutcome::Pushed),
        );
        // Stage 2's PR is still based on stage 1's branch: it is not deleted.
        let deletes = |fx: &Fixture| {
            (host_ops(fx).into_iter())
                .filter(|(_, o)| matches!(o, HostOp::DeleteBranch { .. }))
                .collect::<Vec<_>>()
        };
        assert!(deletes(&fx).is_empty(), "#12 still uses {}", branch(1));
        let (op, _) = stage_op(&fx, 2);
        answer(&mut fx, op, HostResult::Retargeted);
        let found = deletes(&fx);
        if !delete {
            assert!(found.is_empty(), "delete_merged_branches = false");
            later(&mut fx, 30);
            assert!(deletes(&fx).is_empty());
            continue;
        }
        assert_eq!(found.len(), 1, "{:#?}", host_ops(&fx));
        assert_eq!(found[0].1, HostOp::DeleteBranch { stage: 1 });
        answer(&mut fx, found[0].0, HostResult::Deleted);
        assert!(fx.run().delivery.pr(1).unwrap().branch_deleted);
        assert!(logged(
            &fx,
            &format!("stage 1: deleted {} on the remote", branch(1))
        ));
        later(&mut fx, 30);
        assert!(deletes(&fx).is_empty(), "once");
        // Stage 2's own branch backs its open PR: never deleted.
        let all = super::delivery_open::host_ops_in(&fx.log);
        assert!(!all.contains(&HostOp::DeleteBranch { stage: 2 }));
    }
}

/// A `pr` run with one writer: `t1` in stage 1, merged, with PR #11 open (polled every
/// second); `t2` (working) and `t3` (queued for the writer) in stage 2.
fn closing() -> (Fixture, Vec<(String, u32)>) {
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", "stage = 2"),
        doc_task("t3", "stage = 2"),
    ];
    let (mut fx, mut windows) = pr_on(&profile_with("max_writers = 1"), &tasks);
    fast(fx.run_mut());
    with_orchestrator(&mut fx);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    land_propagates(&mut fx);
    super::delivery_open::green(&mut fx, 1);
    open_stage(&mut fx, 1, 11);
    // t2 is dispatched once the writer is free; its prepare then its window.
    for _ in 0..2 {
        windows.extend(fx.launch_all());
    }
    assert_eq!(fx.task("t2").state, TaskState::Working);
    assert_eq!(fx.task("t3").state, TaskState::Queued, "no writer free");
    (fx, windows)
}

/// A `ci` fix task of stage 1, as task M9.2.9 adds one.
fn ci_fix(fx: &mut Fixture) -> String {
    let spec = super::super::fixes::FixSpec {
        origin: TaskOrigin::Ci,
        fixes: FixOf::Ci {
            stage: 1,
            head: commit(1),
            ci_runs: vec![5],
            key: "a::b".into(),
        },
        stage: 1,
        title: "Fix CI on stage 1: a::b".into(),
        brief: "fix it".into(),
        acceptance: vec!["green".into()],
        owns: vec!["docs/t1/**".into()],
        size: proto::Size::S,
        epic: None,
        route: proto::RouteSpec::default(),
        test_mode: TestMode::Check,
        test_mode_reason: Some("fix task: the failing checks are the proof".into()),
        sync: None,
    };
    let now = fx.now;
    let id = super::super::fixes::add_fix(fx.run_mut(), spec, now, &mut Vec::new()).unwrap();
    fx.send(now, EventKind::Tick);
    id
}

const CLOSED: &str = "stage 1 PR closed without merging; resume, re-plan, or cancel the rest";

#[test]
fn closed_stage_pauses_the_stages_above_and_cancels_its_fixes() {
    let (mut fx, windows) = closing();
    let fix = ci_fix(&mut fx);
    assert_eq!(fx.task("t2").state, TaskState::Working, "t2 runs");
    assert!(!fx.task("t3").state.is_finished());
    super::wake_notes::clear(&mut fx);
    let at = head(&fx, 1);
    poll_stage(&mut fx, 1, closed_view(11, &at));
    // Its unfinished fix tasks are cancelled.
    let task = fx.task(&fix);
    assert_eq!(task.state, TaskState::Cancelled);
    let why = "cancelled: stage 1 PR closed";
    assert!(
        task.history.iter().any(|e| e.text == why),
        "{:#?}",
        task.history
    );
    // Every stage above pauses; TT's exact attention line and wake note.
    assert_eq!(fx.run().delivery.stage(2).unwrap().paused_by, Some(1));
    assert!(
        attention(&fx).contains(&CLOSED.to_string()),
        "{:?}",
        attention(&fx)
    );
    assert_eq!(notes(&fx), [CLOSED]);
    let closed = proto::DeliveryAlertKind::PrClosedUnmerged;
    assert!(delivery_alerts(&fx).contains(&(closed, Some(1), CLOSED.to_string())));
    assert!(logged(
        &fx,
        "stage 2: paused (stage 1 PR closed without merging)"
    ));
    // A task already running finishes normally; no pending task of the stage starts,
    // even with a writer free; no PR opens and nothing is pushed for it.
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    land_propagates(&mut fx);
    assert_eq!(fx.task("t2").state, TaskState::Merged);
    later(&mut fx, 5);
    assert_eq!(fx.task("t3").state, TaskState::Pending, "paused");
    let all = super::delivery_open::host_ops_in(&fx.log);
    assert!(
        !all.iter()
            .any(|o| super::super::delivery::stage_of(o) == Some(2)),
        "nothing for stage 2: {all:#?}"
    );
    // Processed once: the same view again adds nothing.
    let notes_before = notes(&fx).len();
    poll_stage(&mut fx, 1, closed_view(11, &at));
    assert_eq!(notes(&fx).len(), notes_before);
}

#[test]
fn reopened_stage_unpauses_the_stages_above() {
    let (mut fx, windows) = closing();
    let at = head(&fx, 1);
    poll_stage(&mut fx, 1, closed_view(11, &at));
    assert_eq!(fx.run().delivery.stage(2).unwrap().paused_by, Some(1));
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    land_propagates(&mut fx);
    fx.tick();
    assert_eq!(fx.task("t3").state, TaskState::Pending);
    // The user reopens it on GitHub; the next poll (closed PRs are polled at the cap)
    // sees it.
    poll_stage(&mut fx, 1, view_of(11, &at));
    assert_eq!(fx.run().delivery.stage(2).unwrap().paused_by, None);
    assert!(logged(
        &fx,
        "stage 1 (PR #11): reopened; the stages above resume"
    ));
    assert!(logged(&fx, "stage 2: no longer paused"));
    assert!(!attention(&fx).contains(&CLOSED.to_string()));
    fx.tick();
    assert_ne!(fx.task("t3").state, TaskState::Pending, "it starts");
}

/// The controller's re-plan ruling: the user (or the orchestrator) cancels a paused
/// stage's tasks, so it counts as skipped and the run completes; a task added to a
/// skipped stage reopens it (it opens its own PR, never rides in the next one).
#[test]
fn a_paused_stage_whose_tasks_are_cancelled_counts_as_skipped() {
    for add in [false, true] {
        let (mut fx, windows) = closing();
        let at = head(&fx, 1);
        poll_stage(&mut fx, 1, closed_view(11, &at));
        let cancel = ["t2", "t3"].map(|id| PlanEdit::CancelTask { task_id: id.into() });
        let effects = edit(&mut fx, cancel.to_vec());
        assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
        // t2's session ends.
        super::turns::exited(&mut fx, window_of(&windows, "t2"));
        assert!(fx.run().delivery.stage(2).unwrap().skipped);
        assert!(logged(
            &fx,
            "stage 2: skipped (its tasks were cancelled while it was paused)"
        ));
        if !add {
            // Its worktrees go; nothing else waits: the run completes.
            for (op, _) in pending(&fx, "RemoveWorktree", Some("t2")) {
                fx.done(
                    op,
                    OpResult::Removed {
                        salvage_ref: None,
                        cleared_locks: Vec::new(),
                    },
                );
            }
            verify_ok(&mut fx);
            assert_eq!(fx.run().state, proto::RunState::Complete);
            continue;
        }
        let text = plan_with(PROFILE, &[doc_task("t4", "stage = 2")]);
        let task = crate::run::plan::parse_plan(&text).unwrap().tasks.remove(0);
        let effects = edit(&mut fx, vec![PlanEdit::AddTask { task }]);
        assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
        assert!(!fx.run().delivery.stage(2).unwrap().skipped);
        assert!(logged(
            &fx,
            "stage 2: a task was added, so it is no longer skipped"
        ));
        assert!(pending(&fx, "VerifyRefs", None).is_empty());
    }
}

/// Task M9.2.8's carried ruling: landing a held stage never counts its unpushed local
/// head as landed. Below a delivering stage its commits go up with it; at the top they
/// are an attention line and the run does not complete on them.
#[test]
fn a_held_stage_lands_only_what_it_pushed() {
    // Stage 1 of two: held, with a fix merged locally and never pushed.
    let (mut fx, _) = two_stages(true);
    park(&mut fx, 2);
    let pushed = head(&fx, 1);
    fx.run_mut().delivery.stages[0].held = Some("protected branch".into());
    set_stage_head(fx.run_mut(), 1, &commit(80));
    land_propagates(&mut fx);
    poll_stage(&mut fx, 1, merged_view(11, &pushed, &commit(70)));
    let line = format!(
        "stage 1 (PR #11): merged at {}, without 80eeeee, so its commits go up with stage 2",
        &pushed[..7]
    );
    assert!(logged(&fx, &line), "{:#?}", fx.run().log);
    assert_eq!(fx.run().delivery.stage(1).unwrap().held, None);
    assert_eq!(fx.run().delivery.pr(1).unwrap().pushed_head, pushed);
    let all = super::delivery_open::host_ops_in(&fx.log);
    assert!(!all.contains(&HostOp::Push {
        stage: 1,
        sha: commit(80)
    }));

    // The top stage of a `Single` run: the work is not delivered, and said so.
    let mut fx = watched();
    let pushed = head(&fx, 1);
    fx.run_mut().delivery.stages[0].held = Some("protected branch".into());
    set_stage_head(fx.run_mut(), 1, &commit(80));
    poll_with(
        &mut fx,
        merged_view(super::delivery_watch::PR, &pushed, &commit(70)),
    );
    fetched(&mut fx, &commit(71), Some(2));
    let line = format!(
        "stage 1 PR #7 was merged at {}, without 80eeeee; that work is not delivered (anthrex run cancel gives up)",
        &pushed[..7]
    );
    assert!(attention(&fx).contains(&line), "{:?}", attention(&fx));
    later(&mut fx, 5);
    assert!(pending(&fx, "VerifyRefs", None).is_empty(), "not complete");
}

/// The `stage` history lines among `effects`.
pub(super) fn stage_lines(effects: &[Effect]) -> Vec<StageLine> {
    ops_in(effects, "AppendHistory")
        .into_iter()
        .filter_map(|(_, kind)| match kind {
            OpKind::AppendHistory { line, .. } => match *line {
                HistoryLine::Stage(l) => Some(l),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn stage_history_line_is_appended_once_with_its_fields() {
    let mut fx = watched();
    fx.run_mut().repo_dir = "/tmp/repo".into();
    let (ready_at, opened_at) = {
        let s = fx.run().delivery.stage(1).unwrap();
        (s.ready_at.unwrap(), s.pr.as_ref().unwrap().opened_at)
    };
    later(&mut fx, 40);
    let at = head(&fx, 1);
    let (merged_at_time, _) = poll_with(
        &mut fx,
        merged_view(super::delivery_watch::PR, &at, &commit(70)),
    );
    assert!(
        stage_lines(&fx.log).is_empty(),
        "the method is not known yet"
    );
    let effects = fetched(&mut fx, &commit(71), Some(2));
    let lines = stage_lines(&effects);
    let wait = fx.run().delivery.stage(1).unwrap().review_wait_secs;
    assert_eq!(
        wait,
        merged_at_time - opened_at,
        "open and waiting until merged"
    );
    assert_eq!(
        lines,
        [StageLine {
            v: HISTORY_VERSION,
            record_id: format!("{RUN_ID}/stage/1"),
            run_id: RUN_ID.into(),
            stage: 1,
            pr: 7,
            time_to_open_secs: opened_at - ready_at,
            human_review_secs: wait,
            ci_rounds: 0,
            review_rounds: 0,
            sync_tasks: 0,
            outcome: StageOutcome::Merged,
            merge_method: MergeMethod::Merge,
            at: fx.now,
        }]
    );
    let (op, _) = (pending(&fx, "AppendHistory", None).into_iter())
        .find(|(_, k)| matches!(k, OpKind::AppendHistory { record_id, .. } if record_id.ends_with("/stage/1")))
        .unwrap();
    fx.done(op, OpResult::HistoryAppended);
    // Once: more passes, another view and a restart add none.
    later(&mut fx, 30);
    poll_with(
        &mut fx,
        merged_view(super::delivery_watch::PR, &at, &commit(70)),
    );
    restart(&mut fx, Vec::new());
    super::control::resume(&mut fx);
    later(&mut fx, 30);
    assert_eq!(stage_lines(&fx.log).len(), 1);
}

/// Decision 44: a `pr`-mode run cancelled with its PR open records it, `open_at_cancel`.
#[test]
fn a_cancelled_run_records_its_open_pr() {
    let mut fx = watched();
    fx.run_mut().repo_dir = "/tmp/repo".into();
    let reply = fx.reply();
    let effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    let lines = stage_lines(&effects);
    assert_eq!(lines.len(), 1, "{effects:#?}");
    assert_eq!(lines[0].outcome, StageOutcome::OpenAtCancel);
    assert_eq!(lines[0].merge_method, MergeMethod::None);
}

#[test]
fn human_review_secs_counts_only_the_waiting_time() {
    let mut fx = watched();
    let secs = |fx: &Fixture| fx.run().delivery.stage(1).unwrap().review_wait_secs;
    let shown = |fx: &Fixture| {
        crate::run::delivery::snapshot::stage_pr_info(fx.run(), 1)
            .unwrap()
            .human_review_secs
    };
    let opened = fx.run().delivery.pr(1).unwrap().opened_at;
    let at = head(&fx, 1);
    // Open, nothing running: a person is being waited on.
    fx.send(opened + 10, EventKind::Tick);
    assert_eq!(secs(&fx), 10);
    // CI starts on the head: from that view on, the stack waits on CI, not a person.
    let running = PrView {
        checks: vec![check("test", None, 5)],
        ..view(&at)
    };
    let (seen, _) = poll_with(&mut fx, running);
    let first = seen - opened;
    assert_eq!(secs(&fx), first);
    later(&mut fx, 30);
    assert_eq!(secs(&fx), first, "CI running");
    // Green: waiting on a person again.
    let green = PrView {
        checks: vec![check("test", Some(Conclusion::Success), 5)],
        ..view(&at)
    };
    poll_with(&mut fx, green);
    later(&mut fx, 5);
    assert_eq!(secs(&fx), first + 5);
    // A fix task of the stage is in flight: anthrex's time, not the person's.
    ci_fix(&mut fx);
    later(&mut fx, 20);
    assert_eq!(secs(&fx), first + 5);
    assert_eq!(shown(&fx), first + 5, "the snapshot carries it");
}
