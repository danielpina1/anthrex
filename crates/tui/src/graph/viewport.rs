//! The two-dimensional viewport onto the canvas `graph::layout` produces
//! (spec §4.2).
//!
//! `Pan` is the canvas coordinate shown at the viewport's top-left corner
//! (decision 14). Every method here keeps it inside the canvas, so the
//! viewport never scrolls past the canvas's own right or bottom edge.

use ratatui::layout::Rect;

/// The canvas coordinate shown at the viewport's top-left corner (decision
/// 14).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pan {
    pub x: u16,
    pub y: u16,
}

impl Pan {
    /// Pulls `self` back inside the canvas so the viewport never extends past
    /// it on the right or the bottom (decision 14).
    ///
    /// A canvas narrower or shorter than `area` cannot be panned at all on
    /// that axis: the pan is pinned to zero there rather than left free to
    /// show blank space no scroll could ever put content behind.
    pub fn clamped(self, canvas: (u16, u16), area: Rect) -> Pan {
        let max_x = canvas.0.saturating_sub(area.width);
        let max_y = canvas.1.saturating_sub(area.height);
        Pan {
            x: self.x.min(max_x),
            y: self.y.min(max_y),
        }
    }

    /// Moves `self` by the smallest amount on each axis that brings `rect`
    /// wholly inside the viewport (decision 15), independently per axis: a
    /// rect off only the right edge moves only `x`, never `y`.
    ///
    /// When `rect` is wider or taller than the viewport itself, "wholly
    /// inside" cannot be satisfied on that axis, so the rule degenerates to
    /// showing the rect's own top-left corner.
    pub fn revealing(self, rect: Rect, canvas: (u16, u16), area: Rect) -> Pan {
        let x = reveal_axis(self.x, rect.x, rect.right(), area.width);
        let y = reveal_axis(self.y, rect.y, rect.bottom(), area.height);
        Pan { x, y }.clamped(canvas, area)
    }
}

/// The one-axis form of `Pan::revealing`: the smallest new offset that brings
/// `[near, far)` wholly inside a `span`-cell window currently at `pan`, or
/// `pan` itself when it already does.
///
/// Each of the two `if` arms below moves toward exactly one edge of `[near,
/// far)`, so a rect that is only off the far edge never touches the near one
/// and vice versa — the independence decision 15 requires.
fn reveal_axis(pan: u16, near: u16, far: u16, span: u16) -> u16 {
    if far.saturating_sub(near) > span {
        // Wider or taller than the viewport itself: "wholly inside" cannot be
        // satisfied on this axis, so the rule degenerates to the rect's own
        // near edge — its top-left corner, read on one axis at a time.
        near
    } else if near < pan {
        near
    } else if far > pan.saturating_add(span) {
        far.saturating_sub(span)
    } else {
        pan
    }
}

#[cfg(test)]
mod tests;
