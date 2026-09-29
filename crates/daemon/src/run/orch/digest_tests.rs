//! Task M9.6: the digest (decision 16, Interfaces "The digest"). Pure.

use proto::{
    BlockReason, HoldState, IntegrationState, MessageKind, RunPath, RunState, Severity,
    TaskNoteKind, TaskState, TokenUsage, Verdict,
};
use serde_json::{Value, json};

use super::*;
use crate::run::edit_log::PlanEditRecord;
use crate::run::orch::test_support::*;
use crate::run::orch::{EpicRecord, PlannerPhase, RefreshState, RunScoutState};

/// 12:31 UTC on a fixed day.
const NOW: u64 = 1_790_000_000 - 1_790_000_000 % 86_400 + 12 * 3600 + 31 * 60;

fn at(hh: u64, mm: u64) -> u64 {
    NOW - NOW % 86_400 + hh * 3600 + mm * 60
}

/// The fixed run of the shape test: four tasks, an orchestrator, a hold, two scouts, a
/// planner with an integration review, a task note, a message and an edit.
fn fixed_run() -> Run {
    let mut run = run_with(&[
        task_toml("t0", "M", "[\"crates/proto/**\"]", ""),
        task_toml("t1", "S", "[\"crates/a/**\"]", "epic = \"a\""),
        task_toml("t2", "M", "[\"crates/b/**\"]", "deps = [\"t0\"]"),
        task_toml("t3", "S", "[\"crates/c/**\"]", "epic = \"c\""),
    ]);
    run.state = RunState::Running;
    run.path = Some(RunPath::Large);
    run.approved_at = Some(at(11, 2));
    run.approved_by = Some("user".into());
    run.orch.digest_rev = 42;
    let mut orch = orchestrator();
    orch.notes = vec!["t2 blocked (question): which endpoint?".into()];
    run.orch.orchestrator = Some(orch);
    run.orch.gate_holds = vec![hold("epic:c", HoldState::Awaiting, &["t3"], None)];
    run.orch.run_scouts = vec![
        scout(
            "3f9a-daemon",
            RunScoutState::Reported,
            &["crates/daemon/**"],
        ),
        scout(
            "3f9a-tui",
            RunScoutState::Failed {
                reason: "the daemon restarted during this scout".into(),
            },
            &["crates/tui/**"],
        ),
    ];
    let mut epic = EpicRecord::new("a", PlannerPhase::Finished);
    epic.edits_rejected = 1;
    epic.last_rejection = Some("task a1: size: needs evidence".into());
    epic.integration_state = IntegrationState::Changes;
    epic.integration_rounds = 1;
    run.orch.epics = vec![epic];

    task_mut(&mut run, "t0").state = TaskState::Merged;
    event(task_mut(&mut run, "t0"), at(11, 30), "merged");
    let t1 = task_mut(&mut run, "t1");
    t1.state = TaskState::Working;
    t1.route = route();
    t1.orch.messages = vec![message(
        at(12, 0),
        MessageKind::Info,
        "use the v2 API",
        false,
    )];
    t1.orch.refresh = Some(RefreshState::Due);
    t1.orch.worker_notes = vec![note(
        at(12, 29),
        TaskNoteKind::Discovery,
        "the hook fires twice",
    )];
    let t2 = task_mut(&mut run, "t2");
    block(t2, BlockReason::Question, "which endpoint?");
    t2.rung = 1;
    t2.reviews = vec![review(
        1,
        Some(Verdict::Changes),
        &[(Severity::Critical, "unchecked unwrap")],
    )];
    event(t2, at(12, 31), "review r1 changes");
    let t3 = task_mut(&mut run, "t3");
    t3.orch.gate_hold = Some("epic:c".into());
    run.plan_edits = vec![PlanEditRecord {
        at: at(11, 40),
        text: "message t1 (info)".into(),
        source: "orchestrator".into(),
        accepted: true,
        error: None,
        recipients: vec!["t1".into()],
    }];
    run
}

#[test]
fn digest_shape_matches_the_interface() {
    let expected: Value = serde_json::from_str(include_str!("digest_fixture.json")).unwrap();
    let got = digest(&fixed_run(), NOW);
    assert_eq!(
        got,
        expected,
        "got:\n{}",
        serde_json::to_string_pretty(&got).unwrap()
    );
}

