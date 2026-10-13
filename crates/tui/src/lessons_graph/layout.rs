//! Decision 44 (task M9.15.14): a deterministic Fruchterman–Reingold layout of the
//! lessons graph in the unit square. Pure, `f64`, no clock and no randomness: the same
//! graph always lays out the same, bit for bit, whatever the order of its nodes and
//! edges.
//!
//! 1. **Order.** Nodes are taken by id (duplicates merged) and edges by their sorted
//!    endpoints (ruling C18), so shuffling the input cannot change the float order.
//! 2. **Start.** Hubs (modules and the root) on a circle of radius [`HUB_RADIUS`]
//!    around the centre in id order; every lesson at its id's FNV-1a 64-bit hash (high
//!    32 bits → x, low 32 bits → y, each ÷ 2³²). A node landing exactly on an earlier
//!    one is nudged along a spiral by its rank in id order.
//! 3. **Forces.** At most [`LAYOUT_ITERATIONS`] rounds with `k = sqrt(1 / n)`:
//!    repulsion `k² / d` between every pair, attraction `w · d² / k` along each edge
//!    (`touches` w = 2, others 1), each move capped by a temperature falling linearly
//!    from [`START_TEMPERATURE`] to 0, positions clamped to [[`POS_MIN`], [`POS_MAX`]].
//!    It stops early when an iteration's largest move is below [`STOP_MOVE`].
//! 4. **Separation** (ruling C18). Force layout with clamping does not guarantee
//!    spacing, so a final pass does: hubs then lessons, each in id order, keep their
//!    place when it is at least [`min_separation`] (with [`SLACK`]) from every node
//!    already placed, else move to the nearest free point of a grid of that spacing.
//!    Should no grid point be free, every node is snapped to its own grid point, which
//!    always succeeds (the grid has at least `3.68 · n` points).
//!
//! Cost is bounded and counted in [`LayoutOps`], never timed.

use std::f64::consts::TAU;

use proto::{GraphEdge, GraphEdgeKind, GraphNode, GraphNodeKind, LessonGraph};

/// The most force iterations (decision 44).
pub const LAYOUT_ITERATIONS: usize = 300;
/// The hubs' starting circle's radius around the centre (decision 44).
pub const HUB_RADIUS: f64 = 0.35;
/// The lowest coordinate (decision 44's clamp).
pub const POS_MIN: f64 = 0.02;
/// The highest coordinate (decision 44's clamp).
pub const POS_MAX: f64 = 0.98;
/// The first iteration's largest move; it falls linearly to 0 (decision 44).
pub const START_TEMPERATURE: f64 = 0.1;
/// An iteration whose largest move is below this ends the layout (decision 44).
pub const STOP_MOVE: f64 = 1e-4;
/// A `touches` edge's attraction weight; every other edge weighs 1 (decision 44).
pub const TOUCHES_WEIGHT: f64 = 2.0;
/// The first nudge's radius for a node starting exactly on an earlier one.
pub const NUDGE: f64 = 1e-3;
/// The separation pass's relative margin over [`min_separation`]: kept nodes are at
/// least `min · (1 + SLACK)` apart and grid points `min · (1 + 2 · SLACK)`, so a
/// check at exactly `min` holds whatever the last bits of a distance's rounding (those
/// are near 1e-16, ten orders of magnitude below the margin).
pub const SLACK: f64 = 1e-6;
/// The distance below which two nodes count as coincident in the force step; their
/// push then takes a direction from their ranks instead of their (zero) offset.
const COINCIDENT: f64 = 1e-9;
/// The golden angle, which spreads successive nudges and coincident pushes.
const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;
/// The most points one grid bucket can hold: its side equals the kept spacing, and a
/// closed square of side `s` holds at most four points pairwise at least `s` apart.
pub const BUCKET_MAX: u64 = 4;

/// A position in the unit square: `(x, y)`.
pub type Point = (f64, f64);

