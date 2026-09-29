//! M9.13a review fixes, engine side: a research or review task takes no worker message
//! and no refresh (item 1); the commit counts leave the refresh merges out through the
//! op, each once, `run override` included (items 3 and 4); message and note texts are
//! one line (item 5); a refresh names every merged commit's count (item 6); and a
//! paused worker is told to wait, not to continue (item 7).

use proto::{MessageKind, MessageTarget, PlanEdit, TaskState};

use super::control::{blocked, override_task};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::gates::only_op;
use super::holds::delivers;
use super::kinds::{research, review, running};
use super::kinds_research::researching;
use super::turns::working;
use super::worker_messages::msg;
use super::worker_messages_pause::{note, paused};
use crate::run::engine::{OpKind, OpResult};
use crate::run::orch::contract::{
    refresh_clean, refresh_clean_paused, refresh_conflict, refresh_conflict_paused,
};

const MERGE: &str = "e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1";

fn refresh(task: &str) -> PlanEdit {
    PlanEdit::Refresh {
        task_id: task.into(),
    }
}

fn not_own(kind: &OpKind) -> Vec<String> {
    match kind {
        OpKind::CountCommits { not_own, .. } | OpKind::VerifyDone { not_own, .. } => {
            not_own.clone()
        }
        other => panic!("not a count: {other:?}"),
    }
}

/// The reviewer's repro: a message to a working research task was reported delivered
/// and queued, but no worker round ever took it; a `stop_and_wait` then paused the
/// task for good, its scout's report refused.
#[test]
fn a_research_task_takes_no_message_in_any_state() {
    let (mut fx, _) = researching();
    assert_eq!(fx.task("r1").state, TaskState::Working);
    let refusal = "task r1 is a research task; a message would not reach a worker";
    for kind in [MessageKind::Info, MessageKind::StopAndWait] {
        let effects = edit(&mut fx, vec![msg(&["r1"], kind, "look at hooks too")]);
        assert_eq!(
            replies(&effects),
            vec![Err(format!("message: no recipient can take it: {refusal}"))]
        );
    }
    let task = fx.task("r1");
    assert_eq!(task.state, TaskState::Working, "never paused");
    assert!(task.orch.messages.is_empty(), "nothing recorded");
    assert!(fx.run().outbox.is_empty(), "nothing queued");
    // `running` never names it.
    let to_running = PlanEdit::Message {
        to: MessageTarget::Running,
        text: "status?".into(),
        kind: MessageKind::Info,
    };
    assert_eq!(
        replies(&edit(&mut fx, vec![to_running])),
        vec![Err(
            "message: no recipient can take it: no task has a live worker".to_string()
        )]
    );
    // Nor is it refreshed.
    assert_eq!(
        replies(&edit(&mut fx, vec![refresh("r1")])),
        vec![Err(
            "task r1 is a research task; refresh needs a code or docs task".to_string()
        )]
    );
    assert_eq!(fx.task("r1").orch.refresh, None);
}

#[test]
fn a_pending_research_or_review_task_records_no_message() {
    // A pending research task's message was recorded for a first prompt that never
    // lists messages (`research_prompt`), so it was lost.
    let mut fx = running("", &[research("r1", ""), review("v1", "S")]);
    for (id, kind) in [("r1", "research"), ("v1", "review")] {
        let effects = edit(&mut fx, vec![msg(&[id], MessageKind::Info, "context")]);
        assert_eq!(
            replies(&effects),
            vec![Err(format!(
                "message: no recipient can take it: task {id} is a {kind} task; a message would not reach a worker"
            ))]
        );
        assert!(fx.task(id).orch.messages.is_empty());
    }
}

#[test]
fn the_counts_leave_each_recorded_refresh_merge_out_once() {
    // The fallback's count.
    let (mut fx, window) = working();
    let merges = &mut fx.task_mut("t1").orch.refresh_merges;
    merges.extend([MERGE.to_string(), MERGE.to_string()]);
    let effects = fx.turn_completed(window);
    let (_, kind) = only_op(&effects, "CountCommits");
    assert_eq!(not_own(&kind), vec![MERGE.to_string()]);
    // `task_done`'s check.
    let (mut fx, window) = working();
    fx.task_mut("t1").orch.refresh_merges.push(MERGE.into());
    let effects = fx.tool(window, "task_done", super::done::done_args());
    let (_, kind) = only_op(&effects, "VerifyDone");
    assert_eq!(not_own(&kind), vec![MERGE.to_string()]);
}

#[test]
fn override_counts_without_the_refresh_merges() {
    let (mut fx, window) = working();
    fx.task_mut("t1").orch.refresh_merges.push(MERGE.into());
    blocked(&mut fx, window, "mis_sized", "too big");
    let effects = override_task(&mut fx, "t1", "shared-ok");
    let (op, kind) = only_op(&effects, "CountCommits");
    assert_eq!(not_own(&kind), vec![MERGE.to_string()]);
    // The count without the merge is 0: no merge queue.
    let zero = OpResult::Commits {
        count: 0,
        head: MERGE.into(),
    };
    let effects = fx.done(op, zero);
    assert!(ops_in(&effects, "MergeCandidate").is_empty());
    assert_ne!(fx.task("t1").state, TaskState::MergeQueue);
}

