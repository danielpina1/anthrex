//! Plan edits against state the engine changed: rung-2 routes and rung-3 sizes,
//! implicit dependencies, cancelled dependencies, live rounds, history (M8a.6 fix
//! round 1).

use proto::{Effort, Route, Runtime, Strength};

use super::*;
use crate::run::roster::pick_reviewer;
use crate::run::validate::combined_cycles;

fn task_mut<'a>(run: &'a mut Run, id: &str) -> &'a mut crate::run::model::Task {
    run.tasks
        .iter_mut()
        .find(|t| t.spec.id == id)
        .unwrap_or_else(|| panic!("no task {id}"))
}

fn check_mode(task_id: &str) -> PlanEdit {
    amend(
        task_id,
        Amend {
            test_mode: Some(TestMode::Check),
            test_mode_reason: Some("covered by the workspace check".to_string()),
            ..Amend::default()
        },
    )
}

/// `flat()` with t1 raised to `size` by rung 3 and blocked as mis-sized.
fn rung3(size: Size) -> Run {
    let mut run = flat();
    set_state(
        &mut run,
        "t1",
        TaskState::Blocked,
        Some(BlockReason::MisSized),
    );
    let t1 = task_mut(&mut run, "t1");
    t1.size = size;
    t1.rung = 3;
    t1.raised_size = Some(size);
    t1.notes
        .push("size raised by rung 3 (decision 38)".to_string());
    run
}

const L_ERROR: &str = "task t1: size: L tasks are never executed; split the task (rule 7.2.4)";

#[test]
fn amend_route_on_a_rung3_l_task_is_rejected() {
    let run = rung3(Size::L);
    let route = amend(
        "t1",
        Amend {
            route: Some(RouteSpec {
                effort: Some(Effort::High),
                ..RouteSpec::default()
            }),
            ..Amend::default()
        },
    );
    let reason = amend(
        "t1",
        Amend {
            test_mode_reason: Some("a reason".to_string()),
            ..Amend::default()
        },
    );
    assert_eq!(rejected(&run, vec![route]), vec![L_ERROR]);
    assert_eq!(rejected(&run, vec![check_mode("t1")]), vec![L_ERROR]);
    assert_eq!(rejected(&run, vec![reason]), vec![L_ERROR]);
    // An explicit smaller size does not lower an engine raise either.
    assert_eq!(
        rejected(&run, vec![amend_size("t1", Size::M)]),
        vec![L_ERROR]
    );
}

#[test]
fn a_rung3_raise_to_m_survives_a_test_mode_amend() {
    let run = rung3(Size::M);
    assert_eq!(task(&run, "t1").spec.size, Size::S);

    let (edited, _) = applied(&run, vec![check_mode("t1")]);
    let t1 = task(&edited, "t1");
    assert_eq!(t1.size, Size::M);
    assert_eq!(t1.test_mode, TestMode::Check);
    assert_eq!(t1.budget, edited.limits.budget_m);
    assert!(
        t1.notes
            .contains(&"size raised by rung 3 (decision 38)".to_string()),
        "the engine's note must stay: {:?}",
        t1.notes
    );

    let (edited, _) = applied(&run, vec![amend_size("t1", Size::S)]);
    assert_eq!(task(&edited, "t1").size, Size::M);
}

#[test]
fn an_escalated_route_survives_a_test_mode_amend() {
    let mut run = flat();
    set_state(&mut run, "t2", TaskState::Blocked, Some(BlockReason::Human));
    // Rung 2 at effort high moves to the peer runtime (decision 39).
    let escalated = Route {
        runtime: Runtime::Codex,
        model: String::new(),
        strength: Strength::Standard,
        effort: Effort::High,
    };
    let t2 = task_mut(&mut run, "t2");
    t2.route = escalated.clone();
    t2.rung = 2;

    let (edited, _) = applied(&run, vec![check_mode("t2")]);
    let t2 = task(&edited, "t2");
    assert_eq!(t2.route, escalated);
    assert_eq!(t2.test_mode, TestMode::Check);
    // The reviewer is picked for the route the task keeps: a Codex author gets a
    // Claude reviewer, where the planned Claude route would have had a Codex one.
    let level = t2.review_level.expect("reviewed");
    let for_kept = pick_reviewer(&edited.roster, &escalated, level);
    assert_eq!(for_kept.runtime, Runtime::Claude);
    assert_eq!(t2.review_route, Some(for_kept));

    // Naming the route replaces it.
    let named = amend(
        "t2",
        Amend {
            route: Some(RouteSpec {
                effort: Some(Effort::Medium),
                ..RouteSpec::default()
            }),
            ..Amend::default()
        },
    );
    let (edited, _) = applied(&run, vec![named]);
    let route = &task(&edited, "t2").route;
    assert_eq!(
        (route.model.as_str(), route.effort),
        ("claude-sonnet-5", Effort::Medium)
    );
}

