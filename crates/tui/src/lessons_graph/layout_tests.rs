//! Task M9.15.14: decision 44's layout, with ruling C18's tests. No test times the
//! layout or relies on floating-point luck: positions are compared bit for bit, counts
//! exactly, and distances against bounds the separation pass keeps with a margin
//! ([`SLACK`], a millionth) far above any rounding of a distance in the unit square
//! (about 1e-16).

use proto::{GraphEdge, GraphEdgeKind, GraphNode, GraphNodeKind};

use super::*;

fn node(id: &str, kind: GraphNodeKind) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        kind,
        label: id.to_string(),
        status: None,
        layer: None,
    }
}

fn edge(from: &str, to: &str, kind: GraphEdgeKind) -> GraphEdge {
    GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
        kind,
    }
}

/// A graph of `n` nodes shaped like the daemon's: the root hub, up to nine module hubs,
/// then lessons, each touching one hub and sharing files with two neighbours.
fn graph(n: usize) -> (Vec<GraphNode>, Vec<GraphEdge>) {
    let modules = (n.saturating_sub(1) / 30).min(9);
    let mut nodes = vec![node("m:(root)", GraphNodeKind::Root)];
    let hubs: Vec<String> = std::iter::once("m:(root)".to_string())
        .chain((0..modules).map(|i| format!("m:mod{i}")))
        .collect();
    nodes.extend(hubs[1..].iter().map(|h| node(h, GraphNodeKind::Module)));
    let lessons = n.saturating_sub(nodes.len());
    let mut edges = Vec::new();
    for i in 0..lessons {
        let id = format!("l-{i:04}");
        nodes.push(node(&id, GraphNodeKind::Lesson));
        edges.push(edge(&id, &hubs[i % hubs.len()], GraphEdgeKind::Touches));
        for j in [i + 1, i + 7] {
            if j < lessons {
                edges.push(edge(&id, &format!("l-{j:04}"), GraphEdgeKind::SharesFiles));
            }
        }
    }
    assert_eq!(nodes.len(), n.max(1));
    (nodes, edges)
}

/// Positions keyed by id, as bits.
fn by_id(nodes: &[GraphNode], layout: &Layout) -> Vec<(String, u64, u64)> {
    let mut all: Vec<(String, u64, u64)> = (nodes.iter().zip(&layout.positions))
        .map(|(n, p)| (n.id.clone(), p.0.to_bits(), p.1.to_bits()))
        .collect();
    all.sort();
    all
}

fn distance(a: Point, b: Point) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

#[test]
fn fnv1a_matches_the_reference_vectors() {
    // The published FNV-1a 64-bit test vectors.
    assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a("foobar"), 0x8594_4171_f739_67e8);
}

#[test]
fn same_input_identical_positions() {
    for n in [2, 50, 300] {
        let (nodes, edges) = graph(n);
        let once = layout_with(&nodes, &edges, fnv1a);
        let twice = layout_with(&nodes, &edges, fnv1a);
        assert_eq!(by_id(&nodes, &once), by_id(&nodes, &twice), "n = {n}");
        // Shuffled nodes and edges (a fixed permutation, no randomness): positions
        // depend on ids, not order.
        let mut shuffled = nodes.clone();
        shuffled.reverse();
        shuffled.rotate_left(n / 3);
        let mut moved = edges.clone();
        moved.reverse();
        moved.rotate_left(edges.len() / 5);
        let other = layout_with(&shuffled, &moved, fnv1a);
        assert_eq!(by_id(&nodes, &once), by_id(&shuffled, &other), "n = {n}");
        assert_eq!(once.iterations, other.iterations);
        assert_eq!(once.ops, other.ops);
    }
}

#[test]
fn no_two_nodes_overlap() {
    for n in [1, 2, 50, 300] {
        let (nodes, edges) = graph(n);
        let laid = layout_with(&nodes, &edges, fnv1a);
        let min = 0.5 / (n as f64).sqrt();
        for (i, p) in laid.positions.iter().enumerate() {
            assert!(
                (POS_MIN..=POS_MAX).contains(&p.0) && (POS_MIN..=POS_MAX).contains(&p.1),
                "n = {n}: {} at {p:?}",
                nodes[i].id
            );
            for (j, q) in laid.positions.iter().enumerate().skip(i + 1) {
                assert!(
                    distance(*p, *q) >= min,
                    "n = {n}: {} and {} are {} apart, below {min}",
                    nodes[i].id,
                    nodes[j].id,
                    distance(*p, *q)
                );
            }
        }
    }
}

