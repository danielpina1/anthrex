//! M8b.16: phase times (decision 31), task and run records (decision 33) and routing
//! decisions (decision 33a), on the pure model.

use proto::{
    AgentRole, BlockReason, DiffStats, DoneSignal, Effort, GateCounts, GateTally, HISTORY_VERSION,
    PhaseSecs, RunPath, RunState, Runtime, Severity, SeverityTally, SizeCheckInfo, Strength,
    TaskOutcome, TaskRecord, TaskState, TokenUsage, Verdict,
};

use super::{due, run_outcome, run_record, run_record_due, task_record, task_record_id};
use crate::run::contract::generated_files_message;
use crate::run::model::{DoneClaim, ProofRecord, ReviewLevel, Run, SizeCheckState};
use crate::run::phases::set_state;
use crate::run::roster::{escalate, pick_reviewer};
use crate::run::routing::{record_reviewer, record_worker};

#[path = "history_tests_fixtures.rs"]
mod fixtures;
use fixtures::*;
#[path = "history_tests_routing.rs"]
mod routing;

#[test]
fn set_state_accumulates_phase_times() {
    let mut run = run_of(&["t1"]);
    let task = &mut run.tasks[0];
    assert_eq!(task.state, TaskState::Pending);
    set_state(task, TaskState::Queued, 100);
    set_state(task, TaskState::Pending, 103);
    // The 47 s pending count nowhere.
    set_state(task, TaskState::Queued, 150);
    set_state(task, TaskState::Preparing, 157);
    set_state(task, TaskState::Working, 162);
    set_state(task, TaskState::Proof, 262);
    set_state(task, TaskState::Check, 269);
    set_state(task, TaskState::Review, 282);
    set_state(task, TaskState::MergeQueue, 312);
    set_state(task, TaskState::Merged, 315);
    let want = PhaseSecs {
        queued: 10,
        preparing: 5,
        working: 100,
        proof: 7,
        check: 13,
        review: 30,
        merge: 3,
        ..PhaseSecs::default()
    };
    assert_eq!(task.phases, want);
    assert_eq!((task.state, task.phase_since), (TaskState::Merged, 315));
    // A state set again changes nothing; a finished state counts nowhere.
    set_state(task, TaskState::Merged, 400);
    assert_eq!((task.phases, task.phase_since), (want, 315));

    // A task from before milestone 8b has no start for its open state.
    let task = &mut run.tasks[0];
    task.state = TaskState::Working;
    task.phase_since = 0;
    task.phases = PhaseSecs::default();
    set_state(task, TaskState::Blocked, 5_000);
    set_state(task, TaskState::Working, 5_060);
    assert_eq!(
        task.phases,
        PhaseSecs {
            blocked: 60,
            ..PhaseSecs::default()
        }
    );
}

#[test]
fn max_rung_is_tracked() {
    let mut run = run_of(&["t1"]);
    let task = &mut run.tasks[0];
    task.rung = 2;
    set_state(task, TaskState::Working, 10);
    assert_eq!(task.max_rung, 2);
    task.rung = 1;
    set_state(task, TaskState::Check, 20);
    assert_eq!(task.max_rung, 2);
    task.rung = 3;
    set_state(task, TaskState::Blocked, 30);
    assert_eq!(task.max_rung, 3);
}

