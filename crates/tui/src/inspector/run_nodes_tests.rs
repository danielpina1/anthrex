//! M8c.7: the sub-planner and scout projections, the shared number formatting, and a
//! sub-agent in the run view (decisions 29–31, Interfaces "Inspector contents, exact").

use super::run_tests::{app_of, inspect_node, pairs, value};
use crate::inspector::{FieldLayout, format_duration, format_tokens, progress_bar};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{GEMINI_NOW, gemini_fixture, planner_fixture, three_task_fixture};
use proto::{PlannerState, SubagentInfo, SubagentState};

#[test]
fn planner_fields_match_the_mockup() {
    let app = app_of(planner_fixture());
    let key = NodeKey::Planner {
        run: "r2".into(),
        epic: "A".into(),
    };
    let inspection = inspect_node(&app, &key);
    assert_eq!(inspection.glyph.content, "✓");
    assert_eq!(inspection.name, "planner A  daemon");
    assert_eq!(
        inspection.right.as_deref(),
        Some("finished · planned 3 tasks in 2m")
    );
    assert_eq!(inspection.layout, FieldLayout::Rows);
    assert_eq!(
        pairs(&inspection),
        [
            ("progress", "███░░░░░░░  1/3 merged · 1 working · 1 waiting"),
            ("area", "crates/daemon/** · crates/cli/src/hook.rs"),
            (
                "edits",
                "3 accepted · 1 rejected (owns outside area) · re-planned once (t2 split)"
            ),
        ]
    );
}

#[test]
fn planner_while_planning() {
    let (mut snapshot, windows) = planner_fixture();
    let planner = &mut snapshot.runs[0].planners[0];
    planner.state = PlannerState::Planning;
    planner.started_at = GEMINI_NOW - 60;
    planner.ended_at = None;
    planner.last_rejection = None;
    planner.replans = vec!["t1 split".into(), "t3 merged".into()];
    let app = app_of((snapshot, windows));
    let key = NodeKey::Planner {
        run: "r2".into(),
        epic: "A".into(),
    };
    let inspection = inspect_node(&app, &key);
    assert_eq!(inspection.right.as_deref(), Some("planning · 1m"));
    assert_eq!(
        value(&inspection, "edits"),
        Some("3 accepted · 1 rejected · re-planned twice (t1 split, t3 merged)")
    );

    let (mut snapshot, windows) = planner_fixture();
    let planner = &mut snapshot.runs[0].planners[0];
    planner.state = PlannerState::Failed;
    planner.replans = vec!["a".into(), "b".into(), "c".into()];
    snapshot.runs[0].tasks.truncate(1);
    let app = app_of((snapshot, windows));
    let inspection = inspect_node(&app, &key);
    assert_eq!(inspection.glyph.content, "✗");
    assert_eq!(inspection.right.as_deref(), Some("failed · 2m"));
    assert_eq!(
        value(&inspection, "edits"),
        Some("3 accepted · 1 rejected (owns outside area) · re-planned 3 times (a, b, c)")
    );
}

#[test]
fn a_finished_planner_of_one_task_says_task() {
    let (mut snapshot, windows) = planner_fixture();
    snapshot.runs[0].tasks.truncate(1);
    let app = app_of((snapshot, windows));
    let key = NodeKey::Planner {
        run: "r2".into(),
        epic: "A".into(),
    };
    let inspection = inspect_node(&app, &key);
    assert_eq!(
        inspection.right.as_deref(),
        Some("finished · planned 1 task in 2m")
    );
    assert_eq!(
        value(&inspection, "progress"),
        Some("██████████  1/1 merged")
    );
}

