//! Milestone 9.17 decisions 22–25 (task M9.17.7): `add_task`'s `replaces`, and the link
//! `split_task` records itself. Every batch goes through `apply_edits`, the pure core
//! of the engine's `apply_batch`.

use proto::{AgentRole, BlockInfo, BlockReason, PlanEdit, PlanTask, TaskState};
use serde_json::json;

use crate::run::edits::{EditConsequence, apply_edits};
use crate::run::model::Run;
use crate::run::orch::tools::{OrchCall, parse_call};
use crate::run::orch::{EditSource, EpicRecord, PlannerPhase};
use crate::run::plan::PlanError;
use crate::run::test_support::{PROFILE, plan_with, run_ok, show, spec_of, task, task_toml};
use crate::run::validate::EditScope;

/// One S task owning `crates/<id>/src/lib.rs`.
fn one(id: &str, extra: &str) -> String {
    task_toml(id, "S", &format!("[\"crates/{id}/src/lib.rs\"]"), extra)
}

fn new_task(id: &str) -> PlanTask {
    spec_of(&one(id, ""))
}

/// `add_task <id> replaces <ids>`.
fn add(id: &str, replaces: &[&str]) -> PlanEdit {
    PlanEdit::AddTask {
        task: new_task(id),
        replaces: replaces.iter().map(|r| r.to_string()).collect(),
    }
}

fn cancel(id: &str) -> PlanEdit {
    PlanEdit::CancelTask {
        task_id: id.to_string(),
    }
}

/// t1; t3 depends on t1; t4 stands alone.
fn fixture() -> Run {
    run_ok(&plan_with(
        PROFILE,
        &[one("t1", ""), one("t3", "deps = [\"t1\"]"), one("t4", "")],
    ))
}

fn set(run: &mut Run, id: &str, state: TaskState) {
    let t = run.tasks.iter_mut().find(|t| t.id() == id).unwrap();
    t.state = state;
    t.block = (state == TaskState::Blocked)
        .then(|| BlockInfo::new(BlockReason::Question, format!("which way for {id}?")));
}

fn apply(
    run: &Run,
    edits: &[PlanEdit],
    source: &EditSource,
) -> Result<(Run, Vec<EditConsequence>), Vec<PlanError>> {
    apply_edits(run, edits, &EditScope::Run, source, 7_000)
}

fn applied(run: &Run, edits: &[PlanEdit], source: &EditSource) -> Run {
    match apply(run, edits, source) {
        Ok((edited, _)) => edited,
        Err(errors) => panic!("expected the batch to apply:\n{}", show(&errors)),
    }
}

/// Every error's `(rule, text)`.
fn refused(run: &Run, edits: &[PlanEdit], source: &EditSource) -> Vec<(String, String)> {
    match apply(run, edits, source) {
        Ok(_) => panic!("expected the batch to be refused"),
        Err(errors) => errors
            .iter()
            .map(|e| (e.rule.clone(), e.to_string()))
            .collect(),
    }
}

fn last_history(run: &Run, id: &str) -> String {
    task(run, id).history.last().unwrap().text.clone()
}

#[test]
fn replacing_a_cancelled_task_links_it() {
    let mut run = fixture();
    set(&mut run, "t1", TaskState::Cancelled);
    let edited = applied(&run, &[add("t2", &["t1"])], &EditSource::Orchestrator);
    assert_eq!(task(&edited, "t2").replaces, ["t1"]);
    assert_eq!(task(&edited, "t1").state, TaskState::Cancelled);
    assert_eq!(last_history(&edited, "t1"), "replaced by t2");
    // Nothing else about the replaced task changes.
    let mut was = task(&run, "t1").clone();
    let now = task(&edited, "t1");
    was.history = now.history.clone();
    assert_eq!(&was, now);
}

#[test]
fn replacing_a_blocked_task_cancels_it() {
    let mut run = fixture();
    set(&mut run, "t1", TaskState::Blocked);
    let edited = applied(&run, &[add("t2", &["t1"])], &EditSource::Orchestrator);
    let t1 = task(&edited, "t1");
    assert_eq!(t1.state, TaskState::Cancelled);
    assert_eq!(t1.block, None);
    assert_eq!(last_history(&edited, "t1"), "cancelled: replaced by t2");
    let t3 = task(&edited, "t3");
    assert_eq!(t3.state, TaskState::Blocked);
    assert_eq!(t3.block.as_ref().unwrap().reason, BlockReason::DepCancelled);
    // Nothing is re-pointed (decision 24).
    assert_eq!(t3.spec.deps, ["t1"]);
    assert_eq!(task(&edited, "t2").replaces, ["t1"]);
}