/// t1 owns `crates/a/**`, t2 `crates/a/src/**`: at plan time t2 waits for t1.
fn overlapping() -> Run {
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", "[\"crates/a/**\"]", ""),
            task_toml("t2", "S", "[\"crates/a/src/**\"]", ""),
        ],
    ));
    assert_eq!(task(&run, "t2").implicit_deps, vec!["t1"]);
    run
}

#[test]
fn edits_recompute_implicit_deps() {
    let run = overlapping();
    let (edited, _) = applied(&run, vec![add_dep("t1", "t2")]);
    assert_eq!(task(&edited, "t1").spec.deps, vec!["t2"]);
    // t1 now declares t2, so t2 no longer waits for t1: no combined cycle.
    assert_eq!(task(&edited, "t2").implicit_deps, Vec::<String>::new());
    assert_eq!(task(&edited, "t1").implicit_deps, Vec::<String>::new());
    assert_eq!(combined_cycles(&edited.tasks), vec![]);
}

#[test]
fn an_added_task_waits_for_an_overlapping_earlier_task() {
    let run = overlapping();
    let add = PlanEdit::AddTask {
        task: spec(&task_toml("t9", "S", "[\"crates/a/src/x.rs\"]", "")),
    };
    let (edited, _) = applied(&run, vec![add]);
    assert_eq!(task(&edited, "t9").implicit_deps, vec!["t1", "t2"]);
}

#[test]
fn a_split_child_before_a_working_task_waits_for_it() {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[
            one("t1", ""),
            one("t2", ""),
            task_toml("t3", "S", "[\"crates/c/src/lib.rs\"]", ""),
        ],
    ));
    set_state(&mut run, "t3", TaskState::Working, None);
    let split = PlanEdit::SplitTask {
        task_id: "t2".to_string(),
        into: vec![spec(&task_toml("t2a", "S", "[\"crates/c/**\"]", ""))],
    };
    let (edited, _) = applied(&run, vec![split]);
    assert_eq!(ids(&edited), vec!["t1", "t2", "t2a", "t3"]);
    assert_eq!(task(&edited, "t2a").implicit_deps, vec!["t3"]);
    assert_eq!(task(&edited, "t3").implicit_deps, Vec::<String>::new());
}

#[test]
fn a_dep_cancelled_task_stays_editable() {
    let (run, _) = applied(&chain(), vec![cancel("t1")]);
    assert_eq!(
        task(&run, "t2").block.as_ref().map(|b| b.reason),
        Some(BlockReason::DepCancelled)
    );
    let priority = amend(
        "t2",
        Amend {
            priority: Some(3),
            ..Amend::default()
        },
    );
    let (edited, _) = applied(&run, vec![priority]);
    assert_eq!(task(&edited, "t2").spec.priority, 3);

    let replacement = PlanEdit::AddTask {
        task: spec(&one("t1b", "")),
    };
    let (edited, _) = applied(&run, vec![replacement, add_dep("t2", "t1b")]);
    assert_eq!(task(&edited, "t2").spec.deps, vec!["t1", "t1b"]);

    // Adding a dependency on the cancelled task itself is still refused.
    assert_eq!(
        rejected(&run, vec![add_dep("t4", "t1")]),
        vec!["task t4: deps: t1 is cancelled"]
    );
}

#[test]
fn cancel_is_live_in_every_live_state() {
    for state in [
        TaskState::Preparing,
        TaskState::Working,
        TaskState::Proof,
        TaskState::Check,
        TaskState::Review,
        TaskState::MergeQueue,
    ] {
        let mut run = flat();
        set_state(&mut run, "t1", state, None);
        let (_, consequences) = applied(&run, vec![cancel("t1")]);
        assert_eq!(
            consequences,
            vec![EditConsequence::CancelLive {
                task_id: "t1".to_string()
            }],
            "{state:?}"
        );
    }
    for (state, block) in [
        (TaskState::Pending, None),
        (TaskState::Queued, None),
        (TaskState::Blocked, Some(BlockReason::Human)),
    ] {
        let mut run = flat();
        set_state(&mut run, "t1", state, block);
        let (edited, consequences) = applied(&run, vec![cancel("t1")]);
        assert_eq!(consequences, vec![], "{state:?}");
        assert_eq!(task(&edited, "t1").block, None, "{state:?}");
    }
}