#[test]
fn task_record_from_a_merged_task() {
    let mut run = run_of(&["t1"]);
    run.path = Some(RunPath::Fast);
    let size_check = SizeCheckInfo {
        engine: proto::Size::S,
        decided: Some(proto::Size::M),
        agreed: false,
        reason: "two modules".into(),
        source: proto::DeciderSource::Decider,
    };
    let task = &mut run.tasks[0];
    task.size = proto::Size::M;
    task.state = TaskState::Merged;
    task.size_check = Some(SizeCheckState::Done(size_check.clone()));
    task.rounds = vec![
        round(AgentRole::Worker, 1, 12, usage(10)),
        round(AgentRole::Reviewer, 1, 3, usage(100)),
        round(AgentRole::Worker, 2, 8, usage(20)),
        round(AgentRole::Reviewer, 2, 2, usage(200)),
    ];
    task.reviews = vec![
        review(1, Verdict::Changes, vec![finding(Severity::Important)]),
        review(2, Verdict::Approve, vec![finding(Severity::Minor)]),
    ];
    task.checks = vec![check(false, false), check(true, false), check(true, true)];
    task.proofs = vec![ProofRecord {
        at: 1_100,
        test: "t".into(),
        red: "r".into(),
        head: "h".into(),
        red_failed: true,
        head_passed: true,
        matched: true,
        red_tail: String::new(),
        head_tail: String::new(),
    }];
    task.failure_log = vec![
        generated_files_message(&["Cargo.lock".to_string()]),
        "the check failed".into(),
    ];
    task.bounces = GateCounts {
        done: 1,
        check: 1,
        review: 1,
        ..GateCounts::default()
    };
    (
        task.failures,
        task.stalls,
        task.budget_exceeded,
        task.conflicts,
    ) = (3, 1, 0, 1);
    (task.rung, task.max_rung, task.session) = (1, 2, 2);
    task.done = Some(DoneClaim {
        summary: "done".into(),
        test: None,
        red: None,
        signal: DoneSignal::TurnEndFallback,
    });
    task.merge_commit = Some("e".repeat(40));
    task.decider_usage = usage(1_000);
    task.diff = Some(DiffStats {
        files: 2,
        hunks: 3,
        added: 30,
        removed: 4,
    });
    task.phases = PhaseSecs {
        queued: 10,
        working: 100,
        proof: 7,
        check: 13,
        review: 30,
        merge: 3,
        ..PhaseSecs::default()
    };
    task.phase_since = 900;
    let task = run.tasks[0].clone();
    let record = task_record(&run, &task, TaskOutcome::Merged, 1_000);
    let reviewer = route(Runtime::Codex, "", Strength::Standard, Effort::Low);
    let want = TaskRecord {
        v: HISTORY_VERSION,
        record_id: format!("{}/t1", run.id),
        at: 1_000,
        run_id: run.id.clone(),
        task_id: "t1".into(),
        path: Some(RunPath::Fast),
        kind: proto::TaskKind::Code,
        hub: false,
        test_mode: task.test_mode,
        planned_size: proto::Size::S,
        final_size: proto::Size::M,
        size_check: Some(size_check),
        route: task.route.clone(),
        review_routes: vec![reviewer.clone(), reviewer],
        routing_decisions: Vec::new(),
        outcome: TaskOutcome::Merged,
        block: None,
        diff: task.diff,
        tool_calls: 20,
        worker_usage: TokenUsage {
            input: 30,
            output: 32,
            cache_read: 34,
            cache_write: 36,
        },
        reviewer_usage: TokenUsage {
            input: 300,
            output: 302,
            cache_read: 304,
            cache_write: 306,
        },
        decider_usage: usage(1_000),
        phases: task.phases,
        wall_secs: 163,
        gates: GateTally {
            proofs: 1,
            proofs_failed: 0,
            checks: 2,
            checks_failed: 1,
            review_rounds: 2,
            reviews_rejected: 1,
            candidates_red: 0,
            generated_bounces: 1,
        },
        severities: SeverityTally {
            critical: 0,
            important: 1,
            minor: 1,
        },
        bounces: task.bounces,
        failures: 3,
        stalls: 1,
        budget_exceeded: 0,
        conflicts: 1,
        max_rung: 2,
        sessions: 2,
        done_signal: Some(DoneSignal::TurnEndFallback),
        merge_commit: Some("e".repeat(40)),
    };
    assert_eq!(record, want);
    assert_eq!(task_record_id(&run.id, "t1"), want.record_id);
}