/// The work one layout did, counted so its cost is pinned without a clock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayoutOps {
    /// Start-position comparisons while nudging coincident nodes apart.
    pub nudge: u64,
    /// Pair and edge force evaluations: `iterations · (n(n−1)/2 + edges)`.
    pub forces: u64,
    /// Distance checks and grid points scanned by the separation pass.
    pub separation: u64,
}

/// A laid-out graph.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// One position per input node, in the input's order.
    pub positions: Vec<Point>,
    /// Force iterations run (at most [`LAYOUT_ITERATIONS`]).
    pub iterations: usize,
    /// The last iteration's largest move (0 when none ran).
    pub last_move: f64,
    /// True when the separation pass had to snap every node to the grid.
    pub snapped: bool,
    pub ops: LayoutOps,
}

/// FNV-1a, 64 bits: decision 44's start-position hash.
pub fn fnv1a(id: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The least distance the separation pass keeps between two of `n` nodes.
pub fn min_separation(n: usize) -> f64 {
    0.5 / (n.max(1) as f64).sqrt()
}

/// Decision 44's layout of `graph`.
pub fn layout(graph: &LessonGraph) -> Layout {
    layout_with(&graph.nodes, &graph.edges, fnv1a)
}

/// The layout of `nodes` and `edges` with `hash` for the lessons' start positions
/// (the seam the coincident-hash test uses).
pub fn layout_with(nodes: &[GraphNode], edges: &[GraphEdge], hash: fn(&str) -> u64) -> Layout {
    let graph = Prepared::new(nodes, edges);
    let mut ops = LayoutOps::default();
    let mut points = graph.start(hash, &mut ops.nudge);
    let (iterations, last_move) = graph.forces(&mut points, &mut ops.forces);
    let snapped = graph.separate(&mut points, &mut ops.separation);
    Layout {
        positions: graph.slot.iter().map(|&u| points[u]).collect(),
        iterations,
        last_move,
        snapped,
        ops,
    }
}

/// The start positions of `nodes` (step 2), one per input node in the input's order.
pub fn start_positions(nodes: &[GraphNode], hash: fn(&str) -> u64) -> Vec<Point> {
    let graph = Prepared::new(nodes, &[]);
    let points = graph.start(hash, &mut 0);
    graph.slot.iter().map(|&u| points[u]).collect()
}

/// The graph in id order: distinct ids, which of them are hubs, each input node's
/// index among them, and the edges by sorted endpoints with their weights.
struct Prepared<'a> {
    ids: Vec<&'a str>,
    hub: Vec<bool>,
    slot: Vec<usize>,
    edges: Vec<(usize, usize, f64)>,
}

impl<'a> Prepared<'a> {
    fn new(nodes: &'a [GraphNode], edges: &[GraphEdge]) -> Self {
        let mut ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        let at = |id: &str| ids.binary_search(&id).ok();
        let slot: Vec<usize> = (nodes.iter()).filter_map(|n| at(&n.id)).collect();
        let mut hub = vec![false; ids.len()];
        for (node, &u) in nodes.iter().zip(&slot) {
            hub[u] |= node.kind != GraphNodeKind::Lesson;
        }
        let mut weighted: Vec<(usize, usize, f64)> = (edges.iter())
            .filter_map(|e| {
                let (a, b) = (at(&e.from)?, at(&e.to)?);
                let w = if e.kind == GraphEdgeKind::Touches {
                    TOUCHES_WEIGHT
                } else {
                    1.0
                };
                (a != b).then_some((a.min(b), a.max(b), w))
            })
            .collect();
        weighted.sort_by(|x, y| (x.0, x.1).cmp(&(y.0, y.1)).then(x.2.total_cmp(&y.2)));
        Prepared {
            ids,
            hub,
            slot,
            edges: weighted,
        }
    }

