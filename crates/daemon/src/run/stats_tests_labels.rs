//! Milestone 9.19 decision 14: `anthrex run stats` labels its aggregates. Every median
//! column says it is one, a footer says over what, the bounces column says it counts
//! every task, and the rounds line names both of its numbers.

use std::path::Path;

use proto::{HistoryLine, Size};

use super::tests::{PATH, merged, run};
use super::{aggregate, render};

const HEADER: &str =
    "CLASS  TASKS  MERGED  MED LINES  MED CALLS  MED TOKENS  MED WORK  BOUNCES (ALL)  REVERTED";
const FOOTER: &str = "MED = median over merged tasks · tokens are billed (worker + reviewer + decider) · BOUNCES counts every task";

#[test]
fn run_stats_labels_its_aggregates() {
    let lines = [
        HistoryLine::Task(merged("r1", "a1", Size::S, (8, 5, 1_500, 89))),
        HistoryLine::Run(run("r1", 3, 1)),
    ];
    let mut stats = aggregate(&lines, Path::new(PATH));
    let text = render(&stats);
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows[1], HEADER, "{text}");
    // The work median is a floored `duration`: 89 s is `1m 29s`, never a rounded `1`.
    assert_eq!(
        rows[2],
        "S      1      1       8          5          1.5k        1m 29s    0              0",
        "{text}"
    );
    // The footer follows the table, before the deciders' line.
    assert_eq!(rows[5], FOOTER, "{text}");
    assert!(rows[6].starts_with("deciders: "), "{text}");
    assert!(!text.contains("rounds:"), "{text}");

    stats.rounds = 3;
    stats.iterated_runs = 1;
    assert!(
        render(&stats).contains("\nrounds: 3 ended · 1 run iterated\n"),
        "{}",
        render(&stats)
    );
    stats.rounds = 1;
    assert!(render(&stats).contains("\nrounds: 1 ended · 1 run iterated\n"));
    stats.rounds = 5;
    stats.iterated_runs = 2;
    assert!(render(&stats).contains("\nrounds: 5 ended · 2 runs iterated\n"));
}
