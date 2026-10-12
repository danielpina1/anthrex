//! Milestone 9.13 decisions 11 and 13 (task M9.13.6): which gate files stay pinned, and
//! the halt that follows a gate command refused because one of them changed. The
//! executor's check is tested against real files in `daemon/tests/it/run_gate_pins.rs`.

use proto::{HoldKind, HoldState, RunState, TaskState};
use serde_json::json;

use super::control::{override_task, resume};
use super::dispatch::replies;
use super::fixture::*;
use super::full::{block, full_jobs, merge_tiered};
use super::merge::{candidate, commit, doc_task, merge, pending, pending_one, start, to_queue};
use super::orch::{add, edit_plan, error};
use crate::run::engine::gate_pins::{GATE_CHANGED, pinned};
use crate::run::engine::user_only;
use crate::run::engine::{Effect, EventKind, OpResult};
use crate::run::gate_words::PinnedFile;
use crate::run::model::Run;
use crate::run::orch::GateHoldRecord;

const SCRIPT: &str = "scripts/check.sh";

/// `GATE_CHANGED` for [`SCRIPT`].
fn changed() -> String {
    format!(
        "the gate's script {SCRIPT} differs from the base commit's; the run changed what measures it"
    )
}

fn file(path: &str) -> PinnedFile {
    PinnedFile {
        path: path.to_string(),
        blob: "a".repeat(40),
    }
}

fn gate_files() -> Vec<PinnedFile> {
    vec![file(SCRIPT), file("Makefile")]
}

fn refused() -> OpResult {
    OpResult::GateRefused {
        path: SCRIPT.to_string(),
    }
}

fn paths(run: &Run) -> Vec<String> {
    pinned(run).into_iter().map(|f| f.path).collect()
}

fn both() -> Vec<String> {
    vec![SCRIPT.to_string(), "Makefile".to_string()]
}

/// An approved orchestrated run whose `t1` owns [`SCRIPT`] exactly.
fn granted() -> Fixture {
    let mut fx = super::wake_notes::approved();
    fx.run_mut().gate_files = gate_files();
    fx.task_mut("t1").spec.owns = vec![SCRIPT.to_string()];
    fx
}

/// PR #60 review I3: a name pinned absent (`blob` empty) is granted as any gate file
/// is, by a released task's `owns` naming it exactly.
#[test]
fn a_granted_absent_name_is_not_pinned() {
    let mut fx = super::wake_notes::approved();
    let absent = PinnedFile {
        path: "GNUmakefile".to_string(),
        blob: String::new(),
    };
    fx.run_mut().gate_files = vec![file("Makefile"), absent.clone()];
    assert_eq!(pinned(fx.run()), [file("Makefile"), absent]);
    fx.task_mut("t1").spec.owns = vec!["GNUmakefile".to_string()];
    assert_eq!(paths(fx.run()), ["Makefile"]);
}

#[test]
fn a_granted_gate_file_is_not_pinned() {
    assert_eq!(GATE_CHANGED.replace("{path}", SCRIPT), changed());
    let fx = granted();
    assert_eq!(paths(fx.run()), ["Makefile"]);
    assert_eq!(pinned(fx.run())[0], file("Makefile"));

    // A cancelled task grants nothing.
    let mut fx = granted();
    fx.task_mut("t1").state = TaskState::Cancelled;
    assert_eq!(paths(fx.run()), both());

    // Nor does a task under a hold the user has not decided.
    let mut fx = granted();
    fx.task_mut("t1").orch.gate_hold = Some("epic:e".to_string());
    fx.run_mut().orch.gate_holds.push(GateHoldRecord {
        id: "epic:e".to_string(),
        kind: HoldKind::Epic { epic: "e".into() },
        state: HoldState::Awaiting,
        tasks: vec!["t1".to_string()],
        created_at: 1,
        decided_at: None,
        decided_by: None,
    });
    assert_eq!(paths(fx.run()), both());
    // Once the user approves the hold, the grant counts.
    fx.run_mut().orch.gate_holds[0].state = HoldState::Approved;
    assert_eq!(paths(fx.run()), ["Makefile"]);

    // C18: nor a task of a plan the user has not approved yet.
    let mut fx = super::orch::launched(false);
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    assert!(fx.run().approved_at.is_none());
    fx.run_mut().gate_files = gate_files();
    fx.task_mut("t1").spec.owns = vec![SCRIPT.to_string()];
    assert_eq!(paths(fx.run()), both());

    // A run from before 9.13 pins nothing.
    let mut fx = granted();
    fx.run_mut().gate_files.clear();
    assert!(pinned(fx.run()).is_empty());
}