#[test]
fn amend_delivers_only_to_an_open_worker_round() {
    let mut run = flat();
    set_state(
        &mut run,
        "t1",
        TaskState::Blocked,
        Some(BlockReason::Question),
    );
    task_mut(&mut run, "t1").rounds.push(open_round());
    set_state(&mut run, "t2", TaskState::Review, None);
    let mut reviewer = open_round();
    reviewer.role = AgentRole::Reviewer;
    task_mut(&mut run, "t2").rounds.push(reviewer);

    let (edited, consequences) = applied(
        &run,
        vec![
            amend_brief("t1", "Worker brief", &["w"]),
            amend_brief("t2", "Reviewed brief", &["r"]),
        ],
    );
    assert_eq!(
        consequences,
        vec![EditConsequence::Deliver {
            task_id: "t1".to_string(),
            text: amend_message(task(&edited, "t1")),
        }]
    );
}

#[test]
fn dependencies_are_never_duplicated() {
    let (edited, _) = applied(&chain(), vec![add_dep("t3", "t2")]);
    assert_eq!(task(&edited, "t3").spec.deps, vec!["t2"]);

    let split = PlanEdit::SplitTask {
        task_id: "t2".to_string(),
        into: vec![spec(&one("t2a", "")), spec(&one("t2b", ""))],
    };
    let (edited, _) = applied(&chain(), vec![add_dep("t3", "t2a"), split]);
    assert_eq!(task(&edited, "t3").spec.deps, vec!["t2a", "t2b"]);
}

#[test]
fn edits_write_task_history() {
    let mut run = chain();
    set_state(&mut run, "t4", TaskState::Working, None);
    let split = PlanEdit::SplitTask {
        task_id: "t3".to_string(),
        into: vec![spec(&one("t3a", ""))],
    };
    let (edited, _) = applied(
        &run,
        vec![
            PlanEdit::AddTask {
                task: spec(&one("t9", "")),
            },
            amend_brief("t1", "New", &["n"]),
            add_dep("t2", "t9"),
            answer("t4", "yes"),
            split,
        ],
    );
    let last = |id: &str| {
        let event = task(&edited, id).history.last().expect("a history entry");
        (event.at, event.text.clone())
    };
    assert_eq!(last("t9"), (5_000, "added by a plan edit".to_string()));
    assert_eq!(
        last("t1"),
        (5_000, "amended: brief, acceptance".to_string())
    );
    assert_eq!(last("t2"), (5_000, "dependency on t9 added".to_string()));
    assert_eq!(last("t4"), (5_000, "answered".to_string()));
    assert_eq!(last("t3"), (5_000, "split into t3a".to_string()));
    assert_eq!(last("t3a"), (5_000, "split from t3".to_string()));
}

#[test]
fn a_cancel_records_the_dependents_earlier_block() {
    let mut run = chain();
    set_state(&mut run, "t2", TaskState::Blocked, Some(BlockReason::Human));
    let (edited, _) = applied(&run, vec![cancel("t1")]);
    let t2 = task(&edited, "t2");
    assert_eq!(
        t2.block.as_ref().map(|b| b.reason),
        Some(BlockReason::DepCancelled)
    );
    assert_eq!(
        t2.history.last().map(|e| e.text.as_str()),
        Some("blocked: dependency t1 was cancelled (was blocked(human): fixture block on t2)")
    );
}

#[test]
fn an_empty_amend_is_refused() {
    assert_eq!(
        rejected(&flat(), vec![amend("t1", Amend::default())]),
        vec!["task t1: amend_task: nothing to amend"]
    );
}

// Fix round 2.

#[test]
fn a_rung3_l_task_cannot_be_lowered_by_two_amends() {
    let run = rung3(Size::L);
    // In one batch: the first amend matches the raise, the second tries to lower it.
    assert_eq!(
        rejected(
            &run,
            vec![amend_size("t1", Size::L), amend_size("t1", Size::S)]
        ),
        vec![L_ERROR]
    );
    // Across two batches: the first is refused by the L rule and changes nothing.
    assert_eq!(
        rejected(&run, vec![amend_size("t1", Size::L)]),
        vec![L_ERROR]
    );
    assert_eq!(
        rejected(&run, vec![amend_size("t1", Size::S)]),
        vec![L_ERROR]
    );
    // A task whose spec already says L (as an earlier amend would leave it) is no
    // different: the recorded raise is the floor, not the spec.
    let mut spec_l = rung3(Size::L);
    task_mut(&mut spec_l, "t1").spec.size = Size::L;
    assert_eq!(
        rejected(&spec_l, vec![amend_size("t1", Size::S)]),
        vec![L_ERROR]
    );
    assert_eq!(task(&spec_l, "t1").size, Size::L);
}

