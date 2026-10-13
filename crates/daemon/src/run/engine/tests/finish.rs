//! Milestone 9.15 decision 26: `run discard --reason` reaches the run and its run record
//! of `history.jsonl`; an accept ignores a reason.

use proto::{DiffStats, FinishAction, HistoryLine, RunState};

use super::fixture::*;
use super::history::{appends, start_history};
use super::merge::{commit, doc_task, merge, pending_one, to_queue, window_of};
use crate::run::engine::{EventKind, OpResult};

/// A complete one-task run whose history is on, its task record appended.
fn complete() -> Fixture {
    let (mut fx, windows) = start_history(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    let (measure, _) = fx.op("MeasureDiff");
    let effects = fx.done(measure, OpResult::DiffMeasured(DiffStats::default()));
    fx.done(appends(&effects)[0].0, OpResult::HistoryAppended);
    let (refs, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(refs, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete);
    fx
}

/// `action` with `reason` on the complete run, finished; the run record appended.
fn finish(action: FinishAction, reason: &str) -> (Fixture, proto::RunRecord) {
    let mut fx = complete();
    let reply = fx.reply();
    fx.next(EventKind::Finish {
        reply,
        run_id: RUN_ID.into(),
        action,
        reason: Some(reason.into()),
    });
    let name = match action {
        FinishAction::Accept => "Accept",
        FinishAction::Discard => "Discard",
    };
    let (op, _) = fx.op(name);
    let effects = fx.done(
        op,
        OpResult::Finished {
            outcome: "done".into(),
            kept_branches: Vec::new(),
        },
    );
    let added = appends(&effects);
    assert_eq!(added.len(), 1, "{effects:#?}");
    let HistoryLine::Run(record) = added[0].2.clone() else {
        panic!("a run line");
    };
    (fx, record)
}

#[test]
fn discard_reason_reaches_run_record() {
    let why = "wrong approach: the cache belongs in the client";
    let (fx, record) = finish(FinishAction::Discard, why);
    assert_eq!(fx.run().state, RunState::Discarded);
    assert_eq!(fx.run().discard_reason.as_deref(), Some(why));
    assert_eq!(record.outcome, "discarded");
    assert_eq!(record.discard_reason.as_deref(), Some(why));

    // An accept ignores a reason.
    let (fx, record) = finish(FinishAction::Accept, why);
    assert_eq!(fx.run().state, RunState::Accepted);
    assert_eq!(fx.run().discard_reason, None);
    assert_eq!(record.discard_reason, None);

    // One line, clipped to 500 characters.
    let long = format!("{}\n{}", "a".repeat(300), "b".repeat(300));
    let (fx, record) = finish(FinishAction::Discard, &long);
    let stored = fx.run().discard_reason.clone().expect("a reason");
    assert_eq!(stored.chars().count(), proto::DISCARD_REASON_MAX);
    assert!(!stored.contains('\n'), "{stored}");
    assert_eq!(stored, format!("{} {}", "a".repeat(300), "b".repeat(199)));
    assert_eq!(record.discard_reason, Some(stored));

    // A blank reason is none.
    let (fx, record) = finish(FinishAction::Discard, " \n ");
    assert_eq!(fx.run().discard_reason, None);
    assert_eq!(record.discard_reason, None);
}

/// Review I1 of task M9.15.7: the reason waits with the `Discard` op and reaches the run
/// only when the discard succeeds, so a failed discard followed by an accept records an
/// accept with no reason.
#[test]
fn a_failed_discard_leaves_no_reason_for_a_later_accept() {
    let mut fx = complete();
    let reply = fx.reply();
    fx.next(EventKind::Finish {
        reply,
        run_id: RUN_ID.into(),
        action: FinishAction::Discard,
        reason: Some("abandoned".into()),
    });
    let (op, _) = fx.op("Discard");
    fx.done(
        op,
        OpResult::Failed {
            message: "git: cannot remove the worktree".into(),
        },
    );
    assert_eq!(fx.run().state, RunState::Complete);
    assert_eq!(fx.run().discard_reason, None);

    let reply = fx.reply();
    fx.next(EventKind::Finish {
        reply,
        run_id: RUN_ID.into(),
        action: FinishAction::Accept,
        reason: None,
    });
    let (op, _) = fx.op("Accept");
    let effects = fx.done(
        op,
        OpResult::Finished {
            outcome: "done".into(),
            kept_branches: Vec::new(),
        },
    );
    let added = appends(&effects);
    assert_eq!(added.len(), 1, "{effects:#?}");
    let HistoryLine::Run(record) = added[0].2.clone() else {
        panic!("a run line");
    };
    assert_eq!(record.outcome, "accepted");
    assert_eq!(record.discard_reason, None);
    assert_eq!(fx.run().discard_reason, None);
}

/// The journaled `Discard` op carries its reason, and leaves the key out while `None`,
/// so a pending discard from before it decodes (ruling C2's rule).
#[test]
fn a_discard_op_journals_its_reason() {
    use crate::run::engine::OpKind;
    let op = |reason: Option<&str>| OpKind::Discard {
        root: "/r".into(),
        worktrees: Vec::new(),
        branch_prefix: "anthrex/x/".into(),
        reason: reason.map(str::to_string),
    };
    let json = serde_json::to_value(op(None)).unwrap();
    assert!(!json.to_string().contains("reason"), "{json}");
    assert_eq!(serde_json::from_value::<OpKind>(json).unwrap(), op(None));
    let with = op(Some("too big"));
    let back: OpKind = serde_json::from_str(&serde_json::to_string(&with).unwrap()).unwrap();
    assert_eq!(back, with);
}