#[test]
fn routing_history_keeps_choice_time_candidates() {
    let mut run = run_of(&["t1"]);
    run.profile_languages = vec!["rust".into()];
    let sonnet = route(
        Runtime::Claude,
        "claude-sonnet-5",
        Strength::Standard,
        Effort::Medium,
    );
    assert_eq!(run.tasks[0].route, sonnet);

    // The first worker: the class default on the task's runtime and strength.
    run.tasks[0].session = 1;
    record_worker(&mut run, 0, 100);
    // A repeated start of the same session adds nothing.
    record_worker(&mut run, 0, 101);
    // A fresh escalated worker (rung 2): escalated from the route it had.
    let up = escalate(&run.roster, &sonnet);
    let task = &mut run.tasks[0];
    task.escalated_from = Some(std::mem::replace(&mut task.route, up.clone()));
    task.session = 2;
    record_worker(&mut run, 0, 200);
    assert_eq!(run.tasks[0].escalated_from, None);
    // A fresh session that is no escalation (a lost resume) chose nothing new.
    run.tasks[0].session = 3;
    record_worker(&mut run, 0, 250);
    // Two review rounds against the escalated author.
    for (round, level, now) in [
        (1, ReviewLevel::Medium, 300),
        (2, ReviewLevel::Frontier, 400),
    ] {
        let chosen = pick_reviewer(&run.roster, &up, level);
        record_reviewer(&mut run, 0, (&up, level), &chosen, round, now);
    }
    // A review round launched again (a restart) is the same decision.
    let chosen = pick_reviewer(&run.roster, &up, ReviewLevel::Frontier);
    record_reviewer(&mut run, 0, (&up, ReviewLevel::Frontier), &chosen, 2, 450);
    let decisions = run.tasks[0].routing_decisions.clone();
    let shape: Vec<_> = decisions
        .iter()
        .map(|d| {
            let who = (d.seq, d.at, d.role, d.session, d.round, d.lane.clone());
            let why = (
                d.trigger.as_str(),
                d.source.as_str(),
                d.policy_version.as_str(),
            );
            (who, why, d.selected_index, d.pick_policy.clone())
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            (
                (1, 100, AgentRole::Worker, 1, None, None),
                ("initial", "class_default", "m8a-worker-v1"),
                1,
                None
            ),
            (
                (2, 200, AgentRole::Worker, 2, None, None),
                ("escalation", "escalation_policy", "m8a-escalate-v1"),
                0,
                None
            ),
            (
                (3, 300, AgentRole::Reviewer, 1, Some(1), None),
                ("review", "review_policy", "m8a-review-v1"),
                0,
                None
            ),
            (
                (4, 400, AgentRole::Reviewer, 2, Some(2), None),
                ("review", "review_policy", "m8a-review-v1"),
                0,
                None
            ),
        ]
    );
    decisions.iter().for_each(assert_selected);
    let (claude, codex) = ("claude:", "codex:");
    let medium_std = s("strength fast, the task needs standard");
    assert_eq!(
        candidates(&decisions[0]),
        vec![
            (
                format!("{claude}claude-haiku-4-5"),
                Effort::Medium,
                medium_std
            ),
            (format!("{claude}claude-sonnet-5"), Effort::Medium, None),
            (
                format!("{claude}claude-opus-5-5"),
                Effort::Medium,
                s("strength frontier, the task needs standard")
            ),
            (
                codex.to_string(),
                Effort::Medium,
                s("runtime codex, the task runs on claude")
            ),
        ]
    );
    let after = s("ranked after the selected route");
    assert_eq!(
        candidates(&decisions[1]),
        vec![
            (format!("{claude}claude-sonnet-5"), Effort::High, None),
            (codex.to_string(), Effort::High, after.clone()),
            (
                format!("{claude}claude-opus-5-5"),
                Effort::High,
                after.clone()
            ),
            (
                format!("{claude}claude-sonnet-5"),
                Effort::Medium,
                after.clone()
            ),
            (
                format!("{claude}claude-haiku-4-5"),
                Effort::High,
                s("not an escalation step from claude-sonnet-5")
            ),
        ]
    );
    assert_eq!(decisions[1].chosen, up);
    assert_eq!(
        candidates(&decisions[2]),
        vec![
            (codex.to_string(), Effort::Medium, None),
            (
                format!("{claude}claude-opus-5-5"),
                Effort::Medium,
                after.clone()
            ),
            (
                format!("{claude}claude-sonnet-5"),
                Effort::Medium,
                s("the author's own model")
            ),
            (
                format!("{claude}claude-haiku-4-5"),
                Effort::Medium,
                s("below the required standard strength")
            ),
        ]
    );
    let below = s("below the required frontier strength");
    assert_eq!(
        candidates(&decisions[3]),
        vec![
            (format!("{claude}claude-opus-5-5"), Effort::High, None),
            (
                format!("{claude}claude-sonnet-5"),
                Effort::High,
                s("the author's own model")
            ),
            (
                format!("{claude}claude-haiku-4-5"),
                Effort::High,
                below.clone()
            ),
            (codex.to_string(), Effort::High, below),
        ]
    );
    for d in &decisions {
        assert_eq!(d.input.title, "Title t1");
        assert_eq!(d.input.brief, "Brief t1");
        assert_eq!(d.input.acceptance, vec!["Accept t1".to_string()]);
        assert_eq!(d.input.owns, vec!["crates/t1/**".to_string()]);
        assert_eq!(d.input.size, proto::Size::S);
        assert_eq!(d.input.languages, vec!["rust".to_string()]);
        assert!(!d.input.hub && !d.input.interface_change);
    }

    // Amending the brief and the size, and changing the roster, rewrites none of it,
    // and the task's record copies them in order.
    let task = &mut run.tasks[0];
    task.spec.brief = "Something else".into();
    task.size = proto::Size::M;
    run.roster.retain(|e| e.runtime == Runtime::Claude);
    run.profile_languages.clear();
    assert_eq!(run.tasks[0].routing_decisions, decisions);
    let task = run.tasks[0].clone();
    let record = task_record(&run, &task, TaskOutcome::Unfinished, 500);
    assert_eq!(record.routing_decisions, decisions);
}