    /// Step 2, in id order.
    fn start(&self, hash: fn(&str) -> u64, ops: &mut u64) -> Vec<Point> {
        let hubs = self.hub.iter().filter(|h| **h).count();
        let mut ring = 0;
        let mut points: Vec<Point> = Vec::with_capacity(self.ids.len());
        for (u, id) in self.ids.iter().enumerate() {
            let base = if self.hub[u] {
                let angle = TAU * ring as f64 / hubs as f64;
                ring += 1;
                (
                    0.5 + HUB_RADIUS * angle.cos(),
                    0.5 + HUB_RADIUS * angle.sin(),
                )
            } else {
                let h = hash(id);
                let unit = |bits: u64| bits as f64 / 4_294_967_296.0;
                (clamp(unit(h >> 32)), clamp(unit(h & 0xffff_ffff)))
            };
            // Step 2's nudge: the k-th try sits on a spiral around the start; a try
            // that the clamp folds back onto a taken point is skipped. Bounded: a
            // node takes at most `4n + 8` tries, then keeps the last.
            let mut point = base;
            let mut tries = 0;
            while tries < 4 * self.ids.len() + 8 && {
                *ops += points.len() as u64;
                points.iter().any(|p| same(*p, point))
            } {
                tries += 1;
                let angle = GOLDEN_ANGLE * tries as f64;
                let radius = NUDGE * (tries as f64).sqrt();
                point = (
                    clamp(base.0 + radius * angle.cos()),
                    clamp(base.1 + radius * angle.sin()),
                );
            }
            points.push(point);
        }
        points
    }

    /// Step 3: the iterations run and the last one's largest move.
    fn forces(&self, points: &mut [Point], ops: &mut u64) -> (usize, f64) {
        let n = points.len();
        if n == 0 {
            return (0, 0.0);
        }
        let k = (1.0 / n as f64).sqrt();
        let mut ran = (0, 0.0);
        for it in 0..LAYOUT_ITERATIONS {
            let t = START_TEMPERATURE * (1.0 - it as f64 / LAYOUT_ITERATIONS as f64);
            let mut disp = vec![(0.0, 0.0); n];
            for i in 0..n {
                for j in i + 1..n {
                    *ops += 1;
                    let (ux, uy, d) = towards(points[i], points[j], i, j);
                    let f = k * k / d;
                    disp[i].0 += ux * f;
                    disp[i].1 += uy * f;
                    disp[j].0 -= ux * f;
                    disp[j].1 -= uy * f;
                }
            }
            for &(a, b, w) in &self.edges {
                *ops += 1;
                let (ux, uy, d) = towards(points[a], points[b], a, b);
                let f = w * d * d / k;
                disp[a].0 -= ux * f;
                disp[a].1 -= uy * f;
                disp[b].0 += ux * f;
                disp[b].1 += uy * f;
            }
            let mut largest: f64 = 0.0;
            for (p, (dx, dy)) in points.iter_mut().zip(disp) {
                let len = dx.hypot(dy);
                if len > 0.0 {
                    let step = len.min(t);
                    let next = (clamp(p.0 + dx / len * step), clamp(p.1 + dy / len * step));
                    largest = largest.max((next.0 - p.0).hypot(next.1 - p.1));
                    *p = next;
                }
            }
            ran = (it + 1, largest);
            if largest < STOP_MOVE {
                break;
            }
        }
        ran
    }

    /// Step 4; true when every node had to be snapped to the grid.
    fn separate(&self, points: &mut [Point], ops: &mut u64) -> bool {
        let n = points.len();
        if n == 0 {
            return false;
        }
        let keep = min_separation(n) * (1.0 + SLACK);
        let grid = Grid::new(min_separation(n) * (1.0 + 2.0 * SLACK));
        let order: Vec<usize> = (0..n)
            .filter(|&u| self.hub[u])
            .chain((0..n).filter(|&u| !self.hub[u]))
            .collect();
        let mut placed = Buckets::new(keep);
        let mut out = points.to_vec();
        let mut fits = true;
        for &u in &order {
            if placed.free(points[u], ops) {
                placed.add(points[u]);
                continue;
            }
            let found = (grid.nearest(points[u], ops).into_iter()).find(|&g| {
                let p = grid.point(g);
                placed.free(p, ops)
            });
            let Some(g) = found else {
                fits = false;
                break;
            };
            out[u] = grid.point(g);
            placed.add(out[u]);
        }
        if !fits {
            let mut taken = vec![false; grid.side * grid.side];
            for &u in &order {
                let g = (grid.nearest(points[u], ops).into_iter())
                    .find(|&g| !taken[g])
                    .expect("the grid has at least n points");
                taken[g] = true;
                out[u] = grid.point(g);
            }
        }
        points.copy_from_slice(&out);
        !fits
    }
}