/// Gives the run an orchestrator whose plan is submitted.
fn with_orchestrator(fx: &mut Fixture) {
    super::bisect::with_orchestrator(fx);
}

fn orch_resume(fx: &mut Fixture) -> Vec<Effect> {
    let edit = json!({"op": "resume_run", "reason": "try again"});
    edit_plan(fx, json!({"edits": [edit]}))
}

/// The run is halted on [`changed`], only the user may resume it, and the user's
/// resume is accepted.
fn assert_users_halt(fx: &mut Fixture) {
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(fx.run().halted_reason.as_deref(), Some(changed().as_str()));
    assert!(user_only::halt(fx.run()));
    with_orchestrator(fx);
    let effects = orch_resume(fx);
    assert_eq!(
        error(&effects),
        format!(
            "run {RUN_ID} waits on something only the user can fix, and they have been alerted: {}",
            changed()
        )
    );
    assert_eq!(fx.run().state, RunState::Halted);
}

/// The user's `run resume`, accepted; its effects.
fn user_resume(fx: &mut Fixture) -> Vec<Effect> {
    let effects = resume(fx);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_ne!(fx.run().state, RunState::Halted);
    effects
}

#[test]
fn gate_refused_halts_the_run_user_only() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    super::merge::claim(&mut fx, "t1", windows[0].1, &super::merge::head_of("t1"));
    let (op, _) = pending_one(&fx, "Check", Some("t1"));
    fx.done(op, refused());
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    let block = t1.block.clone().expect("blocked");
    assert_eq!(block.text, changed());
    assert!(block.user_only);
    assert_eq!(t1.gate_op, None);
    assert_users_halt(&mut fx);
    user_resume(&mut fx);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    assert!(pending(&fx, "Check", Some("t1")).is_empty());
}

#[test]
fn gate_refused_on_the_final_check_halts_only() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", windows[0].1);
    merge(&mut fx, "t1", &commit(1));
    // The run head is not the last green candidate, so the final check runs.
    fx.run_mut().last_green_candidate = None;
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(op, OpResult::RefsOk);
    let (op, _) = pending_one(&fx, "Check", None);
    fx.done(op, refused());
    assert!(fx.run().tasks.iter().all(|t| t.block.is_none()));
    assert_eq!(fx.task("t1").state, TaskState::Merged);
    assert!(pending(&fx, "Check", None).is_empty());
    assert!(!fx.run().final_check_failed);
    assert_users_halt(&mut fx);
    user_resume(&mut fx);
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(op, OpResult::RefsOk);
    let (again, _) = pending_one(&fx, "Check", None);
    assert_ne!(again, op);
    assert_eq!(fx.run().state, RunState::Running);
}

#[test]
fn gate_refused_clears_each_kinds_waiting_state() {
    // A merge candidate's check: the task leaves the queue, blocked for the user.
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", windows[0].1);
    let (op, _) = candidate(&fx, "t1");
    assert_eq!(fx.task("t1").merge_op, Some(op));
    fx.done(op, refused());
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert!(t1.block.as_ref().is_some_and(|b| b.user_only));
    assert_eq!(t1.merge_op, None);
    assert!(fx.run().merge_queue.is_empty());
    assert_users_halt(&mut fx);
    user_resume(&mut fx);
    assert!(pending(&fx, "MergeCandidate", Some("t1")).is_empty());
    assert_eq!(fx.task("t1").state, TaskState::Blocked);

    // A tier-3 job: the run has no job in flight, and the user's resume issues it again.
    let profile = super::full::profile();
    let tasks = [doc_task("t1", ""), doc_task("t2", "")];
    let (mut fx, windows) = super::merge::start_on(&profile, &tasks);
    let window = |id: &str| super::merge::window_of(&windows, id);
    block(&mut fx, "t2", window("t2"));
    merge_tiered(&mut fx, "t1", window("t1"), &commit(1));
    let since = fx.run().queue_idle_since.unwrap();
    let (op, _) = full_jobs(&fx.send(since + 120, EventKind::Tick))[0].clone();
    assert_eq!(fx.run().full_op, Some(op));
    fx.done(op, refused());
    assert_eq!(fx.run().full_op, None);
    assert!(pending(&fx, "Tier", None).is_empty());
    assert_ne!(
        fx.task("t2").block.as_ref().map(|b| b.text.clone()),
        Some(changed())
    );
    assert_users_halt(&mut fx);
    let jobs = full_jobs(&user_resume(&mut fx));
    assert_eq!(jobs.len(), 1);
    assert_ne!(jobs[0].0, op);
    assert_eq!(fx.run().full_op, Some(jobs[0].0));

    // A bisect probe: its probe is cleared, and the user's resume probes again.
    let mut fx = super::bisect::merged(&["t1", "t2", "t3"], "");
    super::bisect::red_full(&mut fx);
    let (op, spec) = super::bisect::probe(&fx);
    fx.done(op, refused());
    let bisect = fx.run().stage(1).unwrap().bisect.clone().expect("kept");
    assert_eq!(bisect.probe, None);
    assert_eq!(fx.run().full_op, None);
    assert!(pending(&fx, "TestAt", None).is_empty());
    assert_users_halt(&mut fx);
    let effects = user_resume(&mut fx);
    let probes = ops_in(&effects, "TestAt");
    assert_eq!(probes.len(), 1, "{effects:#?}");
    assert_ne!(probes[0].0, op);
    assert!(
        matches!(&probes[0].1, crate::run::engine::OpKind::TestAt(s) if s.commit == spec.commit)
    );
}