#[test]
fn cancel_then_replace_in_one_batch() {
    let mut run = fixture();
    set(&mut run, "t1", TaskState::Working);
    let (edited, consequences) = apply(
        &run,
        &[cancel("t1"), add("t2", &["t1"])],
        &EditSource::Orchestrator,
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(task(&edited, "t1").state, TaskState::Cancelled);
    assert_eq!(last_history(&edited, "t1"), "replaced by t2");
    assert_eq!(task(&edited, "t2").replaces, ["t1"]);
    assert!(consequences.contains(&EditConsequence::CancelLive {
        task_id: "t1".into()
    }));
}

/// Rule (b) reads the run after the batch's earlier edits, so order matters.
#[test]
fn replace_before_cancel_in_one_batch_is_refused() {
    let mut run = fixture();
    set(&mut run, "t1", TaskState::Working);
    assert_eq!(
        refused(
            &run,
            &[add("t2", &["t1"]), cancel("t1")],
            &EditSource::Orchestrator
        ),
        [(
            "replaces".to_string(),
            "replaces: t1 is working; only a cancelled or blocked task can be replaced".to_string()
        )]
    );
}

#[test]
fn each_rule_refuses_the_whole_batch() {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[
            one("t1", ""),
            one("t3", ""),
            one("t4", ""),
            one("a1", "epic = \"auth\""),
            one("b1", "epic = \"billing\""),
            one("u1", ""),
        ],
    ));
    set(&mut run, "t1", TaskState::Cancelled);
    set(&mut run, "t3", TaskState::Merged);
    set(&mut run, "t4", TaskState::Working);
    set(&mut run, "a1", TaskState::Cancelled);
    set(&mut run, "b1", TaskState::Cancelled);
    // Pre-flight ruling C3: 9.13's retry cap (`dispatch::block`) makes the third block
    // user-only, and `replaces` never cancels it for the orchestrator or a planner.
    set(&mut run, "u1", TaskState::Blocked);
    let u1 = run.tasks.iter_mut().find(|t| t.id() == "u1").unwrap();
    let block = u1.block.as_mut().unwrap();
    block.text = "which way? · the orchestrator retried it 2 times; the user decides".into();
    block.user_only = true;
    u1.orch.orch_retries = 2;
    let user_only = "task u1 waits on something only the user can fix, and they have been \
                     alerted: which way? · the orchestrator retried it 2 times; the user decides";
    for e in ["auth", "billing"] {
        (run.orch.epics).push(EpicRecord::new(e, PlannerPhase::Planning));
    }
    let orch = EditSource::Orchestrator;
    let planner = EditSource::Planner {
        epic: "auth".into(),
    };
    let thirteen: Vec<String> = (0..13).map(|i| format!("x{i}")).collect();
    let thirteen: Vec<&str> = thirteen.iter().map(String::as_str).collect();
    let cases: Vec<(&str, PlanEdit, &EditSource, &str)> = vec![
        (
            "unknown",
            add("t2", &["t9"]),
            &orch,
            "replaces: no task t9 in this run",
        ),
        (
            "merged",
            add("t2", &["t3"]),
            &orch,
            "replaces: t3 is merged; only a cancelled or blocked task can be replaced",
        ),
        (
            "working",
            add("t2", &["t4"]),
            &orch,
            "replaces: t4 is working; only a cancelled or blocked task can be replaced",
        ),
        (
            "twice",
            add("t2", &["t1", "t1"]),
            &orch,
            "replaces: t1 is named twice",
        ),
        (
            "thirteen",
            add("t2", &thirteen),
            &orch,
            "replaces: at most 12 tasks, not 13",
        ),
        (
            "another epic",
            add("a2", &["b1"]),
            &planner,
            "replaces: b1 is not in epic auth",
        ),
        ("user-only block", add("t2", &["u1"]), &orch, user_only),
    ];
    for (case, edit, source, text) in cases {
        // A valid edit first: the refusal takes the whole batch.
        let edits = [PlanEdit::add_task(new_task("t8")), edit];
        let before = run.clone();
        assert_eq!(
            refused(&run, &edits, source),
            [("replaces".to_string(), text.to_string())],
            "{case}"
        );
        assert_eq!(run, before, "{case}");
    }
    // The controls: the planner replaces its own epic's task, and the user replaces
    // the user-only block.
    let edited = applied(&run, &[add("a2", &["a1"])], &planner);
    assert_eq!(task(&edited, "a2").replaces, ["a1"]);
    let edited = applied(&run, &[add("t2", &["u1"])], &EditSource::User);
    assert_eq!(task(&edited, "u1").state, TaskState::Cancelled);
}

