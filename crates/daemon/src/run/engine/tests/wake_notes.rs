//! Milestone 9 task M9.9: decision 39's wake notes and the wake-up effect, decision
//! 16's digest read, and decision 40's edit log of every source.

use proto::{PlanEdit, RunState, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::kinds::{changes, reviewer_window, verdict};
use super::kinds_integration::{C1, mail_epic, merge_real};
use super::merge::pending_one;
use super::orch::{ORCH, add, edit_plan, launched, orch_tool};
use super::orch_restore::{restart, resume};
use super::planners::{planner_ended, planning_mail, submit_epic, task_in};
use crate::run::engine::{Effect, EventKind, OpResult, OrchEvent, ScoutEnd, notes_seq};
use crate::run::orch::contract::wake_text;

pub(super) fn notes(fx: &Fixture) -> Vec<String> {
    fx.run()
        .orch
        .orchestrator
        .as_ref()
        .map(|o| o.notes.clone())
        .unwrap_or_default()
}

/// A planned run with `t1` and `t2` (which depends on `t1`), submitted and approved
/// by the user; the orchestrator's notes emptied.
pub(super) fn approved() -> Fixture {
    let mut fx = launched(false);
    let mut t2 = add("t2", "mail");
    t2["task"]["deps"] = json!(["t1"]);
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth"), t2], "submit": true}),
    );
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    fx.approve();
    assert_eq!(fx.run().state, RunState::Running);
    clear(&mut fx);
    fx
}

pub(super) fn clear(fx: &mut Fixture) {
    let o = fx.run_mut().orch.orchestrator.as_mut().unwrap();
    o.notes.clear();
    o.note_seqs.clear();
}

fn cancel(id: &str) -> PlanEdit {
    PlanEdit::CancelTask { task_id: id.into() }
}

fn hold_verdict(fx: &mut Fixture, hold: &str, approve: bool) -> Vec<Effect> {
    let reply = fx.reply();
    let (run_id, hold) = (RUN_ID.to_string(), hold.to_string());
    fx.next(EventKind::Orch(if approve {
        OrchEvent::ApproveHold {
            reply,
            run_id,
            hold,
        }
    } else {
        OrchEvent::RejectHold {
            reply,
            run_id,
            hold,
        }
    }))
}

/// `mail`'s sub-planner submitted `m1`, whose round awaits the user.
fn awaiting_mail() -> Fixture {
    let mut fx = planning_mail(false);
    submit_epic(&mut fx, json!([task_in("m1", "mail")]));
    clear(&mut fx);
    fx
}

/// A run driven to a point, and the one note line it must then hold.
type Scenario = Box<dyn Fn() -> (Fixture, String)>;

