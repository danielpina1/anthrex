//! Plan edits, decision 13. Every test goes through `apply_edits`.

use proto::{
    AgentRole, BlockInfo, BlockReason, Effort, PlanEdit, PlanTask, RouteSpec, Size, TaskState,
    TestMode,
};

use super::*;
use crate::run::contract::{amend_message, answer_message};
use crate::run::model::{AgentRound, Run};
use crate::run::plan::parse_plan;
use crate::run::test_support::*;

/// One S task owning `crates/<id>/src/lib.rs`: one module, inside `source`, not hub.
fn one(id: &str, extra: &str) -> String {
    task_toml(id, "S", &format!("[\"crates/{id}/src/lib.rs\"]"), extra)
}

/// t1; t2 depends on t1; t3 depends on t2; t4 stands alone.
fn chain() -> Run {
    run_ok(&plan_with(
        PROFILE,
        &[
            one("t1", ""),
            one("t2", "deps = [\"t1\"]"),
            one("t3", "deps = [\"t2\"]"),
            one("t4", ""),
        ],
    ))
}

/// Four independent tasks.
fn flat() -> Run {
    run_ok(&plan_with(
        PROFILE,
        &[one("t1", ""), one("t2", ""), one("t3", ""), one("t4", "")],
    ))
}

/// A task spec parsed from one `[[task]]` table, exactly as a plan file would give it.
fn spec(table: &str) -> PlanTask {
    let mut plan = parse_plan(&plan_with(PROFILE, &[table.to_string()]))
        .unwrap_or_else(|e| panic!("fixture task must parse: {e}"));
    plan.tasks.pop().expect("one task")
}

fn set_state(run: &mut Run, id: &str, state: TaskState, block: Option<BlockReason>) {
    let task = run
        .tasks
        .iter_mut()
        .find(|t| t.spec.id == id)
        .unwrap_or_else(|| panic!("no task {id}"));
    task.state = state;
    task.block = block.map(|reason| BlockInfo {
        reason,
        text: format!("fixture block on {id}"),
    });
}

fn apply(run: &Run, edits: Vec<PlanEdit>) -> Result<(Run, Vec<EditConsequence>), Vec<PlanError>> {
    apply_edits(run, &edits, &EditScope::Run, 5_000)
}

fn applied(run: &Run, edits: Vec<PlanEdit>) -> (Run, Vec<EditConsequence>) {
    match apply(run, edits) {
        Ok(out) => out,
        Err(errors) => panic!("expected the batch to apply, got:\n{}", show(&errors)),
    }
}

fn rejected(run: &Run, edits: Vec<PlanEdit>) -> Vec<String> {
    match apply(run, edits) {
        Ok(_) => panic!("expected the batch to be rejected"),
        Err(errors) => errors.iter().map(ToString::to_string).collect(),
    }
}

/// The optional fields of an `amend_task`.
#[derive(Default)]
struct Amend {
    brief: Option<String>,
    acceptance: Option<Vec<String>>,
    route: Option<RouteSpec>,
    test_mode: Option<TestMode>,
    test_mode_reason: Option<String>,
    priority: Option<i32>,
    size: Option<Size>,
}

fn amend(task_id: &str, a: Amend) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: task_id.to_string(),
        brief: a.brief,
        acceptance: a.acceptance,
        route: a.route,
        test_mode: a.test_mode,
        test_mode_reason: a.test_mode_reason,
        priority: a.priority,
        size: a.size,
    }
}

fn amend_brief(task_id: &str, brief: &str, acceptance: &[&str]) -> PlanEdit {
    amend(
        task_id,
        Amend {
            brief: Some(brief.to_string()),
            acceptance: Some(acceptance.iter().map(|s| s.to_string()).collect()),
            ..Amend::default()
        },
    )
}

fn amend_size(task_id: &str, size: Size) -> PlanEdit {
    amend(
        task_id,
        Amend {
            size: Some(size),
            ..Amend::default()
        },
    )
}

