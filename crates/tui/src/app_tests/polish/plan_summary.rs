//! M9.0.7.11: the plan review's summary facts (decisions 23 and 24), pure.

use crate::app::ReviewTarget;
use crate::app::plan_review::review_tasks;
use crate::app::plan_summary::{Overlap, columns, header_line, overlap_lines, overlaps};
use crate::tree::plan_fixtures::{plan_task, three_task_plan};
use proto::Size;

#[test]
fn the_header_counts_sizes_budget_and_the_critical_path() {
    // t1 S (40 calls) stage 1; t2 M (100) stage 2 after t1; t3 S (50) stage 2 after t2;
    // critical_path [t1, t2, t3].
    let run = three_task_plan();
    assert_eq!(
        header_line(&run, &review_tasks(&run, &ReviewTarget::Gate), 200, false),
        "3 tasks · 2 stages · S+M+S · ~190 calls · critical t1 › t2 › t3"
    );
    // Cut with `…` to the width, the critical path first; folded in ASCII.
    let tasks = review_tasks(&run, &ReviewTarget::Gate);
    assert_eq!(
        header_line(&run, &tasks, 50, false),
        "3 tasks · 2 stages · S+M+S · ~190 calls · critica…"
    );
    assert_eq!(
        header_line(&run, &tasks, 200, true),
        "3 tasks - 2 stages - S+M+S - ~190 calls - critical t1 > t2 > t3"
    );
    // One stage, no critical path: neither part shows.
    let mut single = three_task_plan();
    for task in &mut single.tasks {
        task.stage = 1;
    }
    single.critical_path.clear();
    let tasks = review_tasks(&single, &ReviewTarget::Gate);
    assert_eq!(
        header_line(&single, &tasks[..1], 200, false),
        "1 task · S · ~40 calls"
    );
}

#[test]
fn overlaps_need_owners_that_can_run_together() {
    let mut run = three_task_plan();
    run.tasks[1].owns = vec!["crates/c/src/lib.rs".into()];
    run.tasks[2].owns = vec!["crates/c/src/".into()];
    assert!(
        overlaps(&run, &run.tasks.iter().collect::<Vec<_>>()).is_empty(),
        "t3 runs after t2"
    );
    run.tasks[2].deps.clear();
    run.tasks[2].implicit_deps.clear();
    assert_eq!(
        overlaps(&run, &run.tasks.iter().collect::<Vec<_>>()),
        vec![Overlap {
            a: "t2",
            b: "t3",
            path: "crates/c/src/lib.rs"
        }]
    );
}

#[test]
fn overlaps_follow_deps_transitively_and_stop_at_a_slash() {
    let mut run = three_task_plan();
    // t3 after t2 after t1: t1 and t3 never run together, through t2.
    run.tasks[0].owns = vec!["docs/".into()];
    run.tasks[2].owns = vec!["docs/guide.md".into()];
    assert!(overlaps(&run, &run.tasks.iter().collect::<Vec<_>>()).is_empty());
    // An implicit dep orders them as well as an explicit one.
    run.tasks[2].deps.clear();
    run.tasks[2].implicit_deps = vec!["t1".into()];
    assert!(overlaps(&run, &run.tasks.iter().collect::<Vec<_>>()).is_empty());
    // `crates/c` is not a directory of `crates/cli`; `crates/c` is one of `crates/c/x`.
    run.tasks[2].implicit_deps.clear();
    run.tasks[0].owns = vec!["crates/c".into()];
    run.tasks[2].owns = vec!["crates/cli/src/main.rs".into()];
    assert!(overlaps(&run, &run.tasks.iter().collect::<Vec<_>>()).is_empty());
    run.tasks[2].owns = vec!["crates/c/x.rs".into()];
    assert_eq!(
        overlaps(&run, &run.tasks.iter().collect::<Vec<_>>()),
        vec![Overlap {
            a: "t1",
            b: "t3",
            path: "crates/c/x.rs"
        }]
    );
    // Fix round 1 (C1): a path whose byte at the shorter one's end lies inside a
    // multi-byte character neither panics nor overlaps; a directory still holds one.
    run.tasks[0].owns = vec!["src/a".into()];
    run.tasks[2].owns = vec!["src/é".into()];
    assert!(overlaps(&run, &run.tasks.iter().collect::<Vec<_>>()).is_empty());
    run.tasks[0].owns = vec!["docs/".into()];
    run.tasks[2].owns = vec!["docs/é.md".into()];
    assert_eq!(
        overlaps(&run, &run.tasks.iter().collect::<Vec<_>>()),
        vec![Overlap {
            a: "t1",
            b: "t3",
            path: "docs/é.md"
        }]
    );
}