#[test]
fn scout_fields_match_the_mockup() {
    let app = app_of(gemini_fixture());
    let key = NodeKey::Scout {
        run: "r1".into(),
        id: "S1".into(),
    };
    let inspection = inspect_node(&app, &key);
    assert_eq!(inspection.glyph.content, "✓");
    assert_eq!(inspection.name, "scout S1");
    assert_eq!(inspection.right.as_deref(), Some("reported · 3m"));
    assert_eq!(inspection.layout, FieldLayout::Rows);
    assert_eq!(
        pairs(&inspection),
        [
            (
                "question",
                "where are Claude hook events parsed, and which of them fire in -p mode?"
            ),
            ("state", "reported"),
            ("took", "3m"),
            ("report", "1840 bytes"),
            (
                "files",
                "crates/daemon/src/hooks.rs, crates/daemon/src/status.rs"
            ),
            ("session", "#4 · window closed"),
        ]
    );
    let wrapping: Vec<&str> = inspection
        .fields
        .iter()
        .filter(|field| field.wrap)
        .map(|field| field.label)
        .collect();
    assert_eq!(wrapping, ["question"]);
}

#[test]
fn a_live_scout_on_a_listed_window() {
    let (mut snapshot, mut windows) = gemini_fixture();
    let scout = &mut snapshot.runs[0].scouts[0];
    scout.state = proto::ScoutState::Working;
    scout.ended_at = None;
    scout.report_bytes = None;
    scout.files = vec![];
    let mut window = crate::tree::run_fixtures::headless(4, "r1/S1", "/r/anthrex", None);
    window.tool = Some("Grep".into());
    windows.push(window);
    let app = app_of((snapshot, windows));
    let key = NodeKey::Scout {
        run: "r1".into(),
        id: "S1".into(),
    };
    let inspection = inspect_node(&app, &key);
    // Its glyph is the canvas's: a static `●` while its listed window is `Idle`.
    assert_eq!(inspection.glyph.content, "●");
    assert_eq!(inspection.right.as_deref(), Some("working · 1h05m"));
    assert_eq!(value(&inspection, "state"), Some("working · Grep"));
    assert_eq!(value(&inspection, "report"), Some("not yet"));
    assert_eq!(value(&inspection, "files"), None);
    assert_eq!(
        value(&inspection, "session"),
        Some("#4 · headless · main checkout, read-only · Enter: conversation")
    );

    // On a `Working` window, it spins.
    let (mut snapshot, mut windows) = gemini_fixture();
    let scout = &mut snapshot.runs[0].scouts[0];
    scout.state = proto::ScoutState::Working;
    scout.ended_at = None;
    let mut window = crate::tree::run_fixtures::headless(4, "r1/S1", "/r/anthrex", None);
    window.status = proto::Status::Working;
    windows.push(window);
    let app = app_of((snapshot, windows));
    assert_eq!(inspect_node(&app, &key).glyph.content, "⠋");

    let (mut snapshot, windows) = gemini_fixture();
    let scout = &mut snapshot.runs[0].scouts[0];
    scout.state = proto::ScoutState::Failed;
    scout.failure = Some("no report".into());
    scout.window_id = None;
    let app = app_of((snapshot, windows));
    let inspection = inspect_node(&app, &key);
    assert_eq!(inspection.right.as_deref(), Some("failed · 3m"));
    assert_eq!(value(&inspection, "state"), Some("failed: no report"));
    assert_eq!(value(&inspection, "session"), Some("no window yet"));
}

#[test]
fn format_duration_cases() {
    let cases = [
        (0, "0s"),
        (59, "59s"),
        (60, "1m"),
        (3599, "59m"),
        (3600, "1h00m"),
        (4320, "1h12m"),
        (26 * 3600 + 300, "26h05m"),
    ];
    for (secs, text) in cases {
        assert_eq!(format_duration(secs), text, "{secs}");
    }
    assert!(format_duration(u64::MAX).ends_with('m'));
}

#[test]
fn format_tokens_cases() {
    let cases = [
        (0, "0"),
        (999, "999"),
        (1000, "1k"),
        (410_000, "410k"),
        (999_999, "999k"),
        (1_000_000, "1.0M"),
        (1_800_000, "1.8M"),
        (12_345_678, "12.3M"),
        (u64::MAX, "18446744073709.5M"),
    ];
    for (n, text) in cases {
        assert_eq!(format_tokens(n), text, "{n}");
    }
}

