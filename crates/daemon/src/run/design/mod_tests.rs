//! Task M9.6.3: `mode_for` follows DF §1's table, one test per row, then the overrides
//! (decision 3): the request first, then `[orchestrator.design] default`.

use proto::{DeciderSource, DesignMode, RunPath, Scale, TaskKind, TriageInfo};

use super::{GoalOrigin, mode_for};

fn triage(path: RunPath, kinds: &[TaskKind]) -> TriageInfo {
    TriageInfo {
        kinds: kinds.to_vec(),
        scale: match path {
            RunPath::Fast => Scale::Single,
            RunPath::Plan => Scale::Plan,
            RunPath::Large => Scale::Large,
        },
        path,
        reason: "a reason".into(),
        source: DeciderSource::Decider,
        fallback_reason: None,
        at: 1,
    }
}

fn config(default: DesignMode) -> config::DesignConfig {
    config::DesignConfig {
        default,
        ..config::DesignConfig::default()
    }
}

/// The mode for `origin` with no request, and with each request, under a config whose
/// default is `Full`.
fn modes(origin: GoalOrigin<'_>) -> [Result<DesignMode, String>; 3] {
    let full = config(DesignMode::Full);
    [
        mode_for(origin, None, &full),
        mode_for(origin, Some(DesignMode::Full), &full),
        mode_for(origin, Some(DesignMode::Off), &full),
    ]
}

fn refusal(what: &str) -> String {
    format!("the design flow runs only for planned code or docs goals; this goal is {what}")
}

/// Row 1: triage path plan or large, kind code or docs: on by default.
#[test]
fn mode_for_follows_the_table_planned_code_or_docs_is_on() {
    for path in [RunPath::Plan, RunPath::Large] {
        for kinds in [
            &[TaskKind::Code][..],
            &[TaskKind::Docs],
            &[TaskKind::Code, TaskKind::Docs],
        ] {
            let info = triage(path, kinds);
            assert_eq!(
                modes(GoalOrigin::Goal(Some(&info))),
                [
                    Ok(DesignMode::Full),
                    Ok(DesignMode::Full),
                    Ok(DesignMode::Off)
                ],
                "{path:?} {kinds:?}"
            );
        }
    }
}

/// Decision 3's M9.6.1 fact: a continued goal is never triaged, and reads as a planned
/// code goal (decision 30: a `start_goal` goal is `Full` by default).
#[test]
fn mode_for_follows_the_table_an_untriaged_goal_is_on() {
    assert_eq!(
        modes(GoalOrigin::Goal(None)),
        [
            Ok(DesignMode::Full),
            Ok(DesignMode::Full),
            Ok(DesignMode::Off)
        ]
    );
}

/// Row 2: the fast path is off, and a request for `Full` is refused exactly.
#[test]
fn mode_for_follows_the_table_the_fast_path_is_off() {
    let info = triage(RunPath::Fast, &[TaskKind::Code]);
    assert_eq!(
        modes(GoalOrigin::Goal(Some(&info))),
        [
            Ok(DesignMode::Off),
            Err(refusal("fast")),
            Ok(DesignMode::Off)
        ]
    );
}

/// Row 3: a research or review kind is off, on any path; the request is refused naming
/// the kind.
#[test]
fn mode_for_follows_the_table_research_and_review_are_off() {
    for (kind, what) in [
        (TaskKind::Research, "research"),
        (TaskKind::Review, "review"),
    ] {
        for path in [RunPath::Plan, RunPath::Large] {
            for kinds in [vec![kind], vec![TaskKind::Code, kind]] {
                let info = triage(path, &kinds);
                assert_eq!(
                    modes(GoalOrigin::Goal(Some(&info))),
                    [Ok(DesignMode::Off), Err(refusal(what)), Ok(DesignMode::Off)],
                    "{path:?} {kinds:?}"
                );
            }
        }
    }
    // A fast research goal is named by its path, the table's earlier row.
    let info = triage(RunPath::Fast, &[TaskKind::Research]);
    let full = config(DesignMode::Full);
    assert_eq!(
        mode_for(GoalOrigin::Goal(Some(&info)), Some(DesignMode::Full), &full),
        Err(refusal("fast"))
    );
}

/// Row 4: `run promote` of a fast-path run is off (work has started); no promotion
/// asks for the flow, so a request for it has no wording of its own and is off.
#[test]
fn mode_for_follows_the_table_a_promotion_is_off() {
    let full = config(DesignMode::Full);
    assert_eq!(
        mode_for(GoalOrigin::Promotion, None, &full),
        Ok(DesignMode::Off)
    );
    assert_eq!(
        mode_for(GoalOrigin::Promotion, Some(DesignMode::Off), &full),
        Ok(DesignMode::Off)
    );
    assert_eq!(
        mode_for(GoalOrigin::Promotion, Some(DesignMode::Full), &full),
        Err(refusal("fast"))
    );
}

/// Row 5: a run started from a `--plan` file is off (the plan already exists).
#[test]
fn mode_for_follows_the_table_a_plan_file_is_off() {
    assert_eq!(
        modes(GoalOrigin::PlanFile),
        [
            Ok(DesignMode::Off),
            Err(refusal("a plan file")),
            Ok(DesignMode::Off)
        ]
    );
}

/// The config default applies only to goals the table puts on, and only without a
/// request; a request wins over it either way.
#[test]
fn mode_for_follows_the_table_the_config_default_is_last() {
    let off = config(DesignMode::Off);
    let info = triage(RunPath::Plan, &[TaskKind::Code]);
    let goal = GoalOrigin::Goal(Some(&info));
    assert_eq!(mode_for(goal, None, &off), Ok(DesignMode::Off));
    assert_eq!(
        mode_for(goal, Some(DesignMode::Full), &off),
        Ok(DesignMode::Full)
    );
    assert_eq!(
        mode_for(GoalOrigin::Goal(None), None, &off),
        Ok(DesignMode::Off)
    );
    // A goal the table puts off stays off under a `full` default.
    let fast = triage(RunPath::Fast, &[TaskKind::Docs]);
    let full = config(DesignMode::Full);
    assert_eq!(
        mode_for(GoalOrigin::Goal(Some(&fast)), None, &full),
        Ok(DesignMode::Off)
    );
}