/// An explicit model outside the roster is appended to the pool and selected.
#[test]
fn an_explicit_route_outside_the_roster_is_appended() {
    let mut run = run_of(&["t1"]);
    let task = &mut run.tasks[0];
    task.spec.route.model = Some("claude-custom".into());
    task.route.model = "claude-custom".into();
    task.session = 1;
    record_worker(&mut run, 0, 10);
    let d = &run.tasks[0].routing_decisions[0];
    assert_eq!(d.source, "explicit_task");
    assert_eq!(d.selected_index as usize, d.candidates.len() - 1);
    assert_eq!(d.candidates.len(), run.roster.len() + 1);
    assert!(d.candidates[..run.roster.len()].iter().all(|c| {
        c.skipped_reason.as_deref() == Some("the task names the model claude-custom")
            || c.skipped_reason.as_deref() == Some("runtime codex, the task runs on claude")
    }));
    assert_selected(d);
}

#[test]
fn old_task_record_has_no_routing_decisions() {
    let run = run_of(&["t1"]);
    let record = task_record(&run, &run.tasks[0], TaskOutcome::Unfinished, 10);
    let mut v = serde_json::to_value(proto::HistoryLine::Task(record)).unwrap();
    v.as_object_mut().unwrap().remove("routing_decisions");
    let line: proto::HistoryLine = serde_json::from_str(&v.to_string()).unwrap();
    let proto::HistoryLine::Task(old) = line else {
        panic!("a task line");
    };
    assert!(old.routing_decisions.is_empty());
    // And a `run.json` task from before decision 33a loads with none.
    let mut task = serde_json::to_value(&run.tasks[0]).unwrap();
    for key in [
        "routing_decisions",
        "escalated_from",
        "phases",
        "phase_since",
        "diff",
    ] {
        task.as_object_mut().unwrap().remove(key);
    }
    let task: crate::run::model::Task = serde_json::from_value(task).unwrap();
    assert!(task.routing_decisions.is_empty() && task.escalated_from.is_none());
    // A whole `run.json` from before milestone 8b.16 loads, history flags unset.
    let mut old = serde_json::to_value(&run).unwrap();
    let object = old.as_object_mut().unwrap();
    object.remove("run_record_written");
    object.remove("profile_languages");
    object.remove("history");
    for task in object["tasks"].as_array_mut().unwrap() {
        for key in [
            "routing_decisions",
            "max_rung",
            "history_written",
            "phase_since",
        ] {
            task.as_object_mut().unwrap().remove(key);
        }
    }
    let loaded: Run = serde_json::from_value(old).unwrap();
    assert!(!loaded.run_record_written && loaded.profile_languages.is_empty());
    assert!(!loaded.history, "an old run writes no history");
    assert!(!loaded.tasks[0].history_written);
}