#[test]
fn a_rung3_m_task_cannot_be_lowered_by_two_amends() {
    let run = rung3(Size::M);
    let (edited, _) = applied(
        &run,
        vec![amend_size("t1", Size::M), amend_size("t1", Size::S)],
    );
    assert_eq!(task(&edited, "t1").size, Size::M);

    let (first, _) = applied(&run, vec![amend_size("t1", Size::M)]);
    assert_eq!(task(&first, "t1").spec.size, Size::M);
    let (second, _) = applied(&first, vec![amend_size("t1", Size::S)]);
    assert_eq!(task(&second, "t1").size, Size::M);
    assert_eq!(task(&second, "t1").spec.size, Size::S);
}

#[test]
fn a_split_child_returns_an_overlapping_queued_task_to_pending() {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[
            one("t1", ""),
            one("t2", ""),
            task_toml("t3", "S", "[\"crates/c/src/lib.rs\"]", ""),
            one("t4", ""),
        ],
    ));
    set_state(&mut run, "t3", TaskState::Queued, None);
    set_state(&mut run, "t4", TaskState::Queued, None);
    let split = PlanEdit::SplitTask {
        task_id: "t2".to_string(),
        into: vec![spec(&task_toml("t2a", "S", "[\"crates/c/**\"]", ""))],
    };
    let (edited, _) = applied(&run, vec![split]);
    assert_eq!(task(&edited, "t3").implicit_deps, vec!["t2a"]);
    assert_eq!(task(&edited, "t3").state, TaskState::Pending);
    // A queued task the edit does not make wait stays queued.
    assert_eq!(task(&edited, "t4").state, TaskState::Queued);
}

#[test]
fn new_tasks_on_a_cancelled_dependency_are_rejected() {
    let mut run = flat();
    set_state(&mut run, "t2", TaskState::Cancelled, None);
    let add = PlanEdit::AddTask {
        task: spec(&one("t9", "deps = [\"t2\"]")),
    };
    assert_eq!(
        rejected(&run, vec![add]),
        vec!["task t9: deps: t2 is cancelled"]
    );
    let split = PlanEdit::SplitTask {
        task_id: "t3".to_string(),
        into: vec![spec(&one("t3a", "deps = [\"t2\"]"))],
    };
    assert_eq!(
        rejected(&run, vec![split]),
        vec!["task t3a: deps: t2 is cancelled"]
    );
}

/// t1 and t3 own overlapping globs under `crates/c`; t2 is split into a child that
/// overlaps both and lands before t3.
fn blocked_later_task(start_commit: Option<&str>) -> Run {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[
            one("t1", ""),
            one("t2", ""),
            task_toml("t3", "S", "[\"crates/c/src/lib.rs\"]", ""),
        ],
    ));
    set_state(&mut run, "t3", TaskState::Blocked, Some(BlockReason::Human));
    task_mut(&mut run, "t3").start_commit = start_commit.map(str::to_string);
    let split = PlanEdit::SplitTask {
        task_id: "t2".to_string(),
        into: vec![spec(&task_toml("t2a", "S", "[\"crates/c/**\"]", ""))],
    };
    applied(&run, vec![split]).0
}

#[test]
fn a_blocked_task_with_a_start_commit_counts_as_started() {
    // Started: the new child waits for it, whatever the plan order.
    let edited = blocked_later_task(Some(&"c".repeat(40)));
    assert_eq!(task(&edited, "t2a").implicit_deps, vec!["t3"]);
    assert_eq!(task(&edited, "t3").implicit_deps, Vec::<String>::new());
    // Not started: plan order decides, so the later t3 waits for the child.
    let edited = blocked_later_task(None);
    assert_eq!(task(&edited, "t2a").implicit_deps, Vec::<String>::new());
    assert_eq!(task(&edited, "t3").implicit_deps, vec!["t2a"]);
}

#[test]
fn two_started_tasks_never_wait_for_each_other() {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", "[\"crates/c/**\"]", ""),
            one("t2", ""),
            task_toml("t3", "S", "[\"crates/c/src/lib.rs\"]", ""),
        ],
    ));
    set_state(&mut run, "t1", TaskState::Working, None);
    set_state(&mut run, "t3", TaskState::Review, None);
    let (edited, _) = applied(&run, vec![answer("t1", "go on")]);
    assert_eq!(task(&edited, "t1").implicit_deps, Vec::<String>::new());
    assert_eq!(task(&edited, "t3").implicit_deps, Vec::<String>::new());
}