#[test]
fn every_node_coincident_is_still_separated() {
    // 300 nodes all hashed to one point, with no edges, still end up spaced and
    // clamped: the nudge, the forces and the separation pass together.
    fn zero(_: &str) -> u64 {
        0
    }
    let nodes: Vec<GraphNode> = (0..300)
        .map(|i| node(&format!("l-{i:04}"), GraphNodeKind::Lesson))
        .collect();
    let laid = layout_with(&nodes, &[], zero);
    let min = 0.5 / 300f64.sqrt();
    for (i, p) in laid.positions.iter().enumerate() {
        assert!((POS_MIN..=POS_MAX).contains(&p.0) && (POS_MIN..=POS_MAX).contains(&p.1));
        for q in &laid.positions[i + 1..] {
            assert!(distance(*p, *q) >= min);
        }
    }
}

#[test]
fn separation_falls_back_to_the_grid() {
    // Constructed so that the first tier finds no free grid point: 289 nodes sit on
    // every other grid cell's centre, which blocks every grid point (each centre is
    // `step / sqrt(2)` from its cell's four corners), and the other 11 sit on top of
    // them. The fallback then snaps every node to its own grid point.
    let n = 300;
    let nodes: Vec<GraphNode> = (0..n)
        .map(|i| node(&format!("l-{i:04}"), GraphNodeKind::Lesson))
        .collect();
    let graph = Prepared::new(&nodes, &[]);
    let grid = Grid::new(min_separation(n) * (1.0 + 2.0 * SLACK));
    let mut cells: Vec<usize> = (0..grid.side - 1).step_by(2).collect();
    if grid.side % 2 == 1 {
        cells.push(grid.side - 2);
    }
    let centre = |i: usize| POS_MIN + (i as f64 + 0.5) * grid.step;
    let mut points: Vec<Point> = Vec::new();
    for &y in &cells {
        for &x in &cells {
            points.push((centre(x), centre(y)));
        }
    }
    assert_eq!(points.len(), 289);
    while points.len() < n {
        points.push(points[points.len() % 7]);
    }
    let mut ops = 0;
    assert!(graph.separate(&mut points, &mut ops));
    let min = min_separation(n);
    for (i, p) in points.iter().enumerate() {
        assert!((POS_MIN..=POS_MAX).contains(&p.0) && (POS_MIN..=POS_MAX).contains(&p.1));
        for q in &points[i + 1..] {
            assert!(distance(*p, *q) >= min, "{p:?} {q:?}");
        }
    }
}

#[test]
fn converges_within_iterations() {
    // Ruling C18: decision 44's cooling keeps a dense graph moving by about the
    // temperature, so the early stop is pinned where it is provable. Two unlinked
    // nodes repel until the clamp holds both on the boundary, where every move is
    // exactly 0: a discrete outcome, reached long before the last iteration.
    let nodes = [
        node("l-a", GraphNodeKind::Lesson),
        node("l-b", GraphNodeKind::Lesson),
    ];
    let laid = layout_with(&nodes, &[], fnv1a);
    assert!(
        (1..LAYOUT_ITERATIONS).contains(&laid.iterations),
        "{}",
        laid.iterations
    );
    assert!(laid.last_move < STOP_MOVE, "{}", laid.last_move);
    assert!(!laid.snapped);
    // Both rest on the boundary: some coordinate of each is exactly a clamp bound.
    let edge = |v: f64| v == POS_MIN || v == POS_MAX;
    for p in &laid.positions {
        assert!(edge(p.0) || edge(p.1), "{p:?}");
    }
    // A layout never runs more than its cap, and an empty graph runs none.
    let (big, edges) = graph(300);
    assert!(layout_with(&big, &edges, fnv1a).iterations <= LAYOUT_ITERATIONS);
    assert_eq!(layout_with(&[], &[], fnv1a).iterations, 0);
}

