//! Decision 46's tables: arrows, zoom, pan, recentre and click. Pure; no clock.

use super::*;

/// A plus: the centre `c`, an arm each way at distance 0.2, and decoys.
fn plus() -> Vec<(&'static str, Point)> {
    vec![
        ("c", (0.5, 0.5)),
        ("e", (0.7, 0.5)),
        ("w", (0.3, 0.5)),
        ("n", (0.5, 0.3)),
        ("s", (0.5, 0.7)),
        // Far out on the east arm's line: in the cone, but further than `e`.
        ("ee", (0.9, 0.5)),
    ]
}

fn id_of(points: &[(&'static str, Point)], i: Option<usize>) -> Option<&'static str> {
    i.map(|i| points[i].0)
}

#[test]
fn nearest_dot_per_direction() {
    let points = plus();
    for (dir, want) in [
        (Dir::Right, "e"),
        (Dir::Left, "w"),
        (Dir::Up, "n"),
        (Dir::Down, "s"),
    ] {
        assert_eq!(
            id_of(&points, nearest(&points, 0, dir)),
            Some(want),
            "{dir:?}"
        );
    }
    // A node at 44° from the right is in the cone, one at 46° is not, whatever its
    // distance.
    let at = |deg: f64, r: f64| {
        let a = deg.to_radians();
        (0.5 + r * a.cos(), 0.5 - r * a.sin())
    };
    let points = vec![("c", (0.5, 0.5)), ("in", at(44.0, 0.3))];
    assert_eq!(id_of(&points, nearest(&points, 0, Dir::Right)), Some("in"));
    let points = vec![("c", (0.5, 0.5)), ("out", at(46.0, 0.01))];
    assert_eq!(nearest(&points, 0, Dir::Right), None);
    assert_eq!(id_of(&points, nearest(&points, 0, Dir::Up)), Some("out"));
    // Two at the same distance (exact binary fractions, so the distances are equal
    // bit for bit): the smaller id, whatever the input order.
    for points in [
        vec![
            ("c", (0.5, 0.5)),
            ("b", (0.75, 0.375)),
            ("a", (0.75, 0.625)),
        ],
        vec![
            ("c", (0.5, 0.5)),
            ("a", (0.75, 0.625)),
            ("b", (0.75, 0.375)),
        ],
    ] {
        assert_eq!(id_of(&points, nearest(&points, 0, Dir::Right)), Some("a"));
    }
}

#[test]
fn nothing_in_the_cone_keeps_the_selection() {
    let points = vec![("c", (0.5, 0.5)), ("w", (0.2, 0.5)), ("n", (0.5, 0.1))];
    assert_eq!(nearest(&points, 0, Dir::Right), None);
    assert_eq!(nearest(&points, 0, Dir::Down), None);
    // A lone node, and a node exactly on the selection, are never chosen.
    assert_eq!(nearest(&[("c", (0.5, 0.5))], 0, Dir::Left), None);
    let same = vec![("c", (0.5, 0.5)), ("d", (0.5, 0.5))];
    for dir in [Dir::Left, Dir::Right, Dir::Up, Dir::Down] {
        assert_eq!(nearest(&same, 0, dir), None);
    }
}

#[test]
fn zoom_steps_and_bounds() {
    let steps = zoom_steps();
    assert_eq!(steps.first(), Some(&ZOOM_MIN));
    assert_eq!(steps.last(), Some(&ZOOM_MAX));
    assert_eq!(steps[ZOOM_ONE], 1.0);
    for pair in steps[1..steps.len() - 1].windows(2) {
        assert!((pair[1] / pair[0] - ZOOM_STEP).abs() < 1e-12, "{pair:?}");
    }
    assert!(steps.windows(2).all(|p| p[0] < p[1]));
    let mut camera = Camera::default();
    assert_eq!(camera.zoom(), 1.0);
    camera.zoom_in();
    assert_eq!(camera.zoom(), ZOOM_STEP);
    camera.zoom_out();
    assert_eq!(camera.zoom(), 1.0, "in then out returns exactly");
    for _ in 0..40 {
        camera.zoom_in();
    }
    assert_eq!(camera.zoom(), ZOOM_MAX);
    for _ in 0..80 {
        camera.zoom_out();
    }
    assert_eq!(camera.zoom(), ZOOM_MIN);
}

#[test]
fn pan_is_a_tenth_of_the_span() {
    let mut camera = Camera::default();
    camera.pan(1, 0);
    assert!((camera.cx - 0.6).abs() < 1e-12, "{camera:?}");
    camera.pan(0, -1);
    assert!((camera.cy - 0.4).abs() < 1e-12, "{camera:?}");
    // At zoom 2 the span is a half, so a pan is 0.05.
    let mut camera = Camera::default();
    while camera.zoom() < 2.0 {
        camera.zoom_in();
    }
    let span = camera.span();
    camera.pan(-1, 1);
    assert!((camera.cx - (0.5 - span / 10.0)).abs() < 1e-12);
    assert!((camera.cy - (0.5 + span / 10.0)).abs() < 1e-12);
    // The centre stays in the unit square.
    let mut camera = Camera::default();
    for _ in 0..100 {
        camera.pan(1, 1);
    }
    assert_eq!((camera.cx, camera.cy), (1.0, 1.0));
}

#[test]
fn zero_recentres() {
    let mut camera = Camera::default();
    camera.zoom_in();
    camera.zoom_in();
    camera.pan(1, -1);
    camera.recentre();
    assert_eq!(camera, Camera::default());
    assert_eq!((camera.zoom(), camera.cx, camera.cy), (1.0, 0.5, 0.5));
}

#[test]
fn click_selects_within_one_cell_else_clears() {
    let cells = [
        ("b", Some((10, 5))),
        ("a", Some((12, 5))),
        ("far", Some((30, 9))),
        ("hidden", None),
    ];
    assert_eq!(pick(&cells, (10, 5)), Some(0), "on the glyph");
    assert_eq!(pick(&cells, (9, 6)), Some(0), "a diagonal neighbour");
    assert_eq!(
        pick(&cells, (11, 5)),
        Some(1),
        "equidistant: the smaller id"
    );
    assert_eq!(pick(&cells, (31, 10)), Some(2));
    assert_eq!(pick(&cells, (20, 5)), None, "nothing within one cell");
    assert_eq!(pick(&cells, (10, 7)), None, "two rows away");
    assert_eq!(pick(&[], (0, 0)), None);
}

/// The projection keeps the unit square's aspect and agrees with ratatui's label
/// placement at the centre and the corners of the view.
#[test]
fn the_view_keeps_cell_aspect() {
    let view = View {
        x: 2,
        y: 1,
        width: 81,
        height: 21,
        camera: Camera::default(),
    };
    let (xb, yb) = view.bounds().unwrap();
    // 80 columns wide and 20 rows tall: 40 columns per unit (2 × 20), so the view is
    // 2 units wide and 1 unit tall.
    assert!((xb[1] - xb[0] - 2.0).abs() < 1e-12, "{xb:?}");
    assert!((yb[1] - yb[0] - 1.0).abs() < 1e-12, "{yb:?}");
    assert_eq!(view.cell((0.5, 0.5)), Some((2 + 40, 1 + 10)));
    assert_eq!(view.cell((0.5, 0.0)), Some((42, 1)));
    assert_eq!(view.cell((0.5, 1.0)), Some((42, 21)));
    assert_eq!(view.cell((0.5, 1.2)), None);
    let tiny = View { width: 1, ..view };
    assert_eq!(tiny.bounds(), None);
    assert_eq!(tiny.cell((0.5, 0.5)), None);
}