#[test]
fn counter_changes_do_not_change_the_fingerprint() {
    let run = fixed_run();
    let before = fingerprint(&run);
    let mut counted = run.clone();
    let usage = TokenUsage {
        input: 900,
        output: 90,
        cache_read: 9_000,
        cache_write: 9,
    };
    task_mut(&mut counted, "t1").rounds = vec![round(1, 41, usage)];
    let fresh = fingerprint(&counted);
    task_mut(&mut counted, "t1").rounds[0].tool_calls = 97;
    task_mut(&mut counted, "t1").rounds[0].usage.output = 1_000_000;
    task_mut(&mut counted, "t1").rounds[0].last_event = 9_999;
    counted.orchestrator_usage = usage;
    counted.scout_usage = usage;
    // A history line on a task whose state did not change.
    event(task_mut(&mut counted, "t1"), at(12, 40), "tool call");
    assert_eq!(fingerprint(&counted), fresh);
    // `now` is not part of it: two digests at different times fingerprint the same run.
    assert_ne!(digest(&run, NOW)["now"], digest(&run, NOW + 60)["now"]);
    assert_eq!(fingerprint(&run), before);
}

/// Beyond the Interfaces (decision 16's read receipt): what a read itself changes —
/// the wake notes it drops, the approved holds it stops showing — never bumps the
/// revision, so an orchestrator is not answered at once after every read.
#[test]
fn a_read_does_not_change_the_fingerprint() {
    let mut run = fixed_run();
    run.orch.gate_holds.push(hold(
        "epic:a",
        HoldState::Approved,
        &["t1"],
        Some(at(12, 0)),
    ));
    let before = fingerprint(&run);
    assert_eq!(
        digest(&run, NOW)["gate"]["holds"].as_array().unwrap().len(),
        2
    );
    run.orch.digest_read_at = Some(at(12, 30));
    run.orch.orchestrator.as_mut().unwrap().notes.clear();
    assert_eq!(
        digest(&run, NOW)["gate"]["holds"].as_array().unwrap().len(),
        1
    );
    assert_eq!(fingerprint(&run), before);
}

/// One change to a run.
type Change = Box<dyn Fn(&mut Run)>;

#[test]
fn state_block_verdict_hold_scout_planner_note_message_and_edit_changes_do() {
    let base = fixed_run();
    let fp = fingerprint(&base);
    let cases: Vec<(&str, Change)> = vec![
        (
            "state",
            Box::new(|r| task_mut(r, "t1").state = TaskState::Check),
        ),
        (
            "block",
            Box::new(|r| block(task_mut(r, "t1"), BlockReason::Human, "needs a key")),
        ),
        (
            "block text",
            Box::new(|r| task_mut(r, "t2").block.as_mut().unwrap().text = "other?".into()),
        ),
        (
            "verdict",
            Box::new(|r| {
                let t = task_mut(r, "t2");
                t.reviews.push(review(2, Some(Verdict::Approve), &[]));
            }),
        ),
        (
            "hold",
            Box::new(|r| r.orch.gate_holds[0].state = HoldState::Approved),
        ),
        (
            "scout",
            Box::new(|r| r.orch.run_scouts[0].state = RunScoutState::Running),
        ),
        (
            "planner",
            Box::new(|r| r.orch.epics[0].phase = PlannerPhase::Failed { reason: "x".into() }),
        ),
        (
            "task note",
            Box::new(|r| {
                let n = note(at(12, 35), TaskNoteKind::Progress, "half done");
                task_mut(r, "t1").orch.worker_notes.push(n);
            }),
        ),
        (
            "message",
            Box::new(|r| {
                let m = message(at(12, 36), MessageKind::Change, "rebase", false);
                task_mut(r, "t2").orch.messages.push(m);
            }),
        ),
        (
            "delivery",
            Box::new(|r| task_mut(r, "t1").orch.messages[0].delivered = true),
        ),
        (
            "edit",
            Box::new(|r| {
                let mut e = r.plan_edits[0].clone();
                e.text = "cancel t3".into();
                r.plan_edits.push(e);
            }),
        ),
    ];
    for (name, change) in cases {
        let mut run = base.clone();
        change(&mut run);
        assert_ne!(
            fingerprint(&run),
            fp,
            "{name} did not change the fingerprint"
        );
    }
}

