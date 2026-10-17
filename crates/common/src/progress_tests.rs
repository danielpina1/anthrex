use super::*;
use proto::{FullInfo, StageInfo};

fn task(id: &str, state: TaskState, stage: u16, round: u32, replaced_by: &[&str]) -> TaskInfo {
    let route = serde_json::json!({
        "runtime": "claude", "model": "", "strength": "standard", "effort": "medium",
    });
    let spend = serde_json::json!({ "tool_calls": 0, "secs": 0, "tokens": 0 });
    let mut value = serde_json::json!({
        "id": id, "title": id, "epic": null, "kind": "code", "size": "S", "hub": false,
        "test_mode": "tdd", "test_mode_reason": null, "notes": [], "owns": [], "deps": [],
        "implicit_deps": [], "priority": 0, "route": route, "review_route": null,
        "budget": { "tool_calls": 100, "minutes": 30 },
        "spent_session": spend, "spent_total": spend, "state": state, "block": null,
        "rung": 0, "failures": 0,
    });
    let rest = serde_json::json!({
        "bounces": { "done": 0, "proof": 0, "check": 0, "review": 0, "merge": 0 },
        "stalls": 0, "budget_exceeded": 0, "conflicts": 0, "branch": "", "worktree": "/tmp",
        "start_commit": null, "head": null, "test": null, "red": null, "done_signal": null,
        "rounds": [], "reviews": [], "last_check": null, "last_proof": null,
        "merge_commit": null, "merged_without_approval": null, "salvage_refs": [],
        "on_critical_path": false, "wave": 0,
        "stage": stage, "round": round, "replaced_by": replaced_by,
    });
    if let (Some(value), serde_json::Value::Object(rest)) = (value.as_object_mut(), rest) {
        value.extend(rest);
    }
    serde_json::from_value(value).expect("a minimal TaskInfo")
}

fn run(tasks: Vec<TaskInfo>, stages: usize) -> RunInfo {
    let mut run: RunInfo = serde_json::from_value(serde_json::json!({
        "run_id": "r-1", "goal": "g", "project": "/r", "root": "/r", "state": "running",
        "paused_from": null, "halted_reason": null, "approved_by": null,
        "base_branch": "main", "base_sha": "", "run_branch": "", "run_head": "",
        "base_moved": null, "revision": 1, "max_writers": 3, "max_readers": 3,
        "max_bounces": 2, "writers_busy": 0, "readers_busy": 0, "unverified": false,
        "worker_sandbox": true, "trusted_project": [], "rate_limits": {}, "tasks": [],
        "critical_path": [], "attention": [], "report_path": "/tmp/report.md",
        "outcome": null, "created_at": 0,
    }))
    .expect("a minimal RunInfo");
    run.tasks = tasks;
    run.stages = (1..=stages)
        .map(|n| StageInfo {
            actions: Vec::new(),
            n: n as u16,
            branch: String::new(),
            head: None,
            tasks: 0,
            merged: 0,
            full: FullInfo::default(),
            fix_tasks: Vec::new(),
            propagate_red: None,
            pr: None,
            round: 1,
        })
        .collect();
    run
}

#[test]
fn progress_counts_every_round_and_leaves_out_what_cannot_merge() {
    let tasks = vec![
        task("a", TaskState::Merged, 1, 1, &[]),
        task("b", TaskState::Working, 1, 1, &[]),
        task("c", TaskState::Cancelled, 1, 1, &[]),
        task("d", TaskState::Reported, 1, 1, &[]),
        // A replaced task is cancelled, and has a replacement.
        task("e", TaskState::Cancelled, 1, 1, &["h"]),
        task("f", TaskState::Merged, 2, 2, &[]),
        task("g", TaskState::MergeQueue, 2, 2, &[]),
        task("h", TaskState::Pending, 2, 2, &[]),
    ];
    let p = run_progress(&run(tasks, 0));
    assert_eq!((p.merged, p.total, p.cancelled), (2, 5, 2));
}

#[test]
fn no_tasks_is_zero_of_zero() {
    let p = run_progress(&run(Vec::new(), 0));
    assert_eq!(p, Progress::default());
    assert_eq!((p.merged, p.total, p.cancelled), (0, 0, 0));
}

#[test]
fn counts_is_the_one_predicate() {
    for state in [
        TaskState::Pending,
        TaskState::Queued,
        TaskState::Preparing,
        TaskState::Working,
        TaskState::Proof,
        TaskState::Check,
        TaskState::Review,
        TaskState::MergeQueue,
        TaskState::Merged,
        TaskState::Blocked,
    ] {
        assert!(counts(state, false), "{state:?}");
        assert!(!counts(state, true), "{state:?} replaced");
    }
    for state in [TaskState::Cancelled, TaskState::Reported] {
        assert!(!counts(state, false), "{state:?}");
        assert!(!counts(state, true), "{state:?} replaced");
    }
}

#[test]
fn tally_takes_any_iterator_of_tasks() {
    let tasks = [
        task("a", TaskState::Merged, 1, 1, &[]),
        task("b", TaskState::Blocked, 1, 1, &[]),
        task("c", TaskState::Cancelled, 1, 1, &[]),
    ];
    let p = tally(tasks.iter().filter(|t| t.stage == 1));
    assert_eq!((p.merged, p.total, p.cancelled), (1, 2, 1));
    assert_eq!(tally(std::iter::empty()), Progress::default());
}

#[test]
fn stage_count_mirrors_the_daemon() {
    let planned = |n: u16| vec![task("a", TaskState::Working, n, 1, &[])];
    // Stage records behind the plan: the plan's highest stage wins.
    assert_eq!(stage_count(&run(planned(3), 1)), 3);
    // Stage records ahead of the plan (a later round created stages): the records win.
    assert_eq!(stage_count(&run(planned(2), 4)), 4);
    // No records yet: the plan.
    assert_eq!(stage_count(&run(planned(2), 0)), 2);
    // Neither tasks nor records: one stage.
    assert_eq!(stage_count(&run(Vec::new(), 0)), 1);
    // A task with stage 0 never makes it 0.
    assert_eq!(stage_count(&run(planned(0), 0)), 1);
}
