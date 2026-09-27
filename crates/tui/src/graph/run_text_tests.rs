//! M8c.5: the run view's node text (Interfaces "Node content", decisions 17 and 18).

use super::*;
use crate::tree::display_rounds;
use crate::tree::run_fixtures::{PROJECT, RUN_ID, planner, reviewer, run, task, worker};
use proto::{RunState, Runtime, Size, TaskState};
use unicode_width::UnicodeWidthStr;

fn with_deps(mut info: TaskInfo, deps: &[&str]) -> TaskInfo {
    info.deps = deps.iter().map(|dep| (*dep).to_owned()).collect();
    info
}

#[test]
fn task_text_max_is_the_widest_box_less_borders_padding_and_glyph() {
    assert_eq!(TASK_TEXT_MAX, 24);
}

#[test]
fn task_text_keeps_its_tail_when_the_title_is_long() {
    let info = with_deps(
        task(
            "t7",
            "map Gemini hook events to status",
            Size::M,
            TaskState::Working,
        ),
        &["t0", "t6"],
    );
    assert_eq!(task_text(&info), "t7 map Gemini … M  ⇠t0t6");
}

#[test]
fn task_text_with_a_hub_mark() {
    let mut info = task("t0", "proto", Size::M, TaskState::Merged);
    info.hub = true;
    assert_eq!(task_text(&info), "t0 proto M ◆");
}

#[test]
fn task_text_drops_the_title_before_the_deps() {
    let deps: Vec<String> = (0..10).map(|index| format!("d{index}")).collect();
    let deps: Vec<&str> = deps.iter().map(String::as_str).collect();
    let info = with_deps(
        task("t1", "a title that has to go", Size::S, TaskState::Queued),
        &deps,
    );
    assert_eq!(task_text(&info), "t1 S  ⇠d0d1d2d3d4d5d6d7…");
}

#[test]
fn task_text_short_title_and_deps() {
    let info = with_deps(task("t2", "status", Size::S, TaskState::Queued), &["t0"]);
    assert_eq!(task_text(&info), "t2 status S  ⇠t0");
    let info = task("t1", "spawn", Size::L, TaskState::Working);
    assert_eq!(task_text(&info), "t1 spawn L");
}

/// Only declared deps are listed; implicit ones are the engine's inference.
#[test]
fn task_text_lists_declared_deps_only() {
    let mut info = with_deps(task("t3", "x", Size::S, TaskState::Queued), &["t0"]);
    info.implicit_deps = vec!["t9".into()];
    assert_eq!(task_text(&info), "t3 x S  ⇠t0");
}

/// The fitting boundary: `width(id) + 1 + width(tail) + 2 ≤ 24` keeps a title,
/// one column more drops it.
#[test]
fn task_text_keeps_a_title_exactly_at_the_boundary() {
    // The tail " S  ⇠" + 14 columns of deps is 19: 2 + 1 + 19 + 2 = 24, which
    // leaves the title two columns.
    let info = with_deps(
        task("t1", "abcdef", Size::S, TaskState::Queued),
        &["a0", "a1", "a2", "a3", "a4", "a5", "ab"],
    );
    assert_eq!(task_text(&info), "t1 a… S  ⇠a0a1a2a3a4a5ab");
    // One more column of deps and the title goes.
    let info = with_deps(
        task("t1", "abcdef", Size::S, TaskState::Queued),
        &["a0", "a1", "a2", "a3", "a4", "a5", "abc"],
    );
    assert_eq!(task_text(&info), "t1 S  ⇠a0a1a2a3a4a5abc");
}

/// Hostile: an empty title leaves no double space behind.
#[test]
fn task_text_with_an_empty_title() {
    let info = with_deps(task("t1", "", Size::S, TaskState::Queued), &["t0"]);
    assert_eq!(task_text(&info), "t1 S  ⇠t0");
    let info = task("t1", "", Size::M, TaskState::Queued);
    assert_eq!(task_text(&info), "t1 M");
}

/// A kept title is trimmed, and an all-blank one counts as empty.
#[test]
fn task_text_trims_the_title() {
    let info = task("t1", "  padded title \t", Size::S, TaskState::Queued);
    assert_eq!(task_text(&info), "t1 padded title S");
    let info = task("t1", "   ", Size::M, TaskState::Queued);
    assert_eq!(task_text(&info), "t1 M");
}

