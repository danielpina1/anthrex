//! `Pan` is asserted by exact coordinates, the same way layout is asserted by
//! exact rectangles: a clamp or a reveal that moves one cell too far, or on
//! the wrong axis, shows up as a number rather than as a property several
//! wrong answers could satisfy.

use super::*;
use crate::graph::{Edge, PlacedNode};
use ratatui::layout::Rect;

/// Two nodes side by side on the canvas: `a` at the origin, `b` three columns
/// past its right edge, with an edge between them — enough shape to test a
/// box, a gap and an edge cell all in one layout.
fn sample_layout() -> (Layout, NodeKey, NodeKey) {
    let a = NodeKey::Window(1);
    let b = NodeKey::Window(2);
    let layout = Layout {
        nodes: vec![
            PlacedNode {
                key: a.clone(),
                rect: Rect::new(0, 0, 12, 3),
                depth: 0,
            },
            PlacedNode {
                key: b.clone(),
                rect: Rect::new(15, 0, 12, 3),
                depth: 1,
            },
        ],
        edges: vec![Edge {
            parent: a.clone(),
            children: vec![b.clone()],
        }],
        size: (27, 3),
    };
    (layout, a, b)
}

#[test]
fn clamped_never_shows_past_the_canvas() {
    let area = Rect::new(0, 0, 10, 5);

    // A canvas bigger than the area on both axes: clamps to the last page
    // that still shows the canvas's own edge, not the pan asked for.
    let pan = Pan { x: 100, y: 100 }.clamped((40, 20), area);
    assert_eq!(pan, Pan { x: 30, y: 15 });

    // A canvas smaller than the area on both axes: pinned to zero rather
    // than left free to show blank space no scroll could ever fill.
    let pan = Pan { x: 5, y: 5 }.clamped((4, 2), area);
    assert_eq!(pan, Pan { x: 0, y: 0 });

    // A canvas that exactly fits the area: the only valid pan is zero.
    let pan = Pan { x: 3, y: 3 }.clamped((10, 5), area);
    assert_eq!(pan, Pan { x: 0, y: 0 });
}

#[test]
fn revealing_moves_the_minimum_on_each_axis() {
    let area = Rect::new(0, 0, 10, 5);
    let canvas = (100, 100);

    // Off the right edge only: x moves just far enough to bring the rect's
    // right edge to the viewport's right edge; y stays put.
    let rect = Rect::new(20, 2, 4, 3);
    assert_eq!(
        Pan::default().revealing(rect, canvas, area),
        Pan { x: 14, y: 0 }
    );

    // Off the bottom edge only: y moves; x stays put.
    let rect = Rect::new(2, 20, 4, 3);
    assert_eq!(
        Pan::default().revealing(rect, canvas, area),
        Pan { x: 0, y: 18 }
    );

    // Off both edges: both move.
    let rect = Rect::new(20, 20, 4, 3);
    assert_eq!(
        Pan::default().revealing(rect, canvas, area),
        Pan { x: 14, y: 18 }
    );
}

#[test]
fn revealing_a_visible_rect_does_not_move() {
    let area = Rect::new(0, 0, 10, 5);
    let canvas = (100, 100);
    let pan = Pan { x: 5, y: 5 };

    // Wholly inside the viewport's [5, 15) x [5, 10): neither axis needs to
    // move, so the pan comes back unchanged.
    let rect = Rect::new(6, 6, 3, 2);
    assert_eq!(pan.revealing(rect, canvas, area), pan);
}

#[test]
fn revealing_a_rect_larger_than_the_area_shows_its_top_left() {
    let area = Rect::new(0, 0, 10, 5);
    let canvas = (100, 100);

    // Wider and taller than the viewport: "wholly inside" is impossible on
    // either axis, so the rule degenerates to the rect's own top-left cell.
    let rect = Rect::new(20, 20, 15, 8);
    assert_eq!(
        Pan::default().revealing(rect, canvas, area),
        Pan { x: 20, y: 20 }
    );
}

#[test]
fn every_cell_of_a_box_hits_it() {
    let (layout, a, _b) = sample_layout();
    let geometry = GraphGeometry {
        area: Rect::new(0, 0, 30, 5),
        pan: Pan::default(),
    };
    let rect = layout.node(&a).unwrap().rect;

    for x in rect.x..rect.right() {
        for y in rect.y..rect.bottom() {
            assert_eq!(
                geometry.node_at(&layout, x, y),
                Some(a.clone()),
                "cell ({x}, {y}) should hit the box"
            );
        }
    }
}

#[test]
fn a_gap_cell_hits_nothing() {
    let (layout, _a, _b) = sample_layout();
    let geometry = GraphGeometry {
        area: Rect::new(0, 0, 30, 5),
        pan: Pan::default(),
    };

    // Column 13 is in the tier gap between the two boxes; row 0 is not the
    // row the edge is painted on. Neither box, nor the edge, is here.
    assert_eq!(geometry.node_at(&layout, 13, 0), None);
}

#[test]
fn an_edge_cell_hits_nothing() {
    let (layout, _a, _b) = sample_layout();
    let geometry = GraphGeometry {
        area: Rect::new(0, 0, 30, 5),
        pan: Pan::default(),
    };

    // Row 1 is the shared middle row of both boxes, where the painter draws
    // the straight run connecting them (decision 13). It is a drawn line,
    // not a node.
    assert_eq!(geometry.node_at(&layout, 13, 1), None);
}

#[test]
fn a_cell_past_the_canvas_hits_nothing() {
    let (layout, _a, _b) = sample_layout();
    // The viewport area is wider and taller than anything the layout placed
    // (the canvas is only 27 x 3). A click in that dead space still lands
    // inside the viewport but past every node.
    let geometry = GraphGeometry {
        area: Rect::new(0, 0, 30, 5),
        pan: Pan::default(),
    };
    assert_eq!(geometry.node_at(&layout, 29, 4), None);

    // A click outside the viewport area altogether must not panic (column
    // and row are unsigned, so going the wrong way could underflow).
    let geometry = GraphGeometry {
        area: Rect::new(5, 5, 10, 5),
        pan: Pan::default(),
    };
    assert_eq!(geometry.node_at(&layout, 0, 0), None);
}

#[test]
fn hit_testing_accounts_for_the_pan() {
    let (layout, a, b) = sample_layout();
    let area = Rect::new(0, 0, 30, 5);

    // With no pan, the top-left screen cell lands on canvas cell (0, 1),
    // inside `a`.
    let unpanned = GraphGeometry {
        area,
        pan: Pan::default(),
    };
    assert_eq!(unpanned.node_at(&layout, 0, 1), Some(a));

    // Panned fifteen columns right, the same screen cell lands on canvas
    // cell (15, 1) instead — `b`'s left border — proving the mapping goes
    // through the pan rather than treating screen coordinates as canvas
    // coordinates directly.
    let panned = GraphGeometry {
        area,
        pan: Pan { x: 15, y: 0 },
    };
    assert_eq!(panned.node_at(&layout, 0, 1), Some(b));
}
