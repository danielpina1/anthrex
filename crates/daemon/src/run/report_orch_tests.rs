//! Milestone 9.17 task M9.17.6 (decision 19): the report's `Plan frame` section lists
//! every round's and every epic's frame, whatever the run's state.

use crate::run::orch::test_support::{plan_frame, run_of};
use crate::run::orch::{EpicRecord, PlannerPhase};
use crate::run::report::render;

#[test]
fn report_lists_every_round_and_epic_frame() {
    let mut run = run_of(1);
    run.state = proto::RunState::Accepted;
    run.orch.plan_frames = vec![
        plan_frame(1, &[("Which DB?", "SQLite")], &["no UI"]),
        plan_frame(2, &[], &[]),
    ];
    let mut framed = EpicRecord::new("e1", PlannerPhase::Finished);
    framed.frame = Some(plan_frame(1, &[], &["# no docs"]));
    run.orch.epics = vec![framed, EpicRecord::new("e2", PlannerPhase::Finished)];
    let text = render(&run, 2_000);
    let section = text
        .split("\n## Plan frame\n\n")
        .nth(1)
        .expect("a Plan frame section")
        .split("\n## ")
        .next()
        .unwrap();
    assert_eq!(
        section,
        "### Round 1\n\n\
         - decided for you: Which DB? → SQLite\n\
         - out of scope: no UI\n\n\
         ### Round 2\n\n\
         - decided for you: none stated\n\
         - out of scope: none stated\n\n\
         ### Epic e1\n\n\
         - decided for you: none stated\n\
         - out of scope: # no docs\n\n"
    );
    // A run with no frame has no section.
    run.orch.plan_frames.clear();
    run.orch.epics.clear();
    assert!(!render(&run, 2_000).contains("## Plan frame"));
}

/// Decision 26: a replaced task's state reads `cancelled (replaced by <ids>)`, in the
/// tasks table and in its own section; any other task's is unchanged.
#[test]
fn report_marks_replaced_cancel() {
    let mut run = run_of(3);
    run.tasks[0].state = proto::TaskState::Cancelled;
    run.tasks[1].state = proto::TaskState::Cancelled;
    run.tasks[2].replaces = vec!["t0".into()];
    let text = render(&run, 2_000);
    assert!(
        text.contains("| t0 | Title t0 | S | tdd | cancelled (replaced by t2) |"),
        "{text}"
    );
    assert!(
        text.contains("| t1 | Title t1 | S | tdd | cancelled |"),
        "{text}"
    );
    let section = |id: &str| {
        let head = format!("## {id}: Title {id}\n\n");
        let rest = text.split(&head).nth(1).expect("a section").to_string();
        rest.lines().next().unwrap().to_string()
    };
    assert!(
        section("t0").starts_with("State: cancelled (replaced by t2) (size S"),
        "{}",
        section("t0")
    );
    assert!(section("t1").starts_with("State: cancelled (size S"));
}
