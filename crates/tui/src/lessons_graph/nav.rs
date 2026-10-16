//! Decision 46 (task M9.15.15): the graph page's camera and navigation. Pure.
//!
//! - **Zoom** steps by factors of [`ZOOM_STEP`] from 1, clamped to [`ZOOM_MIN`] and
//!   [`ZOOM_MAX`] at the ends ([`zoom_steps`]); the camera holds a step index, so
//!   zooming in and back out returns to the same zoom exactly.
//! - **Pan** moves the centre by a tenth of the visible span (`1 / zoom`, the side of
//!   the unit square at zoom 1), and the centre stays inside the unit square.
//! - **Projection** ([`View`]) keeps the unit square's aspect on screen: a unit is
//!   `zoom · min(W − 1, 2(H − 1))` columns wide and half as many rows tall, since a cell
//!   is twice as tall as wide. So layout coordinates are already "canvas coordinates
//!   scaled to cell aspect", and [`nearest`] works on them directly. A node's cell is
//!   computed exactly as ratatui's `Canvas` places a printed label, so a click and the
//!   glyph agree.
//! - **Arrows** ([`nearest`]) pick the nearest node within ±45° of the direction
//!   (inclusive), ties by id; none keeps the selection.
//! - **Click** ([`pick`]) takes the node whose cell is nearest the click, within one
//!   cell (Chebyshev), ties by id; none clears the selection.

use super::layout::Point;

/// The smallest zoom (Interfaces).
pub const ZOOM_MIN: f64 = 0.25;
/// The largest zoom (Interfaces).
pub const ZOOM_MAX: f64 = 8.0;
/// One zoom step's factor (Interfaces).
pub const ZOOM_STEP: f64 = 1.25;
/// The lowest and highest step exponents: `1.25⁻⁷ ≈ 0.21` and `1.25¹⁰ ≈ 9.3`, clamped
/// to the bounds.
const STEP_LOW: i32 = -7;
const STEP_HIGH: i32 = 10;
/// The step index of zoom 1.
pub const ZOOM_ONE: usize = (-STEP_LOW) as usize;
/// The fraction of the visible span one pan moves.
pub const PAN_FRACTION: f64 = 0.1;

/// Every zoom the camera can take, smallest first: `ZOOM_MIN`, `1.25ᵏ` for `k` in
/// `-6..=9`, `ZOOM_MAX`.
pub fn zoom_steps() -> Vec<f64> {
    (STEP_LOW..=STEP_HIGH).map(zoom_at).collect()
}

fn zoom_at(k: i32) -> f64 {
    ZOOM_STEP.powi(k).clamp(ZOOM_MIN, ZOOM_MAX)
}

/// What the canvas shows: a zoom step and the unit-square point at its centre (`y`
/// grows downwards, as the layout's does).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub step: usize,
    pub cx: f64,
    pub cy: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            step: ZOOM_ONE,
            cx: 0.5,
            cy: 0.5,
        }
    }
}

impl Camera {
    pub fn zoom(&self) -> f64 {
        zoom_at(self.step as i32 + STEP_LOW)
    }

    /// `+`/`=`: one step in, stopping at [`ZOOM_MAX`].
    pub fn zoom_in(&mut self) {
        self.step = (self.step + 1).min((STEP_HIGH - STEP_LOW) as usize);
    }

    /// `-`: one step out, stopping at [`ZOOM_MIN`].
    pub fn zoom_out(&mut self) {
        self.step = self.step.saturating_sub(1);
    }

    /// The visible span: the unit square's side at this zoom.
    pub fn span(&self) -> f64 {
        1.0 / self.zoom()
    }

    /// `h`/`j`/`k`/`l`: `dx`, `dy` in `-1..=1` tenths of the span.
    pub fn pan(&mut self, dx: i8, dy: i8) {
        let step = PAN_FRACTION * self.span();
        self.cx = (self.cx + f64::from(dx) * step).clamp(0.0, 1.0);
        self.cy = (self.cy + f64::from(dy) * step).clamp(0.0, 1.0);
    }

    /// `0`: zoom 1, centred.
    pub fn recentre(&mut self) {
        *self = Camera::default();
    }
}