/// Each scenario and the one line it must add.
fn scenarios() -> Vec<(&'static str, Scenario)> {
    let long = "q".repeat(200);
    vec![
        (
            "the plan approved",
            Box::new(|| {
                let mut fx = launched(false);
                edit_plan(
                    &mut fx,
                    json!({"edits": [add("t1", "auth")], "submit": true}),
                );
                clear(&mut fx);
                fx.approve();
                (fx, "the user approved the plan".into())
            }),
        ),
        (
            "the plan rejected",
            Box::new(|| {
                let mut fx = launched(false);
                edit_plan(
                    &mut fx,
                    json!({"edits": [add("t1", "auth")], "submit": true}),
                );
                clear(&mut fx);
                let reply = fx.reply();
                fx.next(EventKind::Reject {
                    reply,
                    run_id: RUN_ID.into(),
                });
                (fx, "the user rejected the plan; run discarded".into())
            }),
        ),
        (
            "a hold approved",
            Box::new(|| {
                let mut fx = awaiting_mail();
                hold_verdict(&mut fx, "epic:mail", true);
                (fx, "the user approved hold epic:mail".into())
            }),
        ),
        (
            "a hold rejected",
            Box::new(|| {
                let mut fx = awaiting_mail();
                hold_verdict(&mut fx, "epic:mail", false);
                (fx, "the user rejected hold epic:mail".into())
            }),
        ),
        (
            "a user edit",
            Box::new(|| {
                let mut fx = approved();
                edit(&mut fx, vec![PlanEdit::Pause]);
                (fx, "the user edited the plan: pause".into())
            }),
        ),
        (
            "a task blocked",
            Box::new(move || {
                let mut fx = approved();
                let window = fx.launch_all()[0].1;
                let args = json!({"kind": "question", "reason": long.clone()});
                fx.tool(window, "task_blocked", args);
                (fx, format!("t1 blocked (question): {}", "q".repeat(120)))
            }),
        ),
        (
            "a sub-planner finished",
            Box::new(|| {
                let mut fx = planning_mail(true);
                clear(&mut fx);
                submit_epic(&mut fx, json!([task_in("m1", "mail")]));
                (fx, "sub-planner mail finished with 1 task".into())
            }),
        ),
        (
            "a sub-planner failed",
            Box::new(|| {
                let mut fx = planning_mail(true);
                clear(&mut fx);
                let failed = ScoutEnd::Failed {
                    reason: "boom".into(),
                };
                planner_ended(&mut fx, ("mail", 1), failed);
                (fx, "sub-planner mail failed: boom".into())
            }),
        ),
        (
            "a run scout reported",
            Box::new(|| {
                let mut fx = approved();
                let args = json!({"id": "api", "question": "Where is the API?", "area": ["crates/api/**"]});
                let (ok, _) = super::orch::answer(&orch_tool(&mut fx, ORCH, "spawn_scout", args));
                assert!(ok);
                clear(&mut fx);
                fx.next(EventKind::Orch(OrchEvent::ScoutEnded {
                    run_id: RUN_ID.into(),
                    scout_id: format!("{H4}-api"),
                    outcome: ScoutEnd::Reported,
                    usage: Default::default(),
                }));
                (fx, format!("scout {H4}-api reported"))
            }),
        ),
        (
            "an integration review asked for changes",
            Box::new(|| {
                let mut fx = mail_epic(&["m1"]);
                merge_real(&mut fx, "m1", C1);
                let window = reviewer_window(&mut fx, "mail-int1", "+x\n");
                clear(&mut fx);
                verdict(&mut fx, window, "mail-int1", changes());
                (
                    fx,
                    "integration review of epic mail: changes (1 critical, 1 important)".into(),
                )
            }),
        ),
        (
            "an integration review approved",
            Box::new(|| {
                let mut fx = mail_epic(&["m1"]);
                merge_real(&mut fx, "m1", C1);
                let window = reviewer_window(&mut fx, "mail-int1", "+x\n");
                clear(&mut fx);
                verdict(&mut fx, window, "mail-int1", super::kinds::approve());
                (fx, "integration review of epic mail: approve".into())
            }),
        ),
        (
            "the run halted",
            Box::new(|| {
                let mut fx = approved();
                fx.run_mut().pending_ops.clear();
                let task = fx.task_mut("t1");
                task.state = TaskState::MergeQueue;
                task.head = Some(HEAD.into());
                fx.run_mut().merge_queue.push("t1".into());
                fx.tick();
                let (op, _) = pending_one(&fx, "MergeCandidate", Some("t1"));
                clear(&mut fx);
                fx.done(
                    op,
                    OpResult::RefMoved {
                        reason: "the run branch moved".into(),
                    },
                );
                (fx, "the run halted: the run branch moved".into())
            }),
        ),
        (
            "the daemon restarted",
            Box::new(|| {
                let mut fx = launched(false);
                restart(&mut fx);
                clear(&mut fx);
                resume(&mut fx);
                (
                    fx,
                    "the daemon restarted and your session was resumed".into(),
                )
            }),
        ),
    ]
}

#[test]
fn each_note_source_adds_its_exact_line() {
    for (name, scenario) in scenarios() {
        let (fx, line) = scenario();
        let notes = notes(&fx);
        assert!(notes.contains(&line), "{name}: {line:?} not in {notes:#?}");
    }
}

#[test]
fn orchestrators_own_edits_add_no_note() {
    // The orchestrator cancels t1: t2 becomes blocked(dep_cancelled), and no note.
    let mut fx = approved();
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [{"op": "cancel_task", "task_id": "t1"}]}),
    );
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    assert_eq!(fx.task("t2").state, TaskState::Blocked);
    assert_eq!(notes(&fx), Vec::<String>::new());
    // The same edit by the user: its edit and the block it caused.
    let mut fx = approved();
    edit(&mut fx, vec![cancel("t1")]);
    assert_eq!(
        notes(&fx),
        vec![
            "the user edited the plan: cancel t1".to_string(),
            "t2 blocked (dep_cancelled): dependency t1 was cancelled".to_string(),
        ]
    );
}