#[test]
fn two_tasks_may_replace_one() {
    let mut run = fixture();
    set(&mut run, "t1", TaskState::Blocked);
    let edited = applied(
        &run,
        &[add("t2", &["t1"]), add("t5", &["t1"])],
        &EditSource::Orchestrator,
    );
    assert_eq!(task(&edited, "t2").replaces, ["t1"]);
    assert_eq!(task(&edited, "t5").replaces, ["t1"]);
    let lines: Vec<&str> = (task(&edited, "t1").history.iter())
        .map(|e| e.text.as_str())
        .collect();
    assert!(lines.contains(&"cancelled: replaced by t2"), "{lines:?}");
    assert_eq!(lines.last(), Some(&"replaced by t5"));
}

#[test]
fn split_children_replace_the_original() {
    let run = fixture();
    let split = PlanEdit::SplitTask {
        task_id: "t1".into(),
        into: vec![new_task("t1a"), new_task("t1b")],
    };
    let edited = applied(&run, &[split], &EditSource::Orchestrator);
    assert_eq!(task(&edited, "t1").state, TaskState::Cancelled);
    assert_eq!(task(&edited, "t1a").replaces, ["t1"]);
    assert_eq!(task(&edited, "t1b").replaces, ["t1"]);
}

/// Decision 22: only `add_task` takes `replaces`; serde refuses it on another op, and
/// the tool bounds let `add_task`'s through (at most 12 ids of at most 16 characters).
#[test]
fn replaces_on_another_op_is_refused() {
    let edit_plan = |edit: serde_json::Value| {
        parse_call(
            AgentRole::Orchestrator,
            "edit_plan",
            &json!({ "edits": [edit] }),
        )
    };
    let error = edit_plan(json!({"op": "cancel_task", "task_id": "t1", "replaces": ["t2"]}))
        .expect_err("refused");
    assert!(error.contains("unknown field `replaces`"), "{error}");
    let task = json!({"id": "t2", "title": "T", "kind": "code", "size": "S",
                      "owns": ["a.txt"], "brief": "B", "acceptance": ["A"]});
    let call = edit_plan(json!({"op": "add_task", "task": task, "replaces": ["t1"]}))
        .unwrap_or_else(|e| panic!("{e}"));
    let OrchCall::EditPlan { edits, .. } = call else {
        panic!("an edit_plan call")
    };
    assert!(
        matches!(&edits[0], PlanEdit::AddTask { replaces, .. } if replaces == &["t1"]),
        "{edits:?}"
    );
    let thirteen: Vec<String> = (0..13).map(|i| format!("t{i}")).collect();
    let error = edit_plan(json!({"op": "add_task", "task": task, "replaces": thirteen}))
        .expect_err("refused");
    assert!(error.contains("replaces: at most 12 items"), "{error}");
    let long = "x".repeat(17);
    let error = edit_plan(json!({"op": "add_task", "task": task, "replaces": [long]}))
        .expect_err("refused");
    assert!(error.contains("must be 1 to 16 characters"), "{error}");
}

/// Rule (e): a cycle in the replaced-by relation is found and named.
#[test]
fn a_replaces_cycle_is_named() {
    let mut run = fixture();
    run.tasks[0].replaces = vec!["t3".into()];
    run.tasks[1].replaces = vec!["t4".into()];
    assert_eq!(super::cycle(&run, "t1"), None);
    run.tasks[2].replaces = vec!["t1".into()];
    assert_eq!(
        super::cycle(&run, "t1").as_deref(),
        Some("replaces: t1 → t3 → t4 → t1 is a cycle")
    );
}

/// Review M1: the walk visits each task once, so a layered DAG of shared ancestors
/// costs its links, not its paths. Each of `LAYERS` layers of two tasks replaces both
/// tasks of the layer below: 2^LAYERS paths, but 4 links per layer.
#[test]
fn the_cycle_walk_is_linear() {
    const LAYERS: usize = 12;
    let ids: Vec<String> = (0..LAYERS)
        .flat_map(|l| [format!("l{l}a"), format!("l{l}b")])
        .chain(["top".to_string()])
        .collect();
    let tomls: Vec<String> = ids.iter().map(|id| one(id, "")).collect();
    let mut run = run_ok(&plan_with(PROFILE, &tomls));
    let layer = |l: usize| vec![format!("l{l}a"), format!("l{l}b")];
    for l in 1..LAYERS {
        for side in ["a", "b"] {
            let i = run.tasks.iter().position(|t| t.id() == format!("l{l}{side}"));
            run.tasks[i.unwrap()].replaces = layer(l - 1);
        }
    }
    let top = run.tasks.iter().position(|t| t.id() == "top").unwrap();
    run.tasks[top].replaces = layer(LAYERS - 1);
    let links = 2 + 4 * (LAYERS - 1);
    let (found, steps) = super::cycle_walk(&run, "top");
    assert_eq!(found, None);
    assert!(steps <= links, "{steps} steps for {links} links");
}
