//! Milestone 9.17 task M9.17.6 (decision 19): `anthrex run status` prints the plan
//! frame after the goal whenever the snapshot carries one.

use common::numbers::Zone;
use proto::{
    DecidedItem, Effort, PlanFrameInfo, PlannerInfo, PlannerState, Route, RunInfo, Runtime,
};

use super::super::status::run_block;
use super::super::status::tests::example;

fn frame(decided: &[(&str, &str)], out: &[&str]) -> PlanFrameInfo {
    PlanFrameInfo {
        decided: (decided.iter())
            .map(|(q, a)| DecidedItem {
                question: q.to_string(),
                answer: a.to_string(),
            })
            .collect(),
        out_of_scope: out.iter().map(|s| s.to_string()).collect(),
        spec_non_goals: None,
    }
}

fn planner(epic: &str, frame: Option<PlanFrameInfo>) -> PlannerInfo {
    PlannerInfo {
        epic: epic.into(),
        title: epic.into(),
        area: vec![],
        route: Route {
            runtime: Runtime::Codex,
            model: String::new(),
            effort: Effort::MEDIUM,
        },
        window_id: None,
        state: PlannerState::Finished,
        started_at: 1,
        ended_at: None,
        edits_accepted: 0,
        edits_rejected: 0,
        last_rejection: None,
        replans: vec![],
        note: None,
        covers: Vec::new(),
        frame,
    }
}

/// The lines right after `  goal: …`, up to the next line that is not a frame line.
fn after_goal(run: &RunInfo) -> Vec<String> {
    let block = run_block(run, 0, Zone::Local(0));
    let lines: Vec<&str> = block.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.starts_with("  goal: "))
        .unwrap();
    (lines[at + 1..].iter())
        .take_while(|l| {
            [
                "  decided for you: ",
                "  out of scope: ",
                "  spec non-goals: ",
            ]
            .iter()
            .any(|p| l.starts_with(p))
        })
        .map(|l| l.to_string())
        .collect()
}

#[test]
fn frame_lines_print_after_goal() {
    let mut run = example();
    run.plan_frame = Some(frame(&[("Which DB?", "SQLite")], &["no UI"]));
    run.planners = vec![
        planner("e1", Some(frame(&[], &["no docs"]))),
        planner("e2", None),
    ];
    assert_eq!(
        after_goal(&run),
        [
            "  decided for you: Which DB? → SQLite",
            "  out of scope: no UI",
            "  out of scope: [e1] no docs",
        ]
    );
}

#[test]
fn none_stated() {
    let mut run = example();
    run.plan_frame = Some(PlanFrameInfo::default());
    assert_eq!(after_goal(&run), ["  decided for you: none stated"]);
    // An epic's empty frame alone says the same.
    run.plan_frame = None;
    run.planners = vec![planner("e1", Some(PlanFrameInfo::default()))];
    assert_eq!(after_goal(&run), ["  decided for you: none stated"]);
}

/// A design run's approved Non-goals, a line each, after the frame's items.
#[test]
fn spec_non_goals_print_a_line_each() {
    let mut run = example();
    run.plan_frame = Some(PlanFrameInfo {
        spec_non_goals: Some("- no mobile app\n\n- no SSO".into()),
        ..PlanFrameInfo::default()
    });
    assert_eq!(
        after_goal(&run),
        [
            "  spec non-goals: - no mobile app",
            "  spec non-goals: - no SSO"
        ]
    );
}

#[test]
fn frame_text_is_sanitised() {
    let mut run = example();
    let esc = '\u{1b}';
    let bidi = '\u{202E}';
    run.plan_frame = Some(frame(
        &[(&format!("Which{esc}[2J DB?"), &format!("SQ{bidi}Lite\nx"))],
        &[&format!("no{esc}]0;t\u{7} UI")],
    ));
    run.planners = vec![planner(&format!("e{esc}1"), Some(frame(&[], &["d"])))];
    let lines = after_goal(&run);
    assert_eq!(lines.len(), 3, "{lines:?}");
    for line in &lines {
        assert!(
            !line.chars().any(|c| c.is_control() || c == bidi),
            "{line:?}"
        );
    }
}

#[test]
fn no_frame_prints_nothing() {
    let run = example();
    assert!(after_goal(&run).is_empty());
    let block = run_block(&run, 0, Zone::Local(0));
    assert!(!block.contains("decided for you") && !block.contains("out of scope"));
}