#[test]
fn notes_are_capped_at_20_with_the_earlier_line() {
    let mut fx = approved();
    for n in 0..24 {
        let pause = if n % 2 == 0 {
            PlanEdit::Pause
        } else {
            PlanEdit::Resume
        };
        edit(&mut fx, vec![pause]);
    }
    let notes_now = notes(&fx);
    assert_eq!(notes_now.len(), 20, "{notes_now:#?}");
    assert_eq!(notes_now[0], "+5 earlier changes");
    assert_eq!(notes_now[19], "the user edited the plan: resume");
    let revs = fx
        .run()
        .orch
        .orchestrator
        .as_ref()
        .unwrap()
        .note_seqs
        .clone();
    assert_eq!(revs.len(), 20);
    // Two more: the earlier line counts them.
    edit(&mut fx, vec![PlanEdit::Pause]);
    edit(&mut fx, vec![PlanEdit::Resume]);
    let notes_now = notes(&fx);
    assert_eq!(notes_now.len(), 20);
    assert_eq!(notes_now[0], "+7 earlier changes");
    // A sub-planner's note (M9.8's source) goes through the same cap.
    let mut fx = planning_mail(true);
    for _ in 0..20 {
        edit(&mut fx, vec![PlanEdit::Pause]);
        edit(&mut fx, vec![PlanEdit::Resume]);
    }
    let failed = ScoutEnd::Failed {
        reason: "boom".into(),
    };
    planner_ended(&mut fx, ("mail", 1), failed);
    let notes_now = notes(&fx);
    assert_eq!(notes_now.len(), 20, "{notes_now:#?}");
    assert!(notes_now[0].ends_with(" earlier changes"), "{notes_now:#?}");
    assert_eq!(notes_now[19], "sub-planner mail failed: boom");
}

pub(super) fn wakes(effects: &[Effect]) -> Vec<(u32, String, u64)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::WakeOrchestrator {
                run_id,
                window_id,
                text,
                digest_revision,
                ..
            } if run_id == RUN_ID => Some((*window_id, text.clone(), *digest_revision)),
            _ => None,
        })
        .collect()
}

#[test]
fn wake_effect_needs_live_window_setting_and_new_revision() {
    let mut fx = approved();
    let effects = edit(&mut fx, vec![PlanEdit::Pause]);
    let rev = fx.run().orch.digest_rev;
    assert_eq!(
        wakes(&effects),
        vec![(ORCH, wake_text(RUN_ID, &notes(&fx)), rev)]
    );
    // A window that is not live is not woken.
    let mut fx = approved();
    fx.run_mut().orch.orchestrator.as_mut().unwrap().live = false;
    assert!(wakes(&edit(&mut fx, vec![PlanEdit::Pause])).is_empty());
    // Nor with `wake_orchestrator = false`; the notes are still kept.
    let mut fx = approved();
    fx.run_mut().limits.orch.wake_orchestrator = false;
    assert!(wakes(&edit(&mut fx, vec![PlanEdit::Pause])).is_empty());
    assert_eq!(notes(&fx).len(), 1);
    // Nor for a revision already woken for.
    let mut fx = approved();
    fx.run_mut()
        .orch
        .orchestrator
        .as_mut()
        .unwrap()
        .last_wake_rev = u64::MAX;
    assert!(wakes(&edit(&mut fx, vec![PlanEdit::Pause])).is_empty());
    // Nor with no note pending.
    let mut fx = approved();
    assert!(wakes(&fx.tick()).is_empty());
}

/// The orchestrator read the digest built from the run as it is now.
pub(super) fn read_now(fx: &mut Fixture) -> Vec<Effect> {
    let (revision, seq) = (fx.run().orch.digest_rev, notes_seq(fx.run()));
    read(fx, revision, seq)
}

pub(super) fn read(fx: &mut Fixture, revision: u64, notes_seq: u64) -> Vec<Effect> {
    fx.next(EventKind::Orch(OrchEvent::DigestRead {
        run_id: RUN_ID.into(),
        digest_revision: revision,
        notes_seq,
    }))
}

#[test]
fn digest_read_drops_notes_up_to_the_revision() {
    let mut fx = approved();
    edit(&mut fx, vec![PlanEdit::Pause]);
    let (first, first_seq) = (fx.run().orch.digest_rev, notes_seq(fx.run()));
    edit(&mut fx, vec![PlanEdit::Resume]);
    let second = fx.run().orch.digest_rev;
    assert!(second > first);
    assert_eq!(notes(&fx).len(), 2);
    read(&mut fx, first, first_seq);
    assert_eq!(
        notes(&fx),
        vec!["the user edited the plan: resume".to_string()]
    );
    let now = fx.now;
    assert_eq!(fx.run().orch.digest_read_at, Some(now));
    read_now(&mut fx);
    assert!(notes(&fx).is_empty());
    assert!(
        fx.run()
            .orch
            .orchestrator
            .as_ref()
            .unwrap()
            .note_seqs
            .is_empty()
    );
}

