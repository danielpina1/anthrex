//! Milestone 9.15 decisions 45, 48 and 50 (task M9.15.15): the graph page drawn whole
//! (ratatui `TestBackend`) at 80×24 and 160×48 from the fixed eight-node graph: each
//! node's glyph in its role at the cell the canvas prints it, the hub labels, the
//! title, the hint row and decision 43's footer; the inspect panel on the side at
//! 160×48 and in the lower half at 80×24; ASCII; and a hostile label on one line.

use super::*;
use crate::app::screens::Screen;
use crate::app::screens::lessons_graph::{GraphAsk, LessonsGraphScreen, Panel};
use crate::lessons_graph::fixture::{self, APPROVED, BILLING, KNOWN, PROPOSED, ROOT, WEB};
use crate::ui::audit;
use crate::ui::profile::tests::sp_app;
use proto::LessonGraph;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

fn app_with(ascii: bool, graph: LessonGraph) -> App {
    let mut app = sp_app(ascii);
    let Some(Screen::Profile(back)) = app.screen.take() else {
        panic!("no profile screen");
    };
    let mut s = LessonsGraphScreen::new(back);
    s.install(graph);
    app.screen = Some(Screen::LessonsGraph(Box::new(s)));
    app
}

fn gs(app: &App) -> &LessonsGraphScreen {
    match &app.screen {
        Some(Screen::LessonsGraph(s)) => s,
        _ => panic!("no graph screen"),
    }
}

fn gs_mut(app: &mut App) -> &mut LessonsGraphScreen {
    match &mut app.screen {
        Some(Screen::LessonsGraph(s)) => s,
        _ => panic!("no graph screen"),
    }
}

/// The canvas's projection in a `w`×`h` frame, as the draw computes it.
fn view_at(app: &App, w: u16, h: u16) -> View {
    let layout = crate::ui::layout_for(app, Rect::new(0, 0, w, h));
    view(gs(app), layout.lessons_canvas)
}

/// The symbol and foreground at node `id`'s cell.
fn at_node(app: &App, buffer: &Buffer, v: View, id: &str) -> (String, Color) {
    let s = gs(app);
    let i = s.index_of(id).expect("a node");
    let (x, y) = v.cell(s.positions[i]).expect("in view at zoom 1");
    let cell = &buffer[(x, y)];
    (cell.symbol().to_string(), cell.fg)
}

fn fg(r: Option<Role>, p: Palette) -> Color {
    r.map_or(Color::Reset, |r| role(r, p).fg.unwrap_or(Color::Reset))
}

/// Every node's glyph in its role, the hub labels, the title and the hints.
fn assert_page(app: &App, w: u16, h: u16) {
    let p = app.palette();
    let ascii = p.ascii;
    let buffer = audit::draw(app, w, h);
    let rows = audit::rows(&buffer);
    let all = rows.join("\n");
    let v = view_at(app, w, h);
    let g = |u: &'static str, a: &'static str| if ascii { a } else { u };
    let want: [(&str, &str, Option<Role>); 8] = [
        (ROOT, g("◉", "O"), None),
        (BILLING, g("◉", "O"), None),
        (WEB, g("◉", "O"), None),
        (KNOWN, g("•", "x"), Some(Role::Failed)),
        (fixture::TRIAL, g("•", "*"), Some(Role::Muted)),
        (PROPOSED, g("○", "o"), Some(Role::Attention)),
        (APPROVED, g("•", "*"), None),
        (fixture::KEPT, g("•", "*"), None),
    ];
    for (id, glyph, r) in want {
        let (symbol, colour) = at_node(app, &buffer, v, id);
        assert_eq!(symbol, glyph, "{id} at {w}x{h} (ascii {ascii})\n{all}");
        assert_eq!(colour, fg(r, p), "{id}'s role at {w}x{h}");
    }
    for hub in ["billing", "web", "(root)"] {
        let label = format!("{} {hub}", g("◉", "O"));
        assert!(!audit::find(&buffer, &label).is_empty(), "{label}\n{all}");
    }
    let dot = if ascii { "-" } else { "·" };
    assert!(all.contains(&format!("Lessons {dot} shop")), "{all}");
    assert!(all.contains("esc back"), "{all}");
    assert!(all.contains("search"), "{all}");
    // Lessons are not labelled at zoom 1 with nothing selected.
    assert!(!all.contains("Templates live"), "{all}");
    if ascii {
        assert_eq!(non_ascii(&buffer), None, "{all}");
    }
}