/// The separation grid: `side × side` points `step` apart from [`POS_MIN`]; `side` is
/// the most that fit in `[POS_MIN, POS_MAX]`, so `side > 0.96 / step`, which for
/// `step = 0.5 / sqrt(n) · (1 + 2e-6)` gives `side² > 3.68 · n · (1 − 4e-6) ≥ n`.
struct Grid {
    step: f64,
    side: usize,
}

impl Grid {
    fn new(step: f64) -> Self {
        let side = ((POS_MAX - POS_MIN) / step).floor() as usize + 1;
        Grid { step, side }
    }

    fn point(&self, g: usize) -> Point {
        let at = |i: usize| (POS_MIN + i as f64 * self.step).min(POS_MAX);
        (at(g % self.side), at(g / self.side))
    }

    /// Every grid point, nearest to `p` first, ties by index.
    fn nearest(&self, p: Point, ops: &mut u64) -> Vec<usize> {
        let count = self.side * self.side;
        *ops += count as u64;
        let mut all: Vec<(f64, usize)> = (0..count)
            .map(|g| {
                let q = self.point(g);
                ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2), g)
            })
            .collect();
        all.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        all.into_iter().map(|(_, g)| g).collect()
    }
}

/// Placed points bucketed by squares of side `keep`: a point closer than `keep` to
/// `p` lies in `p`'s bucket or one of its eight neighbours.
struct Buckets {
    keep: f64,
    side: usize,
    cells: Vec<Vec<Point>>,
}

impl Buckets {
    fn new(keep: f64) -> Self {
        let side = (1.0 / keep).ceil() as usize + 1;
        Buckets {
            keep,
            side,
            cells: vec![Vec::new(); side * side],
        }
    }

    fn cell(&self, p: Point) -> (usize, usize) {
        let at = |v: f64| ((v / self.keep).floor().max(0.0) as usize).min(self.side - 1);
        (at(p.0), at(p.1))
    }

    /// True when `p` is at least `keep` from every placed point.
    fn free(&self, p: Point, ops: &mut u64) -> bool {
        let (cx, cy) = self.cell(p);
        for y in cy.saturating_sub(1)..=(cy + 1).min(self.side - 1) {
            for x in cx.saturating_sub(1)..=(cx + 1).min(self.side - 1) {
                for q in &self.cells[y * self.side + x] {
                    *ops += 1;
                    if (q.0 - p.0).hypot(q.1 - p.1) < self.keep {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn add(&mut self, p: Point) {
        let (x, y) = self.cell(p);
        self.cells[y * self.side + x].push(p);
    }
}

/// The unit vector from `q` to `p` and their distance; coincident points (ranks `i`
/// and `j`) take a direction from their ranks and the distance [`COINCIDENT`].
fn towards(p: Point, q: Point, i: usize, j: usize) -> (f64, f64, f64) {
    let (dx, dy) = (p.0 - q.0, p.1 - q.1);
    let d = dx.hypot(dy);
    if d < COINCIDENT {
        let angle = GOLDEN_ANGLE * (i * 31 + j) as f64;
        return (angle.cos(), angle.sin(), COINCIDENT);
    }
    (dx / d, dy / d, d)
}

fn clamp(v: f64) -> f64 {
    v.clamp(POS_MIN, POS_MAX)
}

fn same(a: Point, b: Point) -> bool {
    a.0.to_bits() == b.0.to_bits() && a.1.to_bits() == b.1.to_bits()
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