#[test]
fn woken_clears_delivered_notes() {
    let mut fx = approved();
    let effects = edit(&mut fx, vec![PlanEdit::Pause]);
    let (_, _, revision) = wakes(&effects)[0].clone();
    let notes_seq = notes_seq(fx.run());
    let effects = fx.next(EventKind::Orch(OrchEvent::OrchestratorWoken {
        run_id: RUN_ID.into(),
        digest_revision: revision,
        notes_seq,
        request: None,
    }));
    let o = fx.run().orch.orchestrator.clone().unwrap();
    assert!(o.notes.is_empty());
    assert_eq!((o.last_wake_rev, o.wakes), (revision, 1));
    assert!(wakes(&effects).is_empty());
}

#[test]
fn completion_note() {
    let mut fx = launched(true);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    merge_real(&mut fx, "t1", C1);
    let (op, _) = fx.op("VerifyRefs");
    // The merge's candidate check was green on the run head: no final check.
    fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete);
    assert_eq!(
        notes(&fx).last().map(String::as_str),
        Some("the run is complete; write your summary with edit_plan summary")
    );
}

fn last_edit(fx: &Fixture) -> crate::run::edit_log::PlanEditRecord {
    fx.run().plan_edits.last().cloned().unwrap()
}

#[test]
fn edit_log_records_every_source_rejections_and_recipients() {
    let mut fx = planning_mail(false);
    let since = fx.run().plan_edits_since_approval;
    // The orchestrator's accepted batch.
    let effects = edit_plan(&mut fx, json!({"edits": [add("t2", "auth")]}));
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    let record = last_edit(&fx);
    assert_eq!(
        (
            record.source.as_str(),
            record.accepted,
            record.text.as_str()
        ),
        ("orchestrator", true, "add t2")
    );
    // Its rejected batch: the epic names no epic of the run.
    let mut bad = add("t3", "auth");
    bad["task"]["epic"] = json!("nope");
    let effects = edit_plan(&mut fx, json!({"edits": [bad]}));
    assert!(matches!(&replies(&effects)[..], [Err(_)]), "{effects:#?}");
    let record = last_edit(&fx);
    assert_eq!(record.source, "orchestrator");
    assert!(!record.accepted);
    assert_eq!(
        record.error.as_deref(),
        Some("task t3: epic: nope is not an epic of this run; create it with spawn_subplanner")
    );
    assert_eq!(record.text, "add t3");
    // A sub-planner's rejected batch, then its accepted one.
    let effects = submit_epic(
        &mut fx,
        json!([{"op": "add_task", "task": {"id": "m0", "title": "x", "size": "S",
            "owns": ["crates/auth/**"], "brief": "b", "acceptance": ["a"]}}]),
    );
    assert!(matches!(&replies(&effects)[..], [Err(_)]), "{effects:#?}");
    let record = last_edit(&fx);
    assert_eq!(
        (record.source.as_str(), record.accepted),
        ("planner:mail", false)
    );
    assert!(record.error.is_some());
    submit_epic(&mut fx, json!([task_in("m1", "mail")]));
    let record = last_edit(&fx);
    assert_eq!(
        (record.source.as_str(), record.accepted),
        ("planner:mail", true)
    );
    // The user's.
    edit(&mut fx, vec![cancel("t2")]);
    let record = last_edit(&fx);
    assert_eq!((record.source.as_str(), record.accepted), ("user", true));
    // Only the three accepted batches count as edits after approval, not the two
    // rejected ones.
    assert_eq!(fx.run().plan_edits_since_approval, since + 3);
    // A message's recipients reach the snapshot with every other field.
    crate::run::edit_log::record(
        fx.run_mut(),
        &[PlanEdit::Pause],
        9_000,
        &crate::run::orch::EditSource::Orchestrator,
        crate::run::edit_log::EditOutcome::Accepted {
            recipients: vec!["t1".into()],
        },
    );
    let snapshot = crate::run::snapshot::snapshot(&fx.state, 9_001);
    let info = &snapshot.runs[0];
    let shown: Vec<(String, bool, Option<String>, Vec<String>)> = info
        .plan_edits
        .iter()
        .map(|e| {
            (
                e.source.clone(),
                e.accepted,
                e.error.clone(),
                e.recipients.clone(),
            )
        })
        .collect();
    assert!(
        shown.contains(&(
            "orchestrator".to_string(),
            true,
            None,
            vec!["t1".to_string()]
        )),
        "{shown:#?}"
    );
    assert!(
        shown
            .iter()
            .any(|(source, accepted, error, _)| source == "planner:mail"
                && !accepted
                && error.is_some()),
        "{shown:#?}"
    );
}