fn add_dep(task_id: &str, dep: &str) -> PlanEdit {
    PlanEdit::AddDep {
        task_id: task_id.to_string(),
        dep: dep.to_string(),
    }
}

fn answer(task_id: &str, text: &str) -> PlanEdit {
    PlanEdit::Answer {
        task_id: task_id.to_string(),
        text: text.to_string(),
    }
}

fn cancel(task_id: &str) -> PlanEdit {
    PlanEdit::CancelTask {
        task_id: task_id.to_string(),
    }
}

fn ids(run: &Run) -> Vec<&str> {
    run.tasks.iter().map(|t| t.id()).collect()
}

#[test]
fn add_task_is_validated_like_a_plan_task() {
    let base = [one("t1", ""), one("t2", "deps = [\"t1\"]")];

    // Invalid in four rule families at once: spans two modules with interface_change
    // (7.2.2 raise, then 7.2.4), check without a reason (8), an unknown model (route),
    // and a blank acceptance item (fields).
    let bad = task_toml(
        "t9",
        "S",
        "[\"crates/a/src/x.rs\", \"crates/b/src/y.rs\"]",
        "interface_change = true\ntest_mode = \"check\"\nacceptance = [\"ok\", \" \"]\n[task.route]\nmodel = \"no-such-model\"",
    )
    .replace("acceptance = [\"Accept t9\"]\n", "");
    let mut plan_tasks = base.to_vec();
    plan_tasks.push(bad.clone());
    let from_plan: Vec<String> = errors_of(&plan_with(PROFILE, &plan_tasks))
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        from_plan.len() >= 4,
        "the fixture must break several rules: {from_plan:?}"
    );

    let run = run_ok(&plan_with(PROFILE, &base));
    let from_edit = rejected(&run, vec![PlanEdit::AddTask { task: spec(&bad) }]);
    assert_eq!(from_edit, from_plan);

    // A valid task is resolved exactly as the plan resolves it: spanning two modules
    // raises it to M with a note, and it gets its branch and worktree.
    let good = task_toml(
        "t8",
        "S",
        "[\"crates/c/src/x.rs\", \"crates/d/src/y.rs\"]",
        "deps = [\"t2\"]\npriority = 7",
    );
    let mut plan_tasks = base.to_vec();
    plan_tasks.push(good.clone());
    let planned = run_ok(&plan_with(PROFILE, &plan_tasks));
    let (edited, consequences) = applied(&run, vec![PlanEdit::AddTask { task: spec(&good) }]);
    assert_eq!(consequences, vec![]);
    assert_eq!(ids(&edited), vec!["t1", "t2", "t8"]);
    let mut added = task(&edited, "t8").clone();
    assert_eq!(added.size, Size::M);
    assert_eq!(added.state, TaskState::Pending);
    added.history.clear();
    assert_eq!(&added, task(&planned, "t8"));
}

#[test]
fn amend_route_on_a_working_task_is_refused() {
    let mut run = flat();
    set_state(&mut run, "t1", TaskState::Working, None);
    let high = RouteSpec {
        effort: Some(Effort::High),
        ..RouteSpec::default()
    };
    let with_route = |id: &str| {
        amend(
            id,
            Amend {
                route: Some(high.clone()),
                ..Amend::default()
            },
        )
    };

    let errors = match apply(&run, vec![with_route("t1")]) {
        Err(errors) => errors,
        Ok(_) => panic!("amending a working task's route must be refused"),
    };
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].task.as_deref(), Some("t1"));
    assert_eq!(
        errors[0].to_string(),
        "task t1 is working; route can be amended only on pending, queued or blocked tasks"
    );

    // Size and test mode follow the same rule, each named.
    let with_mode = amend(
        "t1",
        Amend {
            test_mode: Some(TestMode::Check),
            test_mode_reason: Some("covered by the check".to_string()),
            ..Amend::default()
        },
    );
    assert_eq!(
        rejected(&run, vec![amend_size("t1", Size::M), with_mode]),
        vec![
            "task t1 is working; size can be amended only on pending, queued or blocked tasks",
            "task t1 is working; test_mode can be amended only on pending, queued or blocked tasks",
            "task t1 is working; test_mode_reason can be amended only on pending, queued or blocked tasks",
        ]
    );

    // The same route on a queued task applies and re-resolves the route.
    set_state(&mut run, "t3", TaskState::Queued, None);
    assert_eq!(task(&run, "t3").route.effort, Effort::Low);
    let (edited, consequences) = applied(&run, vec![with_route("t3")]);
    assert_eq!(consequences, vec![]);
    assert_eq!(task(&edited, "t3").route.effort, Effort::High);
    assert_eq!(task(&edited, "t1").route.effort, Effort::Low);
}