#[test]
fn note_change_bumps_the_revision_only_on_a_new_fingerprint() {
    let mut run = fixed_run();
    assert!(note_change(&mut run));
    assert_eq!(run.orch.digest_rev, 43);
    assert!(!note_change(&mut run));
    task_mut(&mut run, "t1").rounds = vec![round(1, 5, TokenUsage::default())];
    assert!(!note_change(&mut run));
    task_mut(&mut run, "t1").state = TaskState::Check;
    assert!(note_change(&mut run));
    assert_eq!(run.orch.digest_rev, 44);
}

/// 50 tasks with 120-character titles and 300-character history lines, half of them
/// blocked with 500-character texts, half merged, and 100 edits.
fn big_run() -> Run {
    let mut run = run_of(50);
    for (i, task) in run.tasks.iter_mut().enumerate() {
        task.spec.title = format!("{i:03}{}", "t".repeat(117));
        event(task, 1_000 + i as u64, &"h".repeat(300));
        if i % 2 == 0 {
            task.state = TaskState::Merged;
        } else {
            block(task, BlockReason::Question, &"q".repeat(500));
        }
    }
    for i in 0..100 {
        crate::run::edit_log::record(
            &mut run,
            &[proto::PlanEdit::CancelTask {
                task_id: format!("t{i}"),
            }],
            2_000 + i,
            &crate::run::orch::EditSource::User,
            crate::run::edit_log::EditOutcome::accepted(),
        );
    }
    run
}

