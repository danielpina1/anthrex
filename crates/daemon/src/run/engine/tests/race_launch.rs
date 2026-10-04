//! Milestone 9.5 task M9.5.18 (ruling T1-3, decision 19): a racer's first turn is a
//! worker's prompt for its lane's checkout and branch, and says nothing else of the
//! race.

use proto::{LaneState, RaceLane, TaskState};

use super::fixture::*;
use super::race::{RACING, all_ops, lane_branch, lane_path, racing};
use crate::run::contract::worker_prompt;
use crate::run::engine::{OpKind, OpResult};
use crate::run::model::OpId;

/// The first turn of the `CreateWindow` named `<h4>/<stem>`.
fn first_turn_of(fx: &Fixture, stem: &str) -> String {
    let name = format!("{H4}/{stem}");
    (all_ops(fx).into_iter())
        .find_map(|kind| match kind {
            OpKind::CreateWindow {
                name: n,
                first_turn,
                ..
            } if n == name => Some(first_turn),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no window {name}"))
}

#[test]
fn a_racers_prompt_names_its_lanes_checkout_and_branch() {
    let (fx, _, _) = racing();
    // The same task unraced, outside any lane's view: its own checkout and branch.
    let unraced = worker_prompt(fx.run(), fx.task("t1"), "", "");
    let own = (
        format!("Worktree: {}", fx.task("t1").worktree.display()),
        format!("Branch: {}", fx.task("t1").branch),
    );
    assert!(unraced.lines().any(|l| l == own.0), "{unraced}");
    for (lane, stem) in [(RaceLane::A, "t1.aw1"), (RaceLane::B, "t1.bw2")] {
        let prompt = first_turn_of(&fx, stem);
        let lines: Vec<&str> = prompt.lines().collect();
        let theirs = (
            format!("Worktree: {}", lane_path(lane).display()),
            format!("Branch: {}", lane_branch(lane)),
        );
        assert!(lines.contains(&theirs.0.as_str()), "{lane:?}: {prompt}");
        assert!(lines.contains(&theirs.1.as_str()), "{lane:?}: {prompt}");
        // Every other line is the unraced worker's: it is not told it is racing.
        let swapped: Vec<String> = (unraced.lines())
            .map(|l| match l {
                l if l == own.0 => theirs.0.clone(),
                l if l == own.1 => theirs.1.clone(),
                l => l.to_string(),
            })
            .collect();
        // Task 18 review m5: matched line for line, never by a substring such as
        // `race`, which an unrelated word (`trace`) would trip.
        assert_eq!(lines, swapped, "{lane:?}");
    }
}

/// Task 18 review m2: a racer whose launch the driver refuses (`ops::worker_git_dirs`:
/// its lane is not stored) fails its `CreateWindow`; that lane goes out and the other
/// races on.
#[test]
fn a_racer_refused_its_launch_puts_its_lane_out() {
    let mut fx = Fixture::with_config(
        &plan_with(PROFILE, &[task("t1", "M", "a", RACING)]),
        config::Orchestrator::default(),
    );
    fx.ready(true);
    fx.complete_prepares();
    let windows: Vec<(OpId, Option<RaceLane>)> = (fx.run().pending_ops.values())
        .filter(|p| matches!(p.kind, OpKind::CreateWindow { .. }))
        .map(|p| (p.op, p.lane))
        .collect();
    assert_eq!(windows.len(), 2, "{windows:?}");
    for (op, lane) in windows {
        let result = match lane {
            Some(RaceLane::B) => OpResult::Failed {
                message: "task t1 has no lane b; its session gets no sandbox roots".into(),
            },
            _ => OpResult::Window {
                window_id: 70,
                pid: None,
            },
        };
        fx.done(op, result);
    }
    let lanes = fx.task("t1").race.as_ref().expect("a race").lanes.clone();
    let state = |l: RaceLane| lanes.iter().find(|x| x.lane == l).map(|x| x.state);
    assert_eq!(state(RaceLane::B), Some(LaneState::Out), "{lanes:?}");
    assert_eq!(state(RaceLane::A), Some(LaneState::Working), "{lanes:?}");
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert!(
        (fx.run().log.iter())
            .any(|l| l.text.contains("racer b out") && l.text.contains("no sandbox")),
        "{:?}",
        fx.run().log
    );
}