#[test]
fn progress_bar_cases() {
    assert_eq!(progress_bar(5, 9, 18), "██████████░░░░░░░░");
    assert_eq!(progress_bar(1, 3, 10), "███░░░░░░░");
    assert_eq!(progress_bar(9, 9, 18), "█".repeat(18));
    assert_eq!(progress_bar(0, 9, 10), "░".repeat(10));
    // Rounding at the half, and never more than the width.
    assert_eq!(progress_bar(1, 20, 10), "█░░░░░░░░░");
    assert_eq!(progress_bar(12, 9, 10), "█".repeat(10));
    assert_eq!(progress_bar(u64::MAX, u64::MAX, 10), "█".repeat(10));
    assert_eq!(progress_bar(0, 0, 4), "░░░░");
}

#[test]
fn a_subagent_in_the_run_view_keeps_the_column_layout() {
    let (snapshot, mut windows) = three_task_fixture();
    windows[2].subagents = vec![SubagentInfo {
        id: "a1".into(),
        parent_id: None,
        kind: "Explore".into(),
        label: Some("find the spawn code".into()),
        model: None,
        state: SubagentState::Running,
        tool: None,
        started_secs: 0,
        ended_secs: None,
        needs_permission: false,
    }];
    let app = app_of((snapshot, windows));
    let key = NodeKey::Subagent {
        window_id: 6,
        id: "a1".into(),
    };
    let inspection = inspect_node(&app, &key);
    assert_eq!(inspection.layout, FieldLayout::Columns);
    assert_eq!(inspection.right, None);
    assert_eq!(inspection.name, "find the spawn code");
    let labels: Vec<&str> = inspection.fields.iter().map(|field| field.label).collect();
    assert_eq!(
        labels,
        [
            "task",
            "spawned by",
            "state",
            "for",
            "model",
            "kind",
            "depth"
        ]
    );
}

#[test]
fn a_reported_scout_whose_window_lingers_shows_no_tool() {
    let (snapshot, mut windows) = gemini_fixture();
    let mut window = crate::tree::run_fixtures::headless(4, "r1/S1", "/r/anthrex", None);
    window.tool = Some("Grep".into());
    window.status = proto::Status::Exited;
    windows.push(window);
    let app = app_of((snapshot, windows));
    let key = NodeKey::Scout {
        run: "r1".into(),
        id: "S1".into(),
    };
    let inspection = inspect_node(&app, &key);
    assert_eq!(value(&inspection, "state"), Some("reported"));
    assert_eq!(
        value(&inspection, "session"),
        Some("#4 · headless · main checkout, read-only · Enter: conversation")
    );
}

/// `clean` keeps exactly 300 characters whole and cuts the 301st with `…`, counting
/// characters, not bytes.
#[test]
fn clean_cuts_past_300_characters() {
    use super::run_format::clean;
    let exact = "é".repeat(300);
    assert_eq!(clean(&exact), exact);
    let over = format!("{exact}x");
    assert_eq!(clean(&over), format!("{exact}…"));
    assert_eq!(clean("a\u{1b}[2Jb\nc\u{7}"), "a [2Jb c ");
}

/// Review M8c.8 m2: a planner's epic reaches the title cleaned, like every other text.
#[test]
fn a_hostile_epic_is_cleaned_in_the_name() {
    let (mut snapshot, windows) = planner_fixture();
    snapshot.runs[0].planners[0].epic = "A\u{1b}[31m\n".into();
    let app = app_of((snapshot, windows));
    let key = NodeKey::Planner {
        run: "r2".into(),
        epic: "A\u{1b}[31m\n".into(),
    };
    let inspection = inspect_node(&app, &key);
    assert!(
        !inspection.name.chars().any(char::is_control),
        "{:?}",
        inspection.name
    );
}