#[test]
fn amend_brief_on_a_working_task_yields_a_deliver_consequence() {
    let mut run = flat();
    set_state(&mut run, "t2", TaskState::Working, None);

    let (edited, consequences) = applied(
        &run,
        vec![amend_brief(
            "t2",
            "Rewrite the parser",
            &["parses a", "rejects b"],
        )],
    );
    let amended = task(&edited, "t2");
    assert_eq!(amended.spec.brief, "Rewrite the parser");
    assert_eq!(amended.spec.acceptance, vec!["parses a", "rejects b"]);
    assert_eq!(amended.state, TaskState::Working);
    let text = amend_message(amended);
    assert_eq!(
        text,
        "[anthrex] The task was amended.\nBrief: Rewrite the parser\nAcceptance criteria:\n- parses a\n- rejects b\nContinue with the amended task."
    );
    assert_eq!(
        consequences,
        vec![EditConsequence::Deliver {
            task_id: "t2".to_string(),
            text
        }]
    );

    // A task with no live worker just changes; a priority change alone delivers nothing.
    let priority = amend(
        "t2",
        Amend {
            priority: Some(9),
            ..Amend::default()
        },
    );
    let (edited, consequences) = applied(
        &run,
        vec![amend_brief("t4", "Another brief", &["c"]), priority],
    );
    assert_eq!(consequences, vec![]);
    assert_eq!(task(&edited, "t4").spec.brief, "Another brief");
    assert_eq!(task(&edited, "t2").spec.priority, 9);

    // A finished task cannot be amended at all.
    set_state(&mut run, "t3", TaskState::Merged, None);
    assert_eq!(
        rejected(&run, vec![amend_brief("t3", "Too late", &["d"])]),
        vec!["task t3 is merged; only unfinished tasks can be amended"]
    );
}

#[test]
fn cancel_of_a_pending_task_marks_dependents_dep_cancelled() {
    let run = chain();
    let (edited, consequences) = applied(&run, vec![cancel("t2")]);
    assert_eq!(consequences, vec![]);
    assert_eq!(task(&edited, "t2").state, TaskState::Cancelled);
    let t3 = task(&edited, "t3");
    assert_eq!(t3.state, TaskState::Blocked);
    assert_eq!(
        t3.block,
        Some(BlockInfo {
            reason: BlockReason::DepCancelled,
            text: "dependency t2 was cancelled".to_string(),
        })
    );
    assert_eq!(task(&edited, "t1").state, TaskState::Pending);
    assert_eq!(task(&edited, "t4").state, TaskState::Pending);
    assert_eq!(task(&edited, "t4").block, None);
    assert_eq!(
        task(&edited, "t2")
            .history
            .last()
            .map(|e| (e.at, e.text.as_str())),
        Some((5_000, "cancelled by a plan edit"))
    );

    // A finished task cannot be cancelled again.
    assert_eq!(
        rejected(&edited, vec![cancel("t2")]),
        vec!["task t2 is cancelled; only unfinished tasks can be cancelled"]
    );
}