/// Hostile: wide CJK and emoji titles are cut by display width, never past 24.
#[test]
fn task_text_cuts_wide_titles_by_display_width() {
    // Budget for the title: 24 − 2 − 1 − width(" M") = 19 columns.
    let info = task("t1", "漢字漢字漢字漢字漢字漢字", Size::M, TaskState::Queued);
    let text = task_text(&info);
    assert_eq!(text, "t1 漢字漢字漢字漢字漢… M");
    assert_eq!(UnicodeWidthStr::width(text.as_str()), 24);

    let info = task("t1", "😀😀😀😀😀😀😀😀😀😀", Size::S, TaskState::Queued);
    let text = task_text(&info);
    assert_eq!(text, "t1 😀😀😀😀😀😀😀😀😀… S");
    assert_eq!(UnicodeWidthStr::width(text.as_str()), 24);

    // A wide character that straddles the boundary is dropped, not split: the
    // title's budget is 14, so 13 columns before the ellipsis hold six
    // characters and the text is one column short of 24.
    let info = with_deps(
        task("t1", "漢字漢字漢字漢字", Size::S, TaskState::Queued),
        &["t0"],
    );
    let text = task_text(&info);
    assert_eq!(text, "t1 漢字漢字漢字… S  ⇠t0");
    assert_eq!(UnicodeWidthStr::width(text.as_str()), 23);
}

/// Hostile: an id longer than the box and fifty deps still fit 24 columns.
#[test]
fn task_text_with_a_huge_id_or_fifty_deps_still_fits() {
    let id = "t-an-extremely-long-task-identifier-0123456789";
    let info = task(id, "title", Size::M, TaskState::Queued);
    let text = task_text(&info);
    assert_eq!(text, "t-an-extremely-long-tas…");
    assert_eq!(UnicodeWidthStr::width(text.as_str()), TASK_TEXT_MAX);

    let deps: Vec<String> = (0..50).map(|index| format!("t{index}")).collect();
    let deps: Vec<&str> = deps.iter().map(String::as_str).collect();
    let info = with_deps(task("t99", "fifty", Size::S, TaskState::Queued), &deps);
    let text = task_text(&info);
    assert_eq!(text, "t99 S  ⇠t0t1t2t3t4t5t6t…");
    assert_eq!(UnicodeWidthStr::width(text.as_str()), TASK_TEXT_MAX);
}

#[test]
fn round_text_names_the_round_and_its_runtime() {
    let mut info = task("t1", "x", Size::S, TaskState::Working);
    let mut bounced = worker(1, None, Runtime::Codex, 100);
    bounced.sent_back_at = vec![300];
    info.rounds = vec![bounced, reviewer(1, None, Runtime::Codex, 200)];
    let rounds = display_rounds(&info, &[]);
    let texts: Vec<String> = rounds.iter().map(round_text).collect();
    assert_eq!(
        texts,
        ["worker #1 codex", "review #1 codex", "worker #1 r2 codex"]
    );

    let mut info = task("t1", "x", Size::S, TaskState::Working);
    info.rounds = vec![worker(2, None, Runtime::Claude, 100)];
    let rounds = display_rounds(&info, &[]);
    assert_eq!(round_text(&rounds[0]), "worker #2 claude");
}

#[test]
fn run_text_with_and_without_an_orchestrator() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    info.tasks = vec![
        task("t0", "a", Size::S, TaskState::Merged),
        task("t1", "b", Size::S, TaskState::Working),
        task("t2", "c", Size::S, TaskState::Queued),
        task("t3", "d", Size::S, TaskState::Cancelled),
    ];
    assert_eq!(run_text(&info, true), "orchestrator  1/3");
    assert_eq!(run_text(&info, false), "run 3f9a  1/3");
}

#[test]
fn planner_text_counts_its_own_tasks() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mine = |id: &str, state| {
        let mut owned = task(id, "x", Size::S, state);
        owned.epic = Some("A".into());
        owned
    };
    info.tasks = vec![
        mine("t1", TaskState::Merged),
        mine("t2", TaskState::Working),
        mine("t3", TaskState::Queued),
        mine("t4", TaskState::Cancelled),
        task("t5", "root", Size::S, TaskState::Merged),
    ];
    assert_eq!(
        planner_text(&info, &planner("A", "daemon")),
        "planner A daemon  1/3"
    );
    assert_eq!(planner_text(&info, &planner("A", "")), "planner A  1/3");
    assert_eq!(
        planner_text(&info, &planner("B", "none")),
        "planner B none  0/0"
    );
}
