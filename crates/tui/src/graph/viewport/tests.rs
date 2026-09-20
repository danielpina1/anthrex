//! `Pan` is asserted by exact coordinates, the same way layout is asserted by
//! exact rectangles: a clamp or a reveal that moves one cell too far, or on
//! the wrong axis, shows up as a number rather than as a property several
//! wrong answers could satisfy.

use super::*;
use ratatui::layout::Rect;

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
