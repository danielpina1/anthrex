//! Milestone 9.5 task M9.5.14: `race` and `pair` on plan tasks (decisions 17 and 24),
//! their amends (rulings RR-7 and RR-8, review ruling I9) and their reach (RR-6). Pure.

use proto::{BlockInfo, BlockReason, PlanEdit, Runtime, TaskState};
use serde_json::json;

use super::*;
use crate::run::edits::apply_edits;
use crate::run::model::{Round, Run};
use crate::run::orch::EditSource;
use crate::run::reach::{edits_may_widen, reachable_runtimes};
use crate::run::test_support::*;
use crate::run::validate::EditScope;

/// One S code task owning `crates/<id>/src/lib.rs`: one module, inside `source`.
fn code(id: &str, extra: &str) -> String {
    task_toml(id, "S", &format!("[\"crates/{id}/src/lib.rs\"]"), extra)
}

fn shown(errors: &[PlanError]) -> Vec<String> {
    errors.iter().map(ToString::to_string).collect()
}

fn amend(task_id: &str, field: &str, on: bool) -> PlanEdit {
    serde_json::from_value(json!({"op": "amend_task", "task_id": task_id, field: on})).unwrap()
}

fn add(table: &str) -> PlanEdit {
    let mut plan = crate::run::plan::parse_plan(&plan_with(PROFILE, &[table.to_string()]))
        .unwrap_or_else(|e| panic!("fixture task must parse: {e}"));
    PlanEdit::AddTask {
        task: plan.tasks.pop().expect("one task"),
    }
}

fn apply(run: &Run, edits: Vec<PlanEdit>) -> Result<Run, Vec<String>> {
    apply_edits(
        run,
        &edits,
        &EditScope::Run,
        &EditSource::Orchestrator,
        5_000,
    )
    .map(|(run, _)| run)
    .map_err(|errors| shown(&errors))
}

/// Four independent S code tasks on Claude, the default roster.
fn flat() -> Run {
    run_ok(&plan_with(
        PROFILE,
        &[
            code("t1", ""),
            code("t2", ""),
            code("t3", ""),
            code("t4", ""),
        ],
    ))
}

/// Milestone 9.8 (task M9.8.13): a race needs a code task, never a peer model: its
/// second racer is the row's fallback, else the task's own route (decision 28), so a
/// Claude-only roster (gone with strength, task M9.8.14) or a Codex the start found
/// missing no longer refuses it.
#[test]
fn race_needs_code_and_no_peer() {
    // A docs task.
    let docs = task_toml(
        "t1",
        "S",
        "[\"docs/a.md\"]",
        "kind = \"docs\"\ntest_mode_reason = \"prose\"\nrace = true",
    );
    assert_eq!(
        shown(&errors_of(&plan_with(PROFILE, &[docs]))),
        ["task t1: race: only code tasks can race (rule 4.1.race)"]
    );
    let text = plan_with(PROFILE, &[code("t1", "race = true")]);
    let built = build_with(&text, &config::Orchestrator::default())
        .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert!(built.task("t1").unwrap().spec.race);
    let mut moved = flat();
    moved.orch.installed = [("claude".to_string(), true), ("codex".to_string(), false)].into();
    moved.tasks.retain(|t| t.id() != "t1");
    let added = apply(&moved, vec![add(&code("t1", "race = true"))]).unwrap();
    assert!(added.task("t1").unwrap().spec.race);
}

#[test]
fn a_hub_cannot_race() {
    let hub = task_toml("t1", "M", "[\"crates/proto/**\"]", "race = true");
    assert_eq!(
        shown(&errors_of(&plan_with(PROFILE, &[hub]))),
        ["task t1: race: a hub task cannot race, because a hub runs alone (rule 4.1.race)"]
    );
    // An atomic task is a hub task too.
    let atomic = code(
        "t2",
        "atomic = true\natomic_reason = \"one step\"\nrace = true",
    );
    assert_eq!(
        shown(&errors_of(&plan_with(PROFILE, &[atomic]))),
        ["task t2: race: a hub task cannot race, because a hub runs alone (rule 4.1.race)"]
    );
}