/// An arrow key's direction (`y` grows downwards).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    fn vector(self) -> (f64, f64) {
        match self {
            Dir::Left => (-1.0, 0.0),
            Dir::Right => (1.0, 0.0),
            Dir::Up => (0.0, -1.0),
            Dir::Down => (0.0, 1.0),
        }
    }
}

/// The node nearest `points[from]` within ±45° of `dir` (the cone's edges included),
/// ties by id; `None` when the cone is empty.
pub fn nearest(points: &[(&str, Point)], from: usize, dir: Dir) -> Option<usize> {
    let (_, (fx, fy)) = *points.get(from)?;
    let (ux, uy) = dir.vector();
    let mut best: Option<(f64, &str, usize)> = None;
    for (i, (id, (x, y))) in points.iter().enumerate() {
        if i == from {
            continue;
        }
        let (vx, vy) = (x - fx, y - fy);
        let along = vx * ux + vy * uy;
        let across = (vx * uy - vy * ux).abs();
        if along <= 0.0 || across > along {
            continue;
        }
        let d = vx * vx + vy * vy;
        let better = match best {
            None => true,
            Some((bd, bid, _)) => d < bd || (d == bd && *id < bid),
        };
        if better {
            best = Some((d, id, i));
        }
    }
    best.map(|(_, _, i)| i)
}

/// The canvas's rectangle on screen and the camera over it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub camera: Camera,
}

impl View {
    /// Columns per unit; `None` for a canvas too small to place anything (ratatui
    /// divides by `width − 1` and `height − 1`).
    fn scale(&self) -> Option<f64> {
        if self.width < 2 || self.height < 2 {
            return None;
        }
        let fit = f64::from(self.width - 1).min(2.0 * f64::from(self.height - 1));
        Some(self.camera.zoom() * fit)
    }

    /// The `Canvas`'s `x_bounds` and `y_bounds`. The canvas's `y` grows upwards, so a
    /// layout point `(x, y)` is drawn at `(x, −y)`.
    pub fn bounds(&self) -> Option<([f64; 2], [f64; 2])> {
        let s = self.scale()?;
        let span_x = f64::from(self.width - 1) / s;
        let span_y = 2.0 * f64::from(self.height - 1) / s;
        let (cx, cy) = (self.camera.cx, self.camera.cy);
        Some((
            [cx - span_x / 2.0, cx + span_x / 2.0],
            [-(cy + span_y / 2.0), -(cy - span_y / 2.0)],
        ))
    }

    /// The cell a label printed at layout point `p` lands in, as ratatui's `Canvas`
    /// computes it; `None` when it is outside the view (ratatui does not draw it).
    pub fn cell(&self, p: Point) -> Option<(u16, u16)> {
        let (xb, yb) = self.bounds()?;
        let (lx, ly) = (p.0, -p.1);
        let (left, right, bottom, top) = (xb[0], xb[1], yb[0], yb[1]);
        if !(lx >= left && lx <= right && ly <= top && ly >= bottom) {
            return None;
        }
        let width = (xb[1] - xb[0]).abs();
        let height = (yb[1] - yb[0]).abs();
        let res = (f64::from(self.width - 1), f64::from(self.height - 1));
        let col = ((lx - left) * res.0 / width) as u16 + self.x;
        let row = ((top - ly) * res.1 / height) as u16 + self.y;
        Some((col, row))
    }
}

/// The node whose cell is nearest `click`, within one cell, ties by id.
pub fn pick(cells: &[(&str, Option<(u16, u16)>)], click: (u16, u16)) -> Option<usize> {
    let mut best: Option<(u32, &str, usize)> = None;
    for (i, (id, cell)) in cells.iter().enumerate() {
        let Some((c, r)) = *cell else { continue };
        let dc = u32::from(c.abs_diff(click.0));
        let dr = u32::from(r.abs_diff(click.1));
        if dc > 1 || dr > 1 {
            continue;
        }
        let d = dc * dc + dr * dr;
        let better = match best {
            None => true,
            Some((bd, bid, _)) => d < bd || (d == bd && *id < bid),
        };
        if better {
            best = Some((d, id, i));
        }
    }
    best.map(|(_, _, i)| i)
}

#[cfg(test)]
#[path = "nav_tests.rs"]
mod tests;