#[test]
fn cancel_of_a_working_task_yields_cancel_live() {
    let mut run = chain();
    set_state(&mut run, "t1", TaskState::Working, None);
    set_state(&mut run, "t4", TaskState::MergeQueue, None);
    run.merge_queue = vec!["t4".to_string()];

    let (edited, consequences) = applied(&run, vec![cancel("t1"), cancel("t4")]);
    assert_eq!(
        consequences,
        vec![
            EditConsequence::CancelLive {
                task_id: "t1".to_string()
            },
            EditConsequence::CancelLive {
                task_id: "t4".to_string()
            },
        ]
    );
    assert_eq!(task(&edited, "t1").state, TaskState::Cancelled);
    assert_eq!(task(&edited, "t4").state, TaskState::Cancelled);
    assert_eq!(edited.merge_queue, Vec::<String>::new());
    assert_eq!(task(&edited, "t2").state, TaskState::Blocked);

    // A blocked task whose session is still open is live too.
    let mut run = chain();
    set_state(
        &mut run,
        "t3",
        TaskState::Blocked,
        Some(BlockReason::Question),
    );
    run.tasks[2].rounds.push(open_round());
    let (_, consequences) = applied(&run, vec![cancel("t3")]);
    assert_eq!(
        consequences,
        vec![EditConsequence::CancelLive {
            task_id: "t3".to_string()
        }]
    );
}

fn open_round() -> AgentRound {
    AgentRound {
        role: AgentRole::Worker,
        session: 1,
        round: 1,
        window_id: Some(7),
        route: task(&chain(), "t1").route.clone(),
        launch_op: 1,
        session_id: None,
        pid: None,
        ended: false,
        started_at: 0,
        ended_at: None,
        turn_open: false,
        turns: 1,
        turn_had_task_done: false,
        last_event: 0,
        tool_calls: 0,
        rate_limited_until: None,
        in_retry_streak: false,
        open_subagents: Default::default(),
        denials: 0,
        usage: Default::default(),
        deaths: 0,
        fallback: Default::default(),
        stall: Default::default(),
        failed_turn: Default::default(),
        review_nudged: false,
        wrap_up_sent: false,
        retiring: false,
        delivery_failures: 0,
        delivery_retry_at: None,
        turn_denied: Vec::new(),
        last_denial: None,
        fallback_waiting: false,
        carried: Vec::new(),
        failed_error: None,
        resume_op: None,
        count_op: None,
        count_failures: 0,
        count_retry_at: None,
    }
}

#[test]
fn split_rewires_dependents_to_every_child() {
    let run = chain();
    let split = PlanEdit::SplitTask {
        task_id: "t2".to_string(),
        into: vec![spec(&one("t2a", "deps = [\"t1\"]")), spec(&one("t2b", ""))],
    };
    let (edited, consequences) = applied(&run, vec![split]);
    assert_eq!(consequences, vec![]);
    assert_eq!(ids(&edited), vec!["t1", "t2", "t2a", "t2b", "t3", "t4"]);
    assert_eq!(task(&edited, "t2").state, TaskState::Cancelled);
    let t3 = task(&edited, "t3");
    assert_eq!(t3.spec.deps, vec!["t2a", "t2b"]);
    assert_eq!(t3.state, TaskState::Pending);
    assert_eq!(t3.block, None);
    assert_eq!(task(&edited, "t2a").spec.deps, vec!["t1"]);
    assert_eq!(task(&edited, "t2b").branch, format!("anthrex/{RUN_ID}/t2b"));

    // Only a task that has not started can be split.
    let mut working = chain();
    set_state(&mut working, "t2", TaskState::Working, None);
    let split = PlanEdit::SplitTask {
        task_id: "t2".to_string(),
        into: vec![spec(&one("t2a", ""))],
    };
    assert_eq!(
        rejected(&working, vec![split]),
        vec!["task t2 is working; only pending, queued or blocked tasks can be split"]
    );

    // A split needs at least one task to split into.
    let empty = PlanEdit::SplitTask {
        task_id: "t2".to_string(),
        into: Vec::new(),
    };
    assert_eq!(
        rejected(&run, vec![empty]),
        vec!["task t2: into: at least one task is required"]
    );
}

#[path = "edits_tests_rules.rs"]
mod rules;

#[path = "edits_tests_state.rs"]
mod state;