#[test]
fn a_hub_may_pair() {
    let hub = task_toml(
        "t1",
        "M",
        "[\"crates/proto/**\"]",
        "pair = true\ntest_to_write = \"proto::parses\"",
    );
    let run = run_ok(&plan_with(PROFILE, &[hub]));
    let t1 = run.task("t1").unwrap();
    assert!(t1.hub && t1.spec.pair);
}

#[test]
fn race_and_pair_are_exclusive() {
    let both = code(
        "t1",
        "race = true\npair = true\ntest_to_write = \"t1::works\"",
    );
    assert_eq!(
        shown(&errors_of(&plan_with(PROFILE, &[both]))),
        ["task t1: pair: a task cannot both race and pair (rule 4.1)"]
    );
}

#[test]
fn pair_needs_tdd_after_resolution() {
    let paired = code("t1", "pair = true\ntest_to_write = \"t1::works\"");
    // A tdd task demoted by a missing single_test (rule 8.3) cannot pair.
    let no_single = PROFILE.replace("single_test = \"cargo test -- --exact {test}\"\n", "");
    assert_ne!(no_single, PROFILE, "the profile lost its single_test");
    let tdd = "task t1: pair: a paired task needs test mode tdd (rule 4.1.pair)";
    assert_eq!(
        shown(&errors_of(&plan_with(
            &no_single,
            std::slice::from_ref(&paired)
        ))),
        [tdd]
    );
    // Nor can a check task.
    let check = code(
        "t1",
        "pair = true\ntest_to_write = \"t1::works\"\ntest_mode = \"check\"\ntest_mode_reason = \"covered\"",
    );
    assert_eq!(shown(&errors_of(&plan_with(PROFILE, &[check]))), [tdd]);
    // With single_test it is tdd, and pairs.
    assert!(run_ok(&plan_with(PROFILE, &[paired])).tasks[0].spec.pair);
}

#[test]
fn pair_needs_test_to_write() {
    let text =
        "task t1: pair: name the test the test writer writes in test_to_write (rule 4.1.pair)";
    for extra in ["pair = true", "pair = true\ntest_to_write = \"  \""] {
        assert_eq!(
            shown(&errors_of(&plan_with(PROFILE, &[code("t1", extra)]))),
            [text],
            "{extra}"
        );
    }
}

/// Review ruling I9 (the stage rule's "started"): a working task, and a blocked one
/// with a start commit, have started; clearing is refused like setting (task 2 review
/// m5). A queued task, and a blocked one without a start commit, are amended.
#[test]
fn race_and_pair_amend_only_before_start() {
    for field in ["race", "pair"] {
        let refused = format!("task t1 has started: its {field} cannot change");
        let mut run = flat();
        run.tasks[0].state = TaskState::Working;
        run.tasks[0].start_commit = Some("c".repeat(40));
        for on in [true, false] {
            assert_eq!(
                apply(&run, vec![amend("t1", field, on)]).unwrap_err(),
                std::slice::from_ref(&refused),
                "working, {field} = {on}"
            );
        }
        run.tasks[0].state = TaskState::Blocked;
        run.tasks[0].block = Some(BlockInfo::new(BlockReason::Human, "fixture"));
        assert_eq!(
            apply(&run, vec![amend("t1", field, false)]).unwrap_err(),
            [refused],
            "blocked with a start commit, {field}"
        );
        // Without a start commit it never started.
        run.tasks[0].start_commit = None;
        let blocked = apply(&run, vec![amend("t1", field, false)]).unwrap();
        assert_eq!(blocked.task("t1").unwrap().state, TaskState::Blocked);
        let mut queued = flat();
        queued.tasks[0].state = TaskState::Queued;
        queued.tasks[0].spec.test_to_write = Some("t1::works".into());
        let amended = apply(&queued, vec![amend("t1", field, true)]).unwrap();
        let spec = &amended.task("t1").unwrap().spec;
        assert!(
            if field == "race" {
                spec.race
            } else {
                spec.pair
            },
            "{field}"
        );
    }
}

