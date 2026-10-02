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
                "                                                            ╭──────────────────────────╮",
                "                                                          ┌─┤ ✓ t1 reset model S       │",
                "                                                          │ ╰──────────────────────────╯",
                "                                                          │",
                "                      ╭─────────────────────────────────╮ │ ╭──────────────────────────╮",
                "                    ┌─┤ ✗ stage 1/2  tier 3 ✗ bisecting ├─┼─┤ ● t2 reset endpoint S    │",
                "                    │ ╰─────────────────────────────────╯ │ ╰──────────────────────────╯",
                "                    │                                     │",
                "╭─────────────────╮ │                                     │ ╭──────────────────────────╮",
                "│ ◉ run 3f9a  1/4 ├─┤                                     └─┤ ▫ fix1 fix t2 S (bisect) │",
                "╰─────────────────╯ │                                       ╰──────────────────────────╯",
                "                    │",
                "                    │ ╭─────────────────────────────────╮   ╭──────────────────────────╮",
                "                    └─┤ ◌ stage 2/2  tier 3 ◌           ├───┤ ◌ t3 reset view S        │",
                "                      ╰─────────────────────────────────╯   ╰──────────────────────────╯",
            ],
            width
        )
    );
    // A stage not created yet is `◌` in the starting colour, never a created stage's.
    let (glyph, style) = glyph_of(&lines, rect_of(&layout, &stage_key(2)));
    assert_eq!(glyph, "◌");
    assert_eq!(style.fg, Some(theme::fg(theme::Role::Muted)));
}

/// Each tier-3 state's text and glyph; a stage with no head is `◌` whatever it says.
/// Milestone 9.0.7 decisions 3 and 18: a bisecting stage is `✗ bisecting` (it is red),
/// never a spinner, one not yet run `◌`, and a finished job shows its time. The box
/// draws the whole text (`MAX_STAGE_NODE_WIDTH`).
#[test]
fn each_tier3_state_has_its_mark_and_glyph() {
    use proto::FullState;
    for (state, text, glyph) in [
        (FullState::Green, "stage 1/2  tier 3 ✓ 41m12s", "✓"),
        (FullState::Red, "stage 1/2  tier 3 ✗ 41m12s", "✗"),
        (FullState::Running, "stage 1/2  tier 3 running", "⠋"),
        (FullState::Bisecting, "stage 1/2  tier 3 ✗ bisecting", "✗"),
        (FullState::None, "stage 1/2  tier 3 ◌", "◌"),
    ] {
        let (mut snap, windows) = staged_fixture();
        snap.runs[0].stages[0].full.state = state;
        let app = app_of((snap, windows));
        let rows = view_rows(&app);
        let row = rows.iter().find(|row| row.key == stage_key(1)).unwrap();
        assert_eq!(crate::graph::content_text(row), text, "{state:?}");
        let (layout, lines) = paint_view(&app);
        let rect = rect_of(&layout, &stage_key(1));
        let drawn: String = lines_text(&lines)[usize::from(rect.y + 1)]
            .chars()
            .skip(usize::from(rect.x))
            .take(usize::from(rect.width))
            .collect();
        assert!(drawn.contains(text), "{state:?}: {drawn}");
        assert_eq!(glyph_of(&lines, rect).0, glyph, "{state:?}");
    }
}

/// Controller ruling on decision 18: every stage form draws uncut, the widest at
/// `stage 10/10` (`MAX_STAGE_NODE_WIDTH`), while task boxes keep `MAX_NODE_WIDTH`.
#[test]
fn every_stage_form_draws_uncut_at_stage_10_of_10() {
    use proto::FullState;
    for (state, secs, tier) in [
        (FullState::Green, Some(64), "✓ 1m04s"),
        (FullState::Red, Some(3720), "✗ 1h02m"),
        (FullState::Running, None, "running"),
        (FullState::Bisecting, Some(64), "✗ bisecting"),
        (FullState::None, None, "◌"),
    ] {
        let (mut snap, windows) = staged_fixture();
        let stages = &mut snap.runs[0].stages;
        for n in 3..=10 {
            stages.push(stage(n, None, 0, 0));
        }
        stages[9].head = Some("a".repeat(40));
        stages[9].full.state = state;
        stages[9].full.secs = secs;
        let app = app_of((snap, windows));
        let (layout, lines) = paint_view(&app);
        let rect = rect_of(&layout, &stage_key(10));
        let drawn: String = lines_text(&lines)[usize::from(rect.y + 1)]
            .chars()
            .skip(usize::from(rect.x))
            .take(usize::from(rect.width))
            .collect();
        let text = format!("stage 10/10  tier 3 {tier}");
        assert!(drawn.contains(&format!("{text} ")), "{state:?}: {drawn}");
        assert!(!drawn.contains('…'), "{state:?}: {drawn}");
        assert!(rect.width <= crate::graph::MAX_STAGE_NODE_WIDTH);
        for (i, node) in layout.nodes.iter().enumerate() {
            if matches!(node.key, NodeKey::Task { .. }) {
                assert!(node.rect.width <= crate::graph::MAX_NODE_WIDTH);
            }
            // The wider stage tier pushes the next one right: no two boxes overlap.
            for other in &layout.nodes[i + 1..] {
                assert!(
                    !node.rect.intersects(other.rect),
                    "{:?} {:?}",
                    node.key,
                    other.key
                );
            }
        }
    }
}

/// Milestone 9.2 decision 42: a `pr`-mode stage row draws whole, its pull request
/// after its tier 3, up to `stage 10/10  tier 3 ✗ bisecting  #9999  ci ✗  99 threads`.
#[test]
fn a_pr_stage_row_draws_uncut() {
    use crate::tree::pr_fixtures::{pr, pr_fixture};
    use proto::{CiState, FullState, PrState};
    let (mut snap, windows) = pr_fixture();
    let stages = &mut snap.runs[0].stages;
    for n in 4..=10 {
        stages.push(stage(n, None, 0, 0));
    }
    stages[9].head = Some("a".repeat(40));
    stages[9].full.state = FullState::Bisecting;
    let mut last = pr(9999, PrState::Open, CiState::Red);
    last.threads.new = 99;
    stages[9].pr = Some(last);
    let app = app_of((snap, windows));
    let (layout, lines) = paint_view(&app);
    for (n, text) in [
        (2, "stage 2/10  tier 3 ✓ 38s  #142  ci ✗  3 threads"),
        (
            10,
            "stage 10/10  tier 3 ✗ bisecting  #9999  ci ✗  99 threads",
        ),
    ] {
        let rect = rect_of(&layout, &stage_key(n));
        let drawn: String = lines_text(&lines)[usize::from(rect.y + 1)]
            .chars()
            .skip(usize::from(rect.x))
            .take(usize::from(rect.width))
            .collect();
        assert!(drawn.contains(&format!("{text} ")), "{drawn}");
        assert!(!drawn.contains('…'), "{drawn}");
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
        assert_eq!(style.fg, Some(theme::fg(theme::Role::Muted)));
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