/// Fix round 1 (m1): a hold reviews `t1` and `t3`; `t3` is after `t2` after `t1`, and
/// `t2` is outside the hold. The two never run together, so no warning.
#[test]
fn a_holds_overlaps_follow_deps_through_tasks_outside_it() {
    let mut run = three_task_plan();
    run.tasks[0].owns = vec!["src/lib.rs".into()];
    run.tasks[2].owns = vec!["src/lib.rs".into()];
    let hold = [&run.tasks[0], &run.tasks[2]];
    assert!(overlaps(&run, &hold).is_empty());
    // Once `t3` no longer waits on `t2`, the pair can run together.
    let mut free = run.clone();
    free.tasks[2].deps.clear();
    let hold = [&free.tasks[0], &free.tasks[2]];
    assert_eq!(overlaps(&free, &hold).len(), 1);
}

#[test]
fn more_than_eight_tasks_count_sizes() {
    let mut run = three_task_plan();
    run.tasks = (1..=9)
        .map(|n| {
            let size = if n % 2 == 1 { Size::S } else { Size::M };
            plan_task(&format!("t{n}"), "x", size, 1, 10)
        })
        .collect();
    run.critical_path.clear();
    let tasks = review_tasks(&run, &ReviewTarget::Gate);
    assert_eq!(
        header_line(&run, &tasks, 200, false),
        "9 tasks · 5S 4M · ~90 calls"
    );
    // Eight are still a sequence.
    assert_eq!(
        header_line(&run, &tasks[..8], 200, false),
        "8 tasks · S+M+S+M+S+M+S+M · ~80 calls"
    );
}

#[test]
fn epics_are_counted_when_present() {
    let mut run = three_task_plan();
    run.tasks[0].epic = Some("api".into());
    run.tasks[1].epic = Some("api".into());
    run.tasks[2].epic = Some("docs".into());
    let tasks = review_tasks(&run, &ReviewTarget::Gate);
    assert!(
        header_line(&run, &tasks, 200, false).starts_with("3 tasks · 2 epics · 2 stages · "),
        "{}",
        header_line(&run, &tasks, 200, false)
    );
    run.tasks[2].epic = None;
    let tasks = review_tasks(&run, &ReviewTarget::Gate);
    assert!(header_line(&run, &tasks, 200, false).starts_with("3 tasks · 1 epic · 2 stages · "));
}

#[test]
fn implied_deps_are_marked() {
    let mut run = three_task_plan();
    // t4 after t1 and, implicitly, t1 again and t3: an implicit dep that is also
    // explicit is not `(implied)`.
    let mut t4 = plan_task("t4", "release", Size::S, 2, 10);
    t4.deps = vec!["t1".into()];
    t4.implicit_deps = vec!["t1".into(), "t3".into()];
    run.tasks.push(t4);
    let tasks = review_tasks(&run, &ReviewTarget::Gate);
    let cells = columns(&run, &tasks, false);
    assert_eq!(cells[3].deps, "after t1, t3 (implied)");
    assert_eq!(cells[1].deps, "after t1");
    assert_eq!(cells[0].deps, "");
    // The other cells, for the route tag and the stage column.
    // Fix round 1: the task panel footer's `<tag> <model>`, the model whole.
    assert_eq!(cells[0].route, "cx gpt-6-sol");
    assert_eq!(cells[1].route, "cl opus");
    assert_eq!(cells[2].route, "cl haiku");
    assert_eq!(cells[2].size, "S none");
    assert_eq!(cells[1].stage.as_deref(), Some("stage 2"));
    run.tasks[0].route.model.clear();
    let tasks = review_tasks(&run, &ReviewTarget::Gate);
    assert_eq!(columns(&run, &tasks, false)[0].route, "cx default");
}