/// Ruling RR-7: 9.3's earlier-round refusal comes first.
#[test]
fn an_earlier_rounds_task_is_refused_first() {
    let mut run = flat();
    run.tasks[0].state = TaskState::Working;
    run.tasks[0].start_commit = Some("c".repeat(40));
    let first = Round::first(&run);
    run.rounds = vec![
        first.clone(),
        Round {
            n: 2,
            first_stage: 2,
            ..first
        },
    ];
    for task in run.tasks.iter_mut() {
        task.round = 1;
    }
    assert_eq!(
        apply(&run, vec![amend("t1", "race", true)]).unwrap_err(),
        ["stage 1 belongs to round 1, which is done; put new work in a new stage"]
    );
}

/// Ruling RR-6: lane b's runtime and the test writer's peer are reached; an amend of
/// either counts as widening (the engine's trust refusal is pinned in
/// `engine/tests/race_pair_plan.rs`).
#[test]
fn race_and_pair_widen_reach() {
    // Milestone 9.8 decision 31: on Claude Sonnet by its row (a plan's route is ignored).
    for flag in ["race = true", "pair = true\ntest_to_write = \"t1::works\""] {
        let text = plan_with(PROFILE, &[code("t1", flag)]);
        let run = run_ok(&text);
        assert_eq!(
            reachable_runtimes(&run),
            [Runtime::Claude, Runtime::Codex],
            "{flag}"
        );
    }
    for field in ["race", "pair"] {
        assert!(edits_may_widen(&[amend("t1", field, true)]), "{field}");
    }
}

/// Review I1 (the A-I1 class): a race is decided at dispatch, so a started racing task
/// is not judged again. Here rung 2 moved its route to Frontier and the task became a
/// hub (milestone 9.8: a race no longer needs a peer, so a hub is what the race rule
/// refuses), and a brief or an acceptance amend still applies.
#[test]
fn a_started_racing_task_on_a_moved_route_still_takes_a_brief() {
    let mut run = run_ok(&plan_with(PROFILE, &[code("t1", "race = true")]));
    let t1 = &mut run.tasks[0];
    t1.state = TaskState::Working;
    t1.start_commit = Some("c".repeat(40));
    t1.route = proto::Route {
        runtime: Runtime::Claude,
        model: "claude-opus-5-5".into(),
        effort: proto::Effort::HIGH,
    };
    t1.hub = true;
    let brief = json!({"op": "amend_task", "task_id": "t1", "brief": "New brief"});
    let accept = json!({"op": "amend_task", "task_id": "t1", "acceptance": ["New"]});
    for edit in [brief, accept] {
        let edited = apply(&run, vec![serde_json::from_value(edit.clone()).unwrap()]);
        assert!(edited.is_ok(), "{edit}: {edited:?}");
    }
    // The control: the same task before dispatch is judged, and refused.
    run.tasks[0].state = TaskState::Queued;
    run.tasks[0].start_commit = None;
    let brief = json!({"op": "amend_task", "task_id": "t1", "brief": "New brief"});
    assert_eq!(
        apply(&run, vec![serde_json::from_value(brief).unwrap()]).unwrap_err(),
        ["task t1: race: a hub task cannot race, because a hub runs alone (rule 4.1.race)"]
    );
}

/// Review m1: an amend naming `race` (or `pair`) and `route` on a started task gets
/// both refusals in one reply. Milestone 9.8 decision 31: only a user's route is read
/// (`run edit`); the orchestrator's is ignored, so it gets the one refusal.
#[test]
fn a_started_tasks_race_and_route_amend_reports_both_refusals() {
    for field in ["race", "pair"] {
        let mut run = flat();
        run.tasks[0].state = TaskState::Working;
        run.tasks[0].start_commit = Some("c".repeat(40));
        let edit = json!({"op": "amend_task", "task_id": "t1", field: true,
            "route": {"runtime": "codex", "model": ""}});
        let edit: PlanEdit = serde_json::from_value(edit).unwrap();
        let by_user = apply_edits(
            &run,
            std::slice::from_ref(&edit),
            &EditScope::Run,
            &EditSource::User,
            5,
        );
        let errors: Vec<String> = (by_user.unwrap_err().iter())
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            errors,
            [
                format!("task t1 has started: its {field} cannot change"),
                "task t1 is working; route can be amended only on pending, queued or blocked tasks"
                    .to_string(),
            ]
        );
        assert_eq!(
            apply(&run, vec![edit]).unwrap_err(),
            [format!("task t1 has started: its {field} cannot change")]
        );
    }
}