/// [`super::wake_notes::approved`] with `t1` blocked and its start commit recorded, the
/// run's gate files set: an override counts its commits.
fn blocked_with_commits() -> Fixture {
    let mut fx = super::wake_notes::approved();
    fx.run_mut().gate_files = gate_files();
    let windows = fx.launch_all();
    let (_, window) = windows.iter().find(|(t, _)| t == "t1").cloned().unwrap();
    super::control::blocked(&mut fx, window, "environment", "locked");
    fx.task_mut("t1").start_commit = Some(BASE.into());
    fx
}

/// The user's override of `t1`, whose one commit changes [`SCRIPT`]; the reply.
fn user_override(fx: &mut Fixture) -> String {
    override_task(fx, "t1", "the user read it");
    let (count, _) = fx.op("CountCommits");
    let effects = fx.done(
        count,
        OpResult::Commits {
            count: 1,
            head: HEAD.into(),
            protected: vec![SCRIPT.to_string()],
        },
    );
    replies(&effects)[0].clone().unwrap()
}

#[test]
fn a_user_override_of_a_pinned_gate_file_says_it_stays_pinned() {
    let lead = "task t1 goes to the merge queue without review: the user read it; it changes protected scripts/check.sh";
    let mut fx = blocked_with_commits();
    assert_eq!(
        user_override(&mut fx),
        format!("{lead}; scripts/check.sh stays pinned until a task's owns names it (run edit)")
    );
    // Granted by another approved task's `owns`: not pinned, so no clause.
    let mut fx = blocked_with_commits();
    fx.task_mut("t2").spec.owns.push(SCRIPT.to_string());
    assert_eq!(user_override(&mut fx), lead);
}

/// Task 6 review I-1: a base sync (`from: 0`) the gate check refused is due again, as a
/// base sync, so the user's resume does not lose it.
#[test]
fn a_refused_base_sync_is_due_again() {
    use super::delivery_sync::{base_sync, base_syncs, conflicting, fetched};
    use super::delivery_watch_adopt::{poll_stage, two_stages};
    let (mut fx, _) = two_stages(true);
    let h1 = fx.run().stage_head(1).unwrap().to_string();
    poll_stage(&mut fx, 1, conflicting(11, &h1));
    fetched(&mut fx, &commit(50), None);
    let (op, spec) = base_sync(&fx);
    assert!(fx.run().delivery.base_sync_due.is_empty(), "taken");
    fx.done(op, refused());
    assert_eq!(fx.run().state, RunState::Halted);
    let due = &fx.run().delivery.base_sync_due;
    assert_eq!(due.get(&spec.to), Some(&spec.from_head), "{due:?}");
    // After the user's resume it is due, or already issued again.
    user_resume(&mut fx);
    let due = fx.run().delivery.base_sync_due.contains_key(&spec.to);
    assert!(due || !base_syncs(&fx).is_empty());
}

/// The same for a stage propagate (`from > 0`): its target is in `propagate_due` again.
#[test]
fn a_refused_stage_propagate_is_due_again() {
    use super::propagate::{propagate, stages};
    let (mut fx, windows) = stages(true);
    super::propagate::merge_t1(&mut fx, &windows);
    let (op, spec) = propagate(&fx);
    assert!(fx.run().propagate_due.is_empty(), "taken");
    fx.done(op, refused());
    assert_eq!(fx.run().state, RunState::Halted);
    assert!(fx.run().propagate_due.contains(&spec.to));
}