#[test]
fn more_than_three_overlaps_are_summed() {
    let mut run = three_task_plan();
    run.tasks = (1..=4)
        .map(|n| {
            let mut t = plan_task(&format!("t{n}"), "x", Size::S, 1, 10);
            t.owns = vec!["src/lib.rs".into()];
            t
        })
        .collect();
    let tasks = review_tasks(&run, &ReviewTarget::Gate);
    // Four tasks that all own the file, none ordered: six pairs.
    let all = overlaps(&run, &tasks);
    assert_eq!(all.len(), 6);
    assert_eq!(
        overlap_lines(&all, false),
        [
            "⚠ t1 and t2 both own src/lib.rs",
            "⚠ t1 and t3 both own src/lib.rs",
            "⚠ t1 and t4 both own src/lib.rs",
            "⚠ 3 more overlaps",
        ]
    );
    assert_eq!(overlap_lines(&all[..5], true)[3], "! 2 more overlaps");
    assert_eq!(overlap_lines(&all[..4], false)[3], "⚠ 1 more overlap");
    assert_eq!(overlap_lines(&all[..3], false).len(), 3);
}

/// Final fix wave I3: a 200-task plan, four chains of fifty, every task owning
/// `Cargo.toml` (15,000 overlapping pairs across the chains). The review's layout
/// orders the tasks once (a closure over an id map), not with two searches a pair: its
/// work is bounded by an operation count, not the wall clock.
#[test]
fn a_200_task_plans_layout_orders_its_tasks_once() {
    use crate::app::plan_summary::work;
    use crate::tree::run_fixtures::{RUN_ID, gate_fixture};
    use proto::{DaemonMsg, RunReply};
    let (mut snap, windows) = gate_fixture();
    let run = &mut snap.runs[0];
    run.tasks = (0..4)
        .flat_map(|c| {
            (0..50).map(move |i| {
                let mut t = plan_task(&format!("c{c}t{i}"), "x", Size::S, 1, 10);
                if i > 0 {
                    t.deps = vec![format!("c{c}t{}", i - 1)];
                }
                t.owns = std::iter::once("Cargo.toml".to_owned())
                    .chain((0..4).map(|k| format!("src/c{c}/t{i}/{k}.rs")))
                    .collect();
                t
            })
        })
        .collect();
    run.critical_path.clear();
    let mut app = crate::app::App::new(windows, "/tmp".into(), Default::default());
    let _ = app.set_terminal_size(120, 40);
    let _ = app.run_subscription();
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    let _ = app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate);
    assert!(app.plan_review.is_some());
    let _ = work::take();
    let layout = app.review_layout(ratatui::layout::Rect::new(0, 0, 120, 39));
    let steps = work::take();
    assert_eq!(layout.warnings.len(), 4);
    assert_eq!(layout.warnings[3], "⚠ 14997 more overlaps");
    // 200 tasks: a closure of 200 walks over at most 50 tasks and 50 deps each
    // (20,000), and one `owns` comparison for each of the 19,900 pairs that cannot
    // be ordered or share `Cargo.toml` first. Two searches a pair cost millions.
    assert!(steps <= 100_000, "{steps} steps");
}

/// Final fix wave (task 11's deferred minor): a full Claude model id, not an alias,
/// reads whole through the route tag, the list's route column and the ASCII detail's
/// route row.
#[test]
fn a_full_claude_id_reads_whole() {
    use crate::app::plan_review::detail_lines;
    use crate::theme::Palette;
    let mut run = three_task_plan();
    run.tasks[1].route.model = "claude-opus-5-5".into();
    let tasks = review_tasks(&run, &ReviewTarget::Gate);
    for ascii in [false, true] {
        assert_eq!(columns(&run, &tasks, ascii)[1].route, "cl claude-opus-5-5");
    }
    assert_eq!(
        crate::inspector::run_format::route_tag(&run.tasks[1].route),
        "cl claude-opus-5-5"
    );
    let p = Palette {
        ascii: true,
        ..Palette::PLAIN
    };
    let rows: Vec<String> = detail_lines(&run, &run.tasks[1], 100, p)
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    let route = rows
        .iter()
        .find(|r| r.trim_start().starts_with("route"))
        .unwrap_or_else(|| panic!("{rows:#?}"));
    assert!(route.contains("claude-opus-5-5"), "{route:?}");
    assert!(route.is_ascii(), "{route:?}");
}
