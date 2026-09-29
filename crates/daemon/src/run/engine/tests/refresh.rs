//! Milestone 9 task M9.13a: the `refresh` edit (decision 42e) in the reducer: due at
//! the worker's turn boundary, M8a's `HandBack` with `list_merged`, the mail it holds,
//! its four results, and why a refresh merge is never the task's work. The git side is
//! `crates/daemon/tests/run_refresh.rs`.

use proto::{MessageKind, PlanEdit, TaskState};

use super::control_restore::restart;
use super::dispatch::{edit, replies, task_path};
use super::done::{done_args, one_reply};
use super::fixture::*;
use super::gates::only_op;
use super::holds::delivers;
use super::turns::working;
use super::worker_messages::{from_user, msg};
use super::worker_messages_pause::{orchestrated, wake_notes};
use crate::run::contract::{DONE_NUDGE, NO_COMMIT_NUDGE};
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::model::OpId;
use crate::run::orch::RefreshState;
use crate::run::orch::contract::{refresh_clean, refresh_conflict};

/// The merge commit a clean refresh made.
const MERGE: &str = "e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1";

fn refresh(task: &str) -> PlanEdit {
    PlanEdit::Refresh {
        task_id: task.into(),
    }
}

/// The worker's turn ends; the fallback's count (none yet) answers first, then the
/// refresh's hand-back goes out: its op and kind.
fn at_boundary(fx: &mut Fixture, window: u32) -> (OpId, OpKind) {
    let effects = fx.turn_completed(window);
    assert!(
        ops_in(&effects, "HandBack").is_empty(),
        "the count comes first"
    );
    let (op, _) = only_op(&effects, "CountCommits");
    let commits = OpResult::Commits {
        count: 0,
        head: HEAD.into(),
    };
    let effects = fx.done(op, commits);
    assert!(delivers(&effects).is_empty(), "the refresh holds the nudge");
    only_op(&effects, "HandBack")
}