#[test]
fn unfinished_tasks_get_records_when_the_run_ends() {
    let mut run = run_of(&["t1", "t2", "t3", "t4", "t5", "t6"]);
    let states = [
        TaskState::Merged,
        TaskState::Cancelled,
        TaskState::Blocked,
        TaskState::Working,
        TaskState::Pending,
        TaskState::Merged,
    ];
    for (task, state) in run.tasks.iter_mut().zip(states) {
        task.state = state;
    }
    run.tasks[5].history_written = true;
    run.tasks[2].block = Some(proto::BlockInfo {
        reason: BlockReason::Question,
        text: "?".into(),
    });
    run.state = RunState::Running;
    assert_eq!(run_outcome(&run), None);
    let ids = |run: &Run| -> Vec<(String, TaskOutcome)> {
        due(run)
            .into_iter()
            .map(|(i, o)| (run.tasks[i].id().to_string(), o))
            .collect()
    };
    // While the run runs, only a merged or cancelled task is due.
    assert_eq!(
        ids(&run),
        vec![
            ("t1".into(), TaskOutcome::Merged),
            ("t2".into(), TaskOutcome::Cancelled)
        ]
    );
    run.tasks[0].merged_without_approval = Some("the user".into());
    for (state, outcome) in [
        (RunState::Accepted, "accepted"),
        (RunState::Discarded, "discarded"),
        (RunState::Failed, "failed"),
    ] {
        run.state = state;
        assert_eq!(run_outcome(&run), Some(outcome));
        assert_eq!(
            ids(&run),
            vec![
                ("t1".into(), TaskOutcome::MergedWithoutApproval),
                ("t2".into(), TaskOutcome::Cancelled),
                ("t3".into(), TaskOutcome::Blocked),
                ("t4".into(), TaskOutcome::Unfinished),
                ("t5".into(), TaskOutcome::Unfinished),
            ]
        );
        assert_eq!(run_record_due(&run), None, "tasks come first");
    }
    for task in &mut run.tasks {
        task.history_written = true;
    }
    assert_eq!(run_record_due(&run), Some("failed"));
    run.run_record_written = true;
    assert_eq!(run_record_due(&run), None);
    // A complete run with an unfinished task has ended too; one whose tasks all
    // finished waits for its accept or discard.
    run.state = RunState::Complete;
    assert_eq!(run_outcome(&run), Some("complete"));
    for task in &mut run.tasks {
        task.state = TaskState::Merged;
    }
    assert_eq!(run_outcome(&run), None);
    // A blocked record carries its block reason.
    let record = task_record(&run, &run.tasks[2], TaskOutcome::Blocked, 9);
    assert_eq!(record.block, Some(BlockReason::Question));
}

#[test]
fn run_record_fields() {
    let mut run = run_of(&["t1", "t2"]);
    run.goal = "é".repeat(250);
    run.path = Some(RunPath::Fast);
    run.profile_source = Some(proto::ProfileSource::Stored);
    run.decider_calls = 4;
    let record = run_record(&run, "accepted", 77);
    assert_eq!(record.v, HISTORY_VERSION);
    assert_eq!(record.record_id, run.id);
    assert_eq!(record.run_id, run.id);
    assert_eq!(record.at, 77);
    assert_eq!(record.goal, "é".repeat(200));
    assert_eq!(record.path, Some(RunPath::Fast));
    assert_eq!(record.triage, None);
    assert_eq!(record.profile_source, Some(proto::ProfileSource::Stored));
    assert_eq!(record.outcome, "accepted");
    assert_eq!(record.base_branch, "main");
    assert_eq!(record.accepted_commit, None, "the driver fills it");
    assert_eq!(record.tasks, 2);
    assert_eq!(record.usage.map(|u| u.decider_calls), Some(4));
}

#[test]
fn a_run_restored_from_m8a_writes_no_history() {
    let mut run = run_of(&["t1"]);
    run.repo_dir = std::path::PathBuf::new();
    run.state = RunState::Accepted;
    run.tasks[0].state = TaskState::Merged;
    assert!(due(&run).is_empty());
    assert_eq!(run_record_due(&run), None);
}