#[test]
fn cost_is_counted_and_bounded() {
    // The bound is pinned by counting, never by a clock.
    let (nodes, edges) = graph(300);
    let laid = layout_with(&nodes, &edges, fnv1a);
    let n = 300u64;
    let pairs = n * (n - 1) / 2;
    assert!(laid.iterations > 0);
    // Each iteration evaluates every pair once and every edge once.
    assert_eq!(
        laid.ops.forces,
        laid.iterations as u64 * (pairs + edges.len() as u64)
    );
    assert!(laid.ops.forces <= LAYOUT_ITERATIONS as u64 * (pairs + edges.len() as u64));
    // The separation pass: per node, one bucket check (9 buckets of at most
    // `BUCKET_MAX`), a grid scan, and a bucket check per grid point; the fallback adds
    // one more scan per node.
    let grid = (((POS_MAX - POS_MIN) / (min_separation(300) * (1.0 + 2.0 * SLACK))).floor() as u64
        + 1)
    .pow(2);
    let check = 9 * BUCKET_MAX;
    assert!(laid.ops.separation <= n * (check + grid + grid * check) + n * grid);
    // The nudge compares each node with the ones before it, at most `4n + 9` times.
    assert!(laid.ops.nudge <= n * (4 * n + 9) * n);
}

#[test]
fn hubs_start_on_the_circle() {
    let mut nodes = vec![
        node("m:zeta", GraphNodeKind::Module),
        node("l-x", GraphNodeKind::Lesson),
        node("m:(root)", GraphNodeKind::Root),
        node("m:alpha", GraphNodeKind::Module),
    ];
    let start = start_positions(&nodes, fnv1a);
    // In id order: `m:(root)`, `m:alpha`, `m:zeta`, at angles 0, 120° and 240°. The
    // tolerance covers the rounding of `0.5 + 0.35 · cos θ` (a few 1e-16).
    let want = [
        ("m:(root)", 0.0),
        ("m:alpha", 1.0 / 3.0),
        ("m:zeta", 2.0 / 3.0),
    ];
    for (id, turn) in want {
        let i = nodes.iter().position(|n| n.id == id).unwrap();
        let angle = std::f64::consts::TAU * turn;
        let p = start[i];
        assert!(
            (p.0 - (0.5 + HUB_RADIUS * angle.cos())).abs() < 1e-12,
            "{id} {p:?}"
        );
        assert!(
            (p.1 - (0.5 + HUB_RADIUS * angle.sin())).abs() < 1e-12,
            "{id} {p:?}"
        );
    }
    // A lesson starts at its hash, exactly: high 32 bits → x, low 32 bits → y.
    let h = fnv1a("l-x");
    let x = ((h >> 32) as f64 / 4_294_967_296.0).clamp(POS_MIN, POS_MAX);
    let y = ((h & 0xffff_ffff) as f64 / 4_294_967_296.0).clamp(POS_MIN, POS_MAX);
    assert_eq!(start[1], (x, y));
    // Order-independent.
    nodes.reverse();
    let again = start_positions(&nodes, fnv1a);
    assert_eq!(again[2], start[1]);
}

#[test]
fn coincident_ids_are_separated() {
    // A constructed hash under which `l-a` and `l-b` collide; `l-c` does not.
    fn colliding(id: &str) -> u64 {
        if id == "l-c" {
            0x4000_0000_4000_0000
        } else {
            0x8000_0000_8000_0000
        }
    }
    let nodes = [
        node("l-b", GraphNodeKind::Lesson),
        node("l-a", GraphNodeKind::Lesson),
        node("l-c", GraphNodeKind::Lesson),
    ];
    let start = start_positions(&nodes, colliding);
    // `l-a` is first in id order and keeps the hashed point; `l-b` is nudged off it by
    // the first spiral step, exactly `NUDGE` away (within rounding, 1e-15).
    assert_eq!(start[1], (0.5, 0.5));
    assert_ne!(start[0], start[1]);
    assert!((distance(start[0], start[1]) - NUDGE).abs() < 1e-15);
    assert_eq!(start[2], (0.25, 0.25));
    // And they end apart.
    let laid = layout_with(&nodes, &[], colliding);
    let min = 0.5 / 3f64.sqrt();
    assert!(distance(laid.positions[0], laid.positions[1]) >= min);
}