fn clean(merged: &[&str]) -> OpResult {
    OpResult::HandedBack {
        files: Vec::new(),
        head: Some(MERGE.into()),
        onto: Some(HEAD.into()),
        merged: merged.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn refresh_waits_for_the_turn_boundary() {
    let (mut fx, window) = working();
    let effects = edit(&mut fx, vec![refresh("t1")]);
    assert_eq!(replies(&effects), vec![Ok("applied 1 edit".to_string())]);
    assert_eq!(fx.task("t1").orch.refresh, Some(RefreshState::Due));
    assert!(ops_in(&effects, "HandBack").is_empty(), "the turn is open");
    assert!(ops_in(&fx.tick(), "HandBack").is_empty());
    let (op, kind) = at_boundary(&mut fx, window);
    assert_eq!(
        kind,
        OpKind::HandBack {
            worktree: task_path("t1"),
            run_head: BASE.into(),
            task_head: None,
            list_merged: true,
        }
    );
    assert_eq!(fx.task("t1").orch.refresh, Some(RefreshState::InFlight(op)));
    assert!(ops_in(&fx.tick(), "HandBack").is_empty(), "one hand-back");
}

#[test]
fn refresh_holds_a_change_message_sent_in_the_next_call_for_one_turn() {
    let (mut fx, window) = working();
    edit(&mut fx, vec![refresh("t1")]);
    let (op, _) = at_boundary(&mut fx, window);
    let effects = edit(&mut fx, vec![msg(&["t1"], MessageKind::Change, "use v2")]);
    assert!(
        delivers(&effects).is_empty(),
        "held while the refresh is in flight"
    );
    let effects = fx.done(op, clean(&["abc1234 feat: v2 client"]));
    let sent = delivers(&effects);
    assert_eq!(sent.len(), 1, "one turn: {sent:?}");
    let list = [("abc1234".to_string(), "feat: v2 client".to_string())];
    assert!(sent[0].contains(&refresh_clean(1, &list)), "{}", sent[0]);
    assert!(
        sent[0].contains(&from_user(MessageKind::Change, "use v2")),
        "{}",
        sent[0]
    );
    assert!(sent[0].contains(NO_COMMIT_NUDGE), "{}", sent[0]);
    let task = fx.task("t1");
    assert_eq!(task.orch.refresh, None);
    assert_eq!(task.orch.refresh_merges, vec![MERGE.to_string()]);
}

#[test]
fn refresh_up_to_date_sends_nothing() {
    let (mut fx, window) = working();
    edit(&mut fx, vec![refresh("t1")]);
    let (op, _) = at_boundary(&mut fx, window);
    let up_to_date = OpResult::HandedBack {
        files: Vec::new(),
        head: Some(HEAD.into()),
        onto: Some(HEAD.into()),
        merged: Vec::new(),
    };
    let effects = fx.done(op, up_to_date);
    assert_eq!(delivers(&effects), vec![NO_COMMIT_NUDGE.to_string()]);
    let task = fx.task("t1");
    assert_eq!(task.history.last().unwrap().text, "refresh: nothing new");
    assert!(task.orch.refresh_merges.is_empty());
}

#[test]
fn refresh_failure_is_a_wake_note_and_no_block() {
    let (mut fx, window) = orchestrated();
    edit(&mut fx, vec![refresh("t1")]);
    let (op, _) = at_boundary(&mut fx, window);
    let failed = OpResult::Failed {
        message: "uncommitted changes".into(),
    };
    fx.done(op, failed);
    let task = fx.task("t1");
    assert_eq!(task.state, TaskState::Working);
    assert_eq!(task.block, None);
    assert_eq!(task.orch.refresh, None);
    assert_eq!(
        task.history.last().unwrap().text,
        "refresh skipped: uncommitted changes"
    );
    let notes = wake_notes(&fx);
    assert!(
        notes.contains(&"refresh of t1 skipped: uncommitted changes".to_string()),
        "{notes:?}"
    );
    let entry = fx.run().plan_edits.last().unwrap();
    assert_eq!(entry.text, "refresh t1");
    assert!(entry.accepted);
    assert_eq!(entry.error.as_deref(), Some("uncommitted changes"));
}

#[test]
fn refresh_is_refused_on_a_resolving_or_awaiting_task() {
    let plan = plan_with(
        PROFILE,
        &[
            task("t1", "S", "a", ""),
            task("t2", "S", "b", "deps = [\"t1\"]"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    fx.launch_all();
    let refused = |fx: &mut Fixture, task: &str, label: &str| {
        assert_eq!(
            replies(&edit(fx, vec![refresh(task)])),
            vec![Err(format!(
                "task {task} is {label}; refresh needs a working or paused task"
            ))]
        );
    };
    fx.task_mut("t1").resolving = true;
    refused(&mut fx, "t1", "working");
    fx.task_mut("t1").resolving = false;
    refused(&mut fx, "t2", "pending");
    assert!(fx.task("t2").orch.refresh.is_none());
    // A second refresh while one is due.
    assert!(replies(&edit(&mut fx, vec![refresh("t1")]))[0].is_ok());
    refused(&mut fx, "t1", "working");
    // A task held on its dependencies (M8a ruling N5).
    let (mut fx, _) = working();
    fx.task_mut("t1").awaiting_deps = true;
    refused(&mut fx, "t1", "working");
}

#[test]
fn a_refresh_merge_alone_is_not_work() {
    // The fallback: one commit, the refresh's merge, is no commit.
    let (mut fx, window) = working();
    fx.task_mut("t1").orch.refresh_merges.push(MERGE.into());
    let effects = fx.turn_completed(window);
    let (op, _) = only_op(&effects, "CountCommits");
    let one = OpResult::Commits {
        count: 1,
        head: MERGE.into(),
    };
    let effects = fx.done(op, one.clone());
    assert_eq!(delivers(&effects), vec![NO_COMMIT_NUDGE.to_string()]);
    // The control: with no refresh merge, that commit is the task's.
    let (mut fx, window) = working();
    let effects = fx.turn_completed(window);
    let (op, _) = only_op(&effects, "CountCommits");
    assert_eq!(delivers(&fx.done(op, one)), vec![DONE_NUDGE.to_string()]);

    // `task_done`: the zero-commit check ignores the refresh's merge.
    let (mut fx, window) = working();
    fx.task_mut("t1").orch.refresh_merges.push(MERGE.into());
    let effects = fx.tool(window, "task_done", done_args());
    let (op, _) = only_op(&effects, "VerifyDone");
    let mut result = fx.clean_check("t1");
    if let OpResult::DoneChecked { commits, .. } = &mut result {
        *commits = 1;
    }
    let effects = fx.done(op, result);
    assert_eq!(
        one_reply(&effects),
        Err(
            "task_done rejected: the branch has no commit since the task started; \
             commit your work first"
                .to_string()
        )
    );
}

#[test]
fn refresh_clean_is_called_with_the_whole_list() {
    let (mut fx, window) = working();
    edit(&mut fx, vec![refresh("t1")]);
    let (op, _) = at_boundary(&mut fx, window);
    let merged: Vec<String> = (0..12).map(|n| format!("c{n:06} commit {n}")).collect();
    let refs: Vec<&str> = merged.iter().map(String::as_str).collect();
    let effects = fx.done(op, clean(&refs));
    let list: Vec<(String, String)> = (0..12)
        .map(|n| (format!("c{n:06}"), format!("commit {n}")))
        .collect();
    let text = refresh_clean(12, &list);
    assert!(
        text.contains("(12 commits: ")
            && text.ends_with("and 2 more). Rebuild before you continue.")
    );
    let sent = delivers(&effects);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains(&text), "{}", sent[0]);
}

#[test]
fn refresh_conflict_sets_resolving_and_leaves_conflicts_alone() {
    let (mut fx, window) = working();
    edit(&mut fx, vec![refresh("t1")]);
    let (op, _) = at_boundary(&mut fx, window);
    let files = vec!["crates/a/x.rs".to_string()];
    let conflict = OpResult::HandedBack {
        files: files.clone(),
        head: Some(HEAD.into()),
        onto: Some(HEAD.into()),
        merged: Vec::new(),
    };
    let effects = fx.done(op, conflict);
    let task = fx.task("t1");
    assert!(task.resolving);
    assert_eq!(task.conflicts, 0, "only the merge queue counts a conflict");
    assert!(task.orch.refresh_merges.is_empty());
    let sent = delivers(&effects);
    assert!(sent[0].contains(&refresh_conflict(&files)), "{sent:?}");
}

#[test]
fn a_refresh_lost_in_a_restart_is_due_again() {
    let (mut fx, window) = working();
    edit(&mut fx, vec![refresh("t1")]);
    at_boundary(&mut fx, window);
    let effects = restart(&mut fx, Vec::new());
    assert!(effects.iter().all(|e| !matches!(e, Effect::Deliver { .. })));
    assert_eq!(fx.task("t1").orch.refresh, Some(RefreshState::Due));
}
