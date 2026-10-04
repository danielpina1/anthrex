//! Milestone 9.5 task M9.5.18 (ruling T1-3, decision 19): a racer's first turn is a
//! worker's prompt for its lane's checkout and branch, and says nothing else of the
//! race.

use proto::RaceLane;

use super::fixture::*;
use super::race::{all_ops, lane_branch, lane_path, racing};
use crate::run::contract::worker_prompt;
use crate::run::engine::OpKind;

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
        assert_eq!(lines, swapped, "{lane:?}");
        assert!(!prompt.to_lowercase().contains("race"), "{prompt}");
    }
}
