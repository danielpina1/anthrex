//! M9.0.5.10: the sections panel (decisions 22, 25 and 26).

use super::*;
use crate::app::task_detail::{DetailState, TaskDetailCache, detail_key};
use crate::inspector::run_tests::{app_of, inspect_node, value};
use crate::inspector::{
    INSPECTOR_HEIGHT, MIN_INTERIOR_FOR_TALL_RUN_PANEL, RUN_INSPECTOR_HEIGHT,
    RUN_INSPECTOR_TALL_HEIGHT,
};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::gemini_fixture;
use proto::{AgentRole, TaskDetailInfo};
use ratatui::layout::Rect;

fn t2_key() -> NodeKey {
    NodeKey::Task {
        run: "r1".into(),
        id: "t2".into(),
    }
}

fn text(lines: &[Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

/// The Gemini fixture with `t2`'s detail landed: a brief of `brief_lines` lines.
fn t2_with_brief(brief_lines: usize) -> crate::app::App {
    let mut app = app_of(gemini_fixture());
    let brief: Vec<String> = (1..=brief_lines).map(|n| format!("brief {n}")).collect();
    let task = app.runs.runs[0]
        .tasks
        .iter()
        .find(|t| t.id == "t2")
        .unwrap();
    app.task_detail = Some(TaskDetailCache {
        run_id: "r1".into(),
        task_id: "t2".into(),
        key: detail_key(task),
        state: DetailState::Ready(Box::new(TaskDetailInfo {
            run_id: "r1".into(),
            task_id: "t2".into(),
            brief: brief.join("\n"),
            acceptance: Vec::new(),
            worker_summary: None,
            summary_source: None,
        })),
    });
    app
}

#[test]
fn the_collapsed_brief_ends_with_more_in_muted() {
    let app = t2_with_brief(5);
    let inspection = inspect_node(&app, &t2_key());
    let body = body_lines(&inspection.sections, 76);
    let rows = text(&body);
    assert_eq!(
        rows[1..5],
        [
            "brief     brief 1",
            "          brief 2",
            "          brief 3",
            "          … (b: more)",
        ]
    );
    let more = body[4].spans.last().unwrap();
    assert_eq!(more.content, MORE);
    assert_eq!(more.style, theme::muted());
    assert!(
        body[0].spans[0].style.add_modifier.contains(Modifier::BOLD),
        "GOAL is bold"
    );
}

#[test]
fn task_panel_scrolls() {
    let mut app = t2_with_brief(12);
    app.brief_expanded = Some(t2_key());
    let mut inspection = inspect_node(&app, &t2_key());
    let body = text(&body_lines(&inspection.sections, 76));
    let height = 10;
    let top = text(&lines(&inspection, 76, height));
    assert_eq!(top.len(), height);
    assert!(
        top[0].starts_with("◐ t2  map Gemini"),
        "the title row stays: {top:?}"
    );
    assert_eq!(top[1..], body[..height - 1]);
    inspection.scroll = 5;
    let scrolled = text(&lines(&inspection, 76, height));
    assert!(scrolled[0].starts_with("◐ t2"), "the title row stays");
    assert_eq!(scrolled[1..], body[5..5 + height - 1]);
    // Past the end: clamped so the last body row is the last panel row.
    inspection.scroll = u16::MAX;
    let end = text(&lines(&inspection, 76, height));
    assert_eq!(end[1..], body[body.len() - (height - 1)..]);
    assert_eq!(end.last(), body.last());
    // `task_panel_rows` counts exactly those body rows.
    assert_eq!(
        crate::inspector::task_panel_rows(&app, 76),
        0,
        "no selection yet"
    );
}

#[test]
fn every_row_fits_the_width() {
    let mut app = t2_with_brief(3);
    app.brief_expanded = Some(t2_key());
    let inspection = inspect_node(&app, &t2_key());
    for width in [1, 9, 10, 11, 30, 76] {
        // The title row is M8c's (`rows::title`); the body is this renderer's.
        for line in body_lines(&inspection.sections, width) {
            let row: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                unicode_width::UnicodeWidthStr::width(row.as_str()) <= width,
                "{width}: {row:?}"
            );
        }
    }
    assert!(body_lines(&inspection.sections, 0).is_empty());
}

/// Pinning: M8c's 8 and 12 row steps are unchanged below 37 rows; the tall step
/// starts at an interior of 34 (a 37-row terminal), and only in the run view.
#[test]
fn m8c_panel_heights_are_unchanged_below_37_rows() {
    let footer = |interior: u16, run_view: bool| {
        let main = Rect::new(0, 0, 120, interior + 2);
        crate::ui::overview::areas(main, true, run_view).1.height
    };
    assert_eq!(MIN_INTERIOR_FOR_TALL_RUN_PANEL, 34);
    assert_eq!(RUN_INSPECTOR_TALL_HEIGHT, 18);
    assert_eq!(footer(17, true), INSPECTOR_HEIGHT);
    assert_eq!(footer(18, true), RUN_INSPECTOR_HEIGHT);
    assert_eq!(footer(33, true), RUN_INSPECTOR_HEIGHT);
    assert_eq!(footer(34, true), RUN_INSPECTOR_TALL_HEIGHT);
    assert_eq!(footer(60, true), RUN_INSPECTOR_TALL_HEIGHT);
    assert_eq!(
        footer(34, false),
        INSPECTOR_HEIGHT,
        "the tree keeps its panel"
    );
}

/// Decision 26: the live round's `doing` reads `now: <activity>`.
#[test]
fn round_doing_reads_now() {
    let (mut snapshot, windows) = gemini_fixture();
    let task = snapshot.runs[0]
        .tasks
        .iter_mut()
        .find(|t| t.id == "t2")
        .unwrap();
    task.activity = Some("Read src/\u{1b}[2Jhooks.rs".into());
    // Only the task's live round reads it: take the reviewers' rounds away so the
    // worker's second display round (after its send-back) is the live one.
    task.rounds.truncate(1);
    task.reviews.clear();
    assert_eq!(task.rounds[0].ended_at, None);
    let (mut other, _) = gemini_fixture();
    let app = app_of((snapshot, windows));
    let round = |role, session, round| NodeKey::AgentRound {
        run: "r1".into(),
        task: "t2".into(),
        role,
        session,
        round,
    };
    let doing = value(
        &inspect_node(&app, &round(AgentRole::Worker, 1, 2)),
        "doing",
    )
    .expect("a doing field")
    .to_owned();
    assert!(doing.starts_with("now: Read src/"), "{doing:?}");
    assert!(!doing.contains('\u{1b}'), "{doing:?}");
    // The display round the send-back ended keeps its old text.
    assert_eq!(
        value(
            &inspect_node(&app, &round(AgentRole::Worker, 1, 1)),
            "doing"
        ),
        Some("finished")
    );
    // With a live review, the worker's round does not take the reviewer's activity.
    let task = other.runs[0]
        .tasks
        .iter_mut()
        .find(|t| t.id == "t2")
        .unwrap();
    task.activity = Some("Read review target".into());
    let app = app_of((other, Vec::new()));
    let inspection = inspect_node(&app, &round(AgentRole::Worker, 1, 2));
    let doing = value(&inspection, "doing");
    assert!(!doing.unwrap_or("").contains("review target"), "{doing:?}");
    let first_review = round(AgentRole::Reviewer, 1, 1);
    assert_eq!(
        value(&inspect_node(&app, &first_review), "doing"),
        None,
        "a review round has no doing"
    );
}