/// Decision 50: in ASCII mode every cell is ASCII but the edges' Braille.
fn non_ascii(buffer: &Buffer) -> Option<(u16, u16, String)> {
    let area = buffer.area;
    (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
        .map(|(x, y)| (x, y, buffer[(x, y)].symbol().to_string()))
        .find(|(_, _, s)| {
            s.chars()
                .any(|c| !c.is_ascii() && !('\u{2800}'..='\u{28ff}').contains(&c))
        })
}

#[test]
fn renders_the_graph_at_80x24_and_160x48() {
    for ascii in [false, true] {
        let app = app_with(ascii, fixture::graph());
        assert_page(&app, 80, 24);
        assert_page(&app, 160, 48);
    }
}

#[test]
fn the_footer_names_what_is_not_drawn() {
    let mut graph = fixture::graph();
    graph.omitted = 7;
    let app = app_with(false, graph);
    for (w, h) in [(80, 24), (160, 48)] {
        let rows = audit::rows(&audit::draw(&app, w, h));
        let footer = "7 more lessons not drawn; anthrex lessons list shows them all";
        let at = rows.iter().position(|r| r.contains(footer));
        let hints = rows.iter().position(|r| r.contains("esc back"));
        assert!(at.is_some(), "{}", rows.join("\n"));
        assert_eq!(
            at.map(|a| a + 1),
            hints,
            "the footer sits above the hint row"
        );
    }
    let mut modules = fixture::graph();
    modules
        .nodes
        .retain(|n| n.kind != proto::GraphNodeKind::Lesson);
    modules.edges.clear();
    modules.lessons.clear();
    let app = app_with(false, modules);
    let all = audit::rows(&audit::draw(&app, 80, 24)).join("\n");
    assert!(all.contains("no lessons yet; modules only"), "{all}");
}

#[test]
fn the_panel_is_on_the_side_at_160x48() {
    let mut app = app_with(false, fixture::graph());
    gs_mut(&mut app).selected = Some(APPROVED.into());
    gs_mut(&mut app).panel = Some(Panel::default());
    let buffer = audit::draw(&app, 160, 48);
    let rows = audit::rows(&buffer);
    let all = rows.join("\n");
    let (x, y) = audit::find(&buffer, "inspect")[0];
    assert!(
        x >= 160 - 1 - PANEL_WIDTH,
        "the panel is on the right: {x}\n{all}"
    );
    assert!(y <= 2, "and full height: {y}");
    for text in [
        "Templates live under web/templates",
        "procedural",
        "approved",
        "The body of Templates live under",
        "It has a second line.",
        "crates/web/templates/**",
        "r-0001, r-0002",
        "3/4",
        "2025-10-13",
    ] {
        let at = audit::find(&buffer, text);
        assert!(!at.is_empty(), "{text}\n{all}");
        // The title is at the panel's left edge plus two (corner, space).
        assert!(at[0].0 >= x - 1, "{text} is inside the panel");
    }
    // The selected lesson is labelled on the canvas, in the accent.
    let v = view_at(&app, 160, 48);
    let (_, _) = at_node(&app, &buffer, v, APPROVED);
    // A known failure's panel shows its signature headline.
    gs_mut(&mut app).selected = Some(KNOWN.into());
    let all = audit::rows(&audit::draw(&app, 160, 48)).join("\n");
    assert!(all.contains("error[E0308]"), "{all}");
}

#[test]
fn the_panel_takes_the_lower_half_at_80x24() {
    let mut app = app_with(false, fixture::graph());
    gs_mut(&mut app).selected = Some(BILLING.into());
    gs_mut(&mut app).panel = Some(Panel {
        focus: true,
        row: 1,
    });
    let buffer = audit::draw(&app, 80, 24);
    let all = audit::rows(&buffer).join("\n");
    let (x, y) = audit::find(&buffer, "inspect")[0];
    assert!(x < 10, "the panel spans the width: {x}\n{all}");
    assert!((10..=13).contains(&y), "the lower half: {y}\n{all}");
    // A module's lessons and known failures, as a list, the panel's row selected.
    for title in [
        "cargo test fails on a type mismatch",
        "Run migrations before tests",
        "Prefer the Invoice builder",
    ] {
        let at = audit::find(&buffer, title);
        assert!(at.first().is_some_and(|(_, ty)| *ty > y), "{title}\n{all}");
    }
    let (tx, ty) = audit::find(&buffer, "Run migrations before tests")[0];
    assert!(
        buffer[(tx, ty)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED),
        "the panel's row is selected"
    );
    let canvas = crate::ui::layout_for(&app, Rect::new(0, 0, 80, 24)).lessons_canvas;
    assert!(
        canvas.y + canvas.height <= y,
        "the canvas is above it: {canvas:?}"
    );
}

#[test]
fn the_hint_row_holds_a_question_or_the_query() {
    let mut app = app_with(false, fixture::graph());
    gs_mut(&mut app).question = Some(GraphAsk::Dismiss { id: KNOWN.into() });
    let buffer = audit::draw(&app, 80, 24);
    let text = "dismiss known failure kf-0badf00d? y/n";
    let (x, y) = audit::find(&buffer, text)[0];
    assert_eq!(buffer[(x, y)].fg, fg(Some(Role::Failed), app.palette()));
    gs_mut(&mut app).question = None;
    gs_mut(&mut app).search.open();
    gs_mut(&mut app).search.query = "Invoice".into();
    let all = audit::rows(&audit::draw(&app, 80, 24)).join("\n");
    assert!(all.contains("/Invoice"), "{all}");
}

#[test]
fn a_hostile_label_is_drawn_on_one_line() {
    let hostile = crate::safe_text::tests::hostile_text();
    let mut graph = fixture::graph();
    for node in &mut graph.nodes {
        if node.id == WEB {
            node.label = format!("we\nb\x1b[31m{hostile}");
        }
    }
    for lesson in &mut graph.lessons {
        if lesson.id == APPROVED {
            lesson.title = format!("tit\nle {hostile}");
            lesson.body = format!("bo\x1b]0;x\x07dy {hostile}");
            lesson.files = vec![hostile.clone()];
        }
    }
    for ascii in [false, true] {
        let mut app = app_with(ascii, graph.clone());
        gs_mut(&mut app).selected = Some(APPROVED.into());
        gs_mut(&mut app).panel = Some(Panel::default());
        gs_mut(&mut app).error = Some(hostile.clone());
        for (w, h) in [(80, 24), (160, 48)] {
            let buffer = audit::draw(&app, w, h);
            let rows = audit::rows(&buffer);
            let all = rows.join(" ");
            assert_eq!(crate::safe_text::tests::first_hostile(&all), None, "{all}");
            let hub = if ascii {
                "O we b [31m"
            } else {
                "◉ we b [31m"
            };
            assert!(
                rows.iter().any(|r| r.contains(hub)),
                "{hub}\n{}",
                rows.join("\n")
            );
            if ascii {
                assert_eq!(non_ascii(&buffer), None, "{all}");
            }
        }
        let canvas_label = label(&format!("we\nb\x1b[31m{hostile}"), app.palette());
        assert!(canvas_label.chars().count() <= LABEL_MAX);
    }
}

/// Labels: lessons only for the selected node, or for every visible node at zoom 2.
#[test]
fn lesson_labels_follow_selection_and_zoom() {
    let mut app = app_with(false, fixture::graph());
    gs_mut(&mut app).selected = Some(PROPOSED.into());
    let all = audit::rows(&audit::draw(&app, 160, 48)).join("\n");
    // Cut to 24 characters (26 long), drawn last so it stays on top.
    assert!(all.contains("○ Prefer the Invoice buil…"), "{all}");
    assert!(!all.contains("Keep the CHANGELOG"), "{all}");
    gs_mut(&mut app).selected = None;
    let all = audit::rows(&audit::draw(&app, 160, 48)).join("\n");
    assert!(
        !all.contains("Prefer the Invoice"),
        "unselected at zoom 1: no label\n{all}"
    );
    // Centred on the proposed lesson at zoom 2 and more: labelled, unselected.
    let i = gs(&app).index_of(PROPOSED).unwrap();
    let (x, y) = gs(&app).positions[i];
    let camera = &mut gs_mut(&mut app).camera;
    (camera.cx, camera.cy) = (x, y);
    while camera.zoom() < LABEL_ZOOM {
        camera.zoom_in();
    }
    let all = audit::rows(&audit::draw(&app, 160, 48)).join("\n");
    assert!(all.contains("○ Prefer the Invoice buil…"), "{all}");
}
