//! Milestone 9.1 task M9.1.20: the stage node on the run view's canvas (decision 55).

use super::*;
use crate::tree::stage_fixtures::{stage, staged_fixture, staged_gate_fixture};

fn stage_key(n: u16) -> NodeKey {
    NodeKey::Stage {
        run: RUN_ID.into(),
        n,
    }
}

#[test]
fn stage_node_is_drawn_between_run_and_tasks_for_a_multi_run() {
    let app = app_of(staged_fixture());
    let (layout, lines) = paint_view(&app);
    let width = usize::from(layout.size.0);
    assert_eq!(
        lines_text(&lines),
        padded(
            &[
                "                                                  ╭──────────────────────────╮",
                "                                                ┌─┤ ✓ t1 reset model S       │",
                "                                                │ ╰──────────────────────────╯",
                "                                                │",
                "                      ╭───────────────────────╮ │ ╭──────────────────────────╮",
                "                    ┌─┤ ⠋ stage 1/2  tier 3 … ├─┼─┤ ● t2 reset endpoint S    │",
                "                    │ ╰───────────────────────╯ │ ╰──────────────────────────╯",
                "                    │                           │",
                "╭─────────────────╮ │                           │ ╭──────────────────────────╮",
                "│ ◉ run 3f9a  1/4 ├─┤                           └─┤ ▫ fix1 fix t2 S (bisect) │",
                "╰─────────────────╯ │                             ╰──────────────────────────╯",
                "                    │",
                "                    │ ╭───────────────────────╮   ╭──────────────────────────╮",
                "                    └─┤ ◌ stage 2/2  tier 3 · ├───┤ ◌ t3 reset view S        │",
                "                      ╰───────────────────────╯   ╰──────────────────────────╯",
            ],
            width
        )
    );
    // A stage not created yet is `◌` in the starting colour, never a created stage's.
    let (glyph, style) = glyph_of(&lines, rect_of(&layout, &stage_key(2)));
    assert_eq!(glyph, "◌");
    assert_eq!(style.fg, Some(theme::status_color(proto::Status::Starting)));
}

/// Each tier-3 state's mark and glyph; a stage with no head is `◌` whatever it says.
#[test]
fn each_tier3_state_has_its_mark_and_glyph() {
    use proto::FullState;
    for (state, mark, glyph) in [
        (FullState::Green, "✓", "✓"),
        (FullState::Red, "✗", "✗"),
        (FullState::Running, "…", "⠋"),
        (FullState::None, "·", "○"),
    ] {
        let (mut snap, windows) = staged_fixture();
        snap.runs[0].stages[0].full.state = state;
        let app = app_of((snap, windows));
        let (layout, lines) = paint_view(&app);
        let rect = rect_of(&layout, &stage_key(1));
        let text: String = lines_text(&lines)[usize::from(rect.y + 1)]
            .chars()
            .skip(usize::from(rect.x))
            .take(usize::from(rect.width))
            .collect();
        assert!(
            text.contains(&format!("stage 1/2  tier 3 {mark}")),
            "{text}"
        );
        assert_eq!(glyph_of(&lines, rect).0, glyph, "{state:?}");
    }
}

/// The plan gate groups its tasks by stage node, and neither stage, uncreated before
/// approval (C-24), is drawn as created.
#[test]
fn the_gate_groups_tasks_by_uncreated_stages() {
    let app = app_of(staged_gate_fixture());
    let rows = view_rows(&app);
    let keys: Vec<(NodeKey, u16)> = rows.iter().map(|r| (r.key.clone(), r.depth)).collect();
    assert_eq!(
        keys,
        [
            (NodeKey::Run(RUN_ID.into()), 0),
            (stage_key(1), 1),
            (task_key("t1"), 2),
            (stage_key(2), 1),
            (task_key("t2"), 2),
        ]
    );
    let (layout, lines) = paint_view(&app);
    for n in [1, 2] {
        let (glyph, style) = glyph_of(&lines, rect_of(&layout, &stage_key(n)));
        assert_eq!(glyph, "◌", "stage {n}");
        assert_eq!(style.fg, Some(theme::status_color(proto::Status::Starting)));
    }
}

/// Pinning: a one-stage run has no stage node; its view is milestone 8c's.
#[test]
fn a_single_stage_run_has_no_stage_node() {
    let (mut snap, windows) = three_task_fixture();
    snap.runs[0].stages = vec![stage(1, Some(&"1".repeat(40)), 3, 1)];
    let app = app_of((snap, windows));
    let rows = view_rows(&app);
    assert!(
        rows.iter()
            .all(|row| !matches!(row.key, NodeKey::Stage { .. })),
    );
    assert_eq!(rows[1].key, task_key("t0"));
    assert_eq!(rows[1].depth, 1);
}

#[test]
fn fix_task_row_shows_its_origin() {
    let app = app_of(staged_fixture());
    let rows = view_rows(&app);
    let text = |id: &str| {
        let row = rows.iter().find(|row| row.key == task_key(id)).unwrap();
        crate::graph::content_text(row)
    };
    assert_eq!(text("fix1"), "fix1 fix t2 S (bisect)");
    assert_eq!(text("t1"), "t1 reset model S");
}

/// A stage is dimmed as finished only when every task merged and tier 3 is green on
/// its head (decision 16's dimming, extended to stages).
#[test]
fn a_stage_is_dim_only_when_merged_and_green() {
    use proto::FullState;
    for (merged, state, dim) in [
        (3, FullState::Green, true),
        (2, FullState::Green, false),
        (3, FullState::Red, false),
        (3, FullState::None, false),
    ] {
        let (mut snap, windows) = staged_fixture();
        snap.runs[0].stages[0].merged = merged;
        snap.runs[0].stages[0].full.state = state;
        let app = app_of((snap, windows));
        let (layout, lines) = paint_view(&app);
        let rect = rect_of(&layout, &stage_key(1));
        for (x, y) in all_cells(rect) {
            assert_eq!(
                style_at(&lines, x, y).add_modifier.contains(Modifier::DIM),
                dim,
                "{merged} merged, {state:?}: cell ({x}, {y})"
            );
        }
    }
}