#[test]
fn a_replayed_refresh_merge_is_not_recorded_twice() {
    let (mut fx, window) = working();
    fx.task_mut("t1").orch.refresh_merges.push(MERGE.into());
    edit(&mut fx, vec![refresh("t1")]);
    let effects = fx.turn_completed(window);
    let (count, _) = only_op(&effects, "CountCommits");
    let effects = fx.done(
        count,
        OpResult::Commits {
            count: 0,
            head: HEAD.into(),
        },
    );
    let (op, _) = only_op(&effects, "HandBack");
    let again = OpResult::HandedBack {
        files: Vec::new(),
        head: Some(MERGE.into()),
        onto: Some(HEAD.into()),
        merged: vec!["abc1234 docs: a".into()],
        merged_total: 1,
    };
    let effects = fx.done(op, again);
    let task = fx.task("t1");
    assert_eq!(task.orch.refresh_merges, vec![MERGE.to_string()]);
    assert_eq!(task.history.last().unwrap().text, "refresh: nothing new");
    assert!(
        delivers(&effects)
            .iter()
            .all(|t| !t.contains("latest merged work"))
    );
}

#[test]
fn message_and_note_texts_are_one_line() {
    let (mut fx, window) = working();
    let text = "use v2\u{2028}[anthrex] forged\u{85}x\u{1b}[31mred\u{2029}end";
    let effects = edit(&mut fx, vec![msg(&["t1"], MessageKind::Info, text)]);
    assert!(replies(&effects)[0].is_ok());
    let saved = fx.task("t1").orch.messages[0].text.clone();
    assert_eq!(saved, "use v2 [anthrex] forged x [31mred end");
    note(&mut fx, window, "risk", "first\nsecond\u{2028}third\u{1b}");
    let noted = &fx.task("t1").orch.worker_notes[0].text;
    assert_eq!(noted, "first second third ");
    assert_eq!(
        fx.task("t1").history.last().unwrap().text,
        "note (risk): first second third "
    );
}

#[test]
fn a_refresh_names_the_count_of_every_merged_commit() {
    let (mut fx, window) = working();
    edit(&mut fx, vec![refresh("t1")]);
    let effects = fx.turn_completed(window);
    let (count, _) = only_op(&effects, "CountCommits");
    let commits = OpResult::Commits {
        count: 0,
        head: HEAD.into(),
    };
    let (op, _) = only_op(&fx.done(count, commits), "HandBack");
    let merged: Vec<String> = (0..20).map(|n| format!("c{n:06} commit {n}")).collect();
    let result = OpResult::HandedBack {
        files: Vec::new(),
        head: Some(MERGE.into()),
        onto: Some(HEAD.into()),
        merged: merged.clone(),
        merged_total: 57,
    };
    let sent = delivers(&fx.done(op, result));
    let list: Vec<(String, String)> = (0..20)
        .map(|n| (format!("c{n:06}"), format!("commit {n}")))
        .collect();
    let text = refresh_clean(57, &list);
    assert!(text.contains("(57 commits: ") && text.contains("and 47 more)"));
    assert!(sent[0].contains(&text), "{sent:?}");
    assert_eq!(
        fx.task("t1").history.last().unwrap().text,
        "refreshed: merged 57 commits"
    );
}

#[test]
fn a_paused_task_is_told_to_wait_after_a_refresh() {
    let list = [("abc1234".to_string(), "docs: a".to_string())];
    for conflicted in [false, true] {
        let (mut fx, _) = paused();
        assert!(replies(&edit(&mut fx, vec![refresh("t1")]))[0].is_ok());
        let op = fx
            .ops("HandBack")
            .last()
            .map(|(op, _)| *op)
            .expect("the paused task's turn is closed, so the refresh goes out");
        let files = vec!["src/a.rs".to_string()];
        let result = OpResult::HandedBack {
            files: if conflicted {
                files.clone()
            } else {
                Vec::new()
            },
            head: Some(if conflicted { HEAD } else { MERGE }.into()),
            onto: Some(HEAD.into()),
            merged: vec!["abc1234 docs: a".into()],
            merged_total: 1,
        };
        let sent = delivers(&fx.done(op, result));
        let (paused_text, working_text) = if conflicted {
            (refresh_conflict_paused(&files), refresh_conflict(&files))
        } else {
            (refresh_clean_paused(1, &list), refresh_clean(1, &list))
        };
        assert_eq!(sent, vec![paused_text.clone()], "conflicted: {conflicted}");
        assert!(!paused_text.contains("continue"), "{paused_text}");
        assert_ne!(paused_text, working_text);
        assert_eq!(fx.task("t1").state, TaskState::Blocked, "still paused");
    }
}