#[test]
fn digest_is_capped_and_trims_in_order() {
    let run = big_run();
    assert!(
        crate::run::orch::json::size(&build(&run, NOW, false)) > DIGEST_MAX_BYTES,
        "the test run must start over the cap"
    );
    let d = digest(&run, NOW);
    assert!(
        crate::run::orch::json::size(&d) <= DIGEST_MAX_BYTES,
        "{}",
        crate::run::orch::json::size(&d)
    );
    let omitted = d["omitted_tasks"].as_u64().unwrap() as usize;
    assert!(omitted > 0);
    let kept: Vec<&str> = d["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(kept.len() + omitted, 50);
    // Finished tasks go first, oldest first: every blocked task stays, and the kept
    // merged tasks are the newest ones.
    for task in &run.tasks {
        let shown = kept.contains(&task.id());
        if task.state == TaskState::Blocked {
            assert!(shown, "{} was dropped before the finished tasks", task.id());
        }
    }
    let dropped: Vec<usize> = (0..50)
        .filter(|i| !kept.contains(&run.tasks[*i].id()))
        .collect();
    assert_eq!(dropped, (0..omitted).map(|i| i * 2).collect::<Vec<_>>());
    // Dropping finished tasks sufficed, so the later steps did not run.
    assert_eq!(d["edits"].as_array().unwrap().len(), EDITS_SHOWN);
    let text = d["tasks"][0]["block"]["text"].as_str().unwrap();
    assert_eq!(text.chars().count(), BLOCK_TEXT_MAX);
}

#[test]
fn past_the_finished_tasks_edits_notes_and_blocks_are_cut() {
    let mut run = big_run();
    for (i, task) in run.tasks.iter_mut().enumerate() {
        block(task, BlockReason::Question, &"q".repeat(500));
        for n in 0..3 {
            task.orch.worker_notes.push(note(
                3_000 + i as u64 * 3 + n,
                TaskNoteKind::Risk,
                &"r".repeat(400),
            ));
        }
    }
    let d = digest(&run, NOW);
    assert!(
        crate::run::orch::json::size(&d) <= DIGEST_MAX_BYTES,
        "{}",
        crate::run::orch::json::size(&d)
    );
    assert_eq!(d["edits"].as_array().unwrap().len(), 3);
    assert_eq!(d["task_notes"].as_array().unwrap().len(), 3);
    // Past the block cut, the texts are shortened again (to 120) before an unfinished
    // task is dropped (review fix I-3), so every task is still shown.
    assert_eq!(d["omitted_tasks"], 0);
    for task in d["tasks"].as_array().unwrap() {
        let text = task["block"]["text"].as_str().unwrap();
        assert!(text.chars().count() <= BLOCK_TEXT_TRIMMED + 1, "{text}"); // with `…`
    }
}

/// M9.6 review fix M-2: ten notes on `t3` at second T, then a `risk` note on `t0` in
/// the same second: the new note is the newest, so it is shown and the fingerprint
/// moves.
#[test]
fn a_new_note_in_the_same_second_moves_the_fingerprint() {
    let t = at(12, 30);
    let mut run = run_of(4);
    for i in 0..10 {
        let n = note(t, TaskNoteKind::Progress, &format!("step {i}"));
        assert!(crate::run::orch::add_worker_note(&mut run, "t3", n));
    }
    let before = fingerprint(&run);
    let risk = note(t, TaskNoteKind::Risk, "the schema is shared");
    assert!(crate::run::orch::add_worker_note(&mut run, "t0", risk));
    assert_ne!(fingerprint(&run), before);
    let d = digest(&run, NOW);
    assert_eq!(d["task_notes"][0]["task"], "t0", "{}", d["task_notes"]);
    assert_eq!(d["task_notes"][0]["text"], "the schema is shared");
    assert_eq!(d["task_notes"][1]["text"], "step 9");
}

#[test]
fn gate_states() {
    let gate = |run: &Run| digest(run, NOW)["gate"].clone();
    let mut run = fixed_run();
    run.state = RunState::Planning;
    assert_eq!(gate(&run)["state"], "planning");
    run.state = RunState::AwaitingApproval;
    assert_eq!(gate(&run)["state"], "awaiting_approval");
    assert_eq!(gate(&run)["at"], Value::Null);
    run.state = RunState::Running;
    assert_eq!(gate(&run)["state"], "approved");
    assert_eq!(gate(&run)["at"], "11:02");
    assert_eq!(digest(&run, NOW)["run"]["approved_by"], "user");
    // A fast-path run the orchestrator has not taken over.
    run.path = Some(RunPath::Fast);
    run.orch.orchestrator = None;
    assert_eq!(gate(&run)["state"], "none");
    // Holds: every one not approved, and an approved one decided since the last read.
    let mut run = fixed_run();
    run.orch.gate_holds = vec![
        hold("epic:a", HoldState::Approved, &["t1"], Some(at(12, 0))),
        hold("epic:b", HoldState::Rejected, &["t2"], Some(at(9, 0))),
        hold("promotion", HoldState::Drafting, &[], None),
    ];
    let ids = |run: &Run| -> Vec<String> {
        gate(run)["holds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(&run), ["epic:a", "epic:b", "promotion"]);
    run.orch.digest_read_at = Some(at(11, 0));
    assert_eq!(ids(&run), ["epic:a", "epic:b", "promotion"]);
    run.orch.digest_read_at = Some(at(12, 1));
    assert_eq!(ids(&run), ["epic:b", "promotion"]);
    assert_eq!(
        gate(&run)["holds"][0],
        json!({"id": "epic:b", "state": "rejected", "tasks": 1})
    );
}

#[test]
fn message_pause_counts_as_paused() {
    let mut run = fixed_run();
    block(
        task_mut(&mut run, "t1"),
        BlockReason::MessagePause,
        "asked to stop and wait: hold on",
    );
    let d = digest(&run, NOW);
    assert_eq!(d["counts"]["paused"], 1);
    assert_eq!(d["counts"]["blocked"], 1); // t2's question only
    assert_eq!(d["tasks"][1]["state"], "blocked");
    assert_eq!(d["tasks"][1]["block"]["reason"], "message_pause");
}

/// Carry-forward rule: a worker's or scout's text cannot open an anthrex line in what
/// the orchestrator reads.
#[test]
fn untrusted_text_stays_inside_its_json_string() {
    let mut run = fixed_run();
    block(task_mut(&mut run, "t2"), BlockReason::Question, FORGED);
    task_mut(&mut run, "t1").orch.worker_notes[0].text = FORGED.into();
    run.orch.run_scouts[0].question = FORGED.into();
    run.orch.epics[0].note = Some(FORGED.into());
    assert_contained(&digest(&run, NOW));
}

#[path = "digest_tests_trim.rs"]
mod trim;
