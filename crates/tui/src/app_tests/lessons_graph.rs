//! Milestone 9.15 decisions 46 to 49 (task M9.15.15): the lessons graph page's state and
//! keys. `g` on the Lessons page opens it and asks for the graph; `Enter` inspects;
//! `d`, `a` and `r` remove, approve and reject (an answer acts on the id it asked
//! about); after any reply the graph is asked again, the camera and the selection
//! kept; `Esc` returns to the Profile screen's Lessons page. No test sleeps: replies
//! are delivered by id.

use super::*;
use crate::app::screens::Screen;
use crate::app::screens::lessons_graph::{GraphAsk, LessonsGraphScreen, NOT_REMOVABLE, Panel};
use crate::lessons_graph::fixture::{
    self, APPROVED, BILLING, KEPT, KNOWN, PROPOSED, ROOT, TRIAL, WEB,
};
use proto::LessonGraph;

fn gs(app: &App) -> &LessonsGraphScreen {
    match &app.screen {
        Some(Screen::LessonsGraph(s)) => s,
        other => panic!("no graph screen: {:?}", other.is_some()),
    }
}

fn gs_mut(app: &mut App) -> &mut LessonsGraphScreen {
    match &mut app.screen {
        Some(Screen::LessonsGraph(s)) => s,
        _ => panic!("no graph screen"),
    }
}

fn graph_reply(app: &mut App, id: u64, graph: LessonGraph) -> Vec<Effect> {
    app.on_daemon(DaemonMsg::Run(RunReply::LessonsGraph {
        graph: Box::new(graph),
        request_id: Some(id),
    }))
}

/// The page open on the Lessons page's screen, its first graph delivered.
fn graph_app() -> App {
    let mut app = page_app();
    let (id, _) = one(&tap(&mut app, KeyCode::Char('g')));
    graph_reply(&mut app, id, fixture::graph());
    app
}

fn select(app: &mut App, id: &str) {
    gs_mut(app).selected = Some(id.into());
}

fn graph_request(effects: &[Effect]) -> u64 {
    let (id, request) = one(effects);
    assert_eq!(request, LessonsRequest::Graph { dir: dir() });
    id
}

#[test]
fn g_opens_and_requests_the_graph() {
    let mut app = page_app();
    let id = graph_request(&tap(&mut app, KeyCode::Char('g')));
    let s = gs(&app);
    assert!(s.graph.is_none(), "loading until the graph comes");
    assert_eq!(s.dir, dir());
    assert!(matches!(s.back.page, Some(ProfilePage::Lessons(_))));
    graph_reply(&mut app, id, fixture::graph());
    let s = gs(&app);
    assert_eq!(s.graph.as_ref().map(|g| g.nodes.len()), Some(8));
    assert_eq!(s.positions.len(), 8);
    assert_eq!(s.layouts, 1);
    assert!(!app.replies.contains(id));
    // Not connected: a toast, no page.
    let mut app = page_app();
    app.on_link_lost("gone");
    assert!(tagged(&tap(&mut app, KeyCode::Char('g'))).is_empty());
    assert_eq!(app.toast_text(), Some("not connected"));
    assert!(matches!(app.screen, Some(Screen::Profile(_))));
}

/// Task 14's review, M5 (the controller's ruling): the layout runs exactly once per
/// graph reply, never on a key or a render.
#[test]
fn the_layout_runs_once_per_graph_reply() {
    let mut app = graph_app();
    assert_eq!(gs(&app).layouts, 1);
    for code in [
        KeyCode::Char('+'),
        KeyCode::Char('-'),
        KeyCode::Char('l'),
        KeyCode::Char('0'),
        KeyCode::Right,
        KeyCode::Down,
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::Char('/'),
        KeyCode::Char('w'),
        KeyCode::Enter,
        KeyCode::Char('n'),
    ] {
        tap(&mut app, code);
    }
    for (w, h) in [(80, 24), (160, 48), (80, 24)] {
        crate::ui::audit::draw(&app, w, h);
    }
    let layout = crate::ui::layout_for(&app, ratatui::layout::Rect::new(0, 0, 80, 24));
    app.on_click(layout.lessons_canvas.x, layout.lessons_canvas.y, &layout);
    assert_eq!(
        gs(&app).layouts,
        1,
        "keys, renders and clicks lay out nothing"
    );
    // A stale graph reply (not the latest request) does not lay out either.
    select(&mut app, PROPOSED);
    let (act, _) = one(&tap(&mut app, KeyCode::Char('a')));
    let first = graph_request(&done(&mut app, act, "approved 1"));
    let (act, _) = one(&tap(&mut app, KeyCode::Char('a')));
    let latest = graph_request(&done(&mut app, act, "approved 1"));
    graph_reply(&mut app, first, fixture::graph());
    assert_eq!(gs(&app).layouts, 1, "a superseded graph is dropped");
    graph_reply(&mut app, latest, fixture::graph());
    assert_eq!(gs(&app).layouts, 2);
}

#[test]
fn enter_opens_the_panel_for_a_lesson_and_for_a_module() {
    let mut app = graph_app();
    tap(&mut app, KeyCode::Enter);
    assert_eq!(gs(&app).panel, None, "nothing selected: no panel");
    select(&mut app, APPROVED);
    tap(&mut app, KeyCode::Enter);
    assert_eq!(gs(&app).panel, Some(Panel::default()));
    assert_eq!(
        gs(&app).lesson(APPROVED).map(|l| l.id.as_str()),
        Some(APPROVED)
    );
    tap(&mut app, KeyCode::Esc);
    assert_eq!(gs(&app).panel, None, "Esc closes the panel first");
    select(&mut app, BILLING);
    tap(&mut app, KeyCode::Enter);
    assert_eq!(gs(&app).panel, Some(Panel::default()));
    assert_eq!(gs(&app).module_rows(BILLING), [KNOWN, TRIAL, PROPOSED]);
    assert_eq!(gs(&app).module_rows(ROOT), [KEPT]);
    assert_eq!(gs(&app).module_rows(WEB), [APPROVED]);
}

#[test]
fn panel_list_enter_selects_that_node() {
    let mut app = graph_app();
    select(&mut app, BILLING);
    tap(&mut app, KeyCode::Enter);
    tap(&mut app, KeyCode::Tab);
    assert_eq!(gs(&app).panel.map(|p| p.focus), Some(true));
    let camera = gs(&app).camera;
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Char('j'));
    assert_eq!(
        gs(&app).panel.map(|p| p.row),
        Some(2),
        "stops at the last row"
    );
    tap(&mut app, KeyCode::Char('k'));
    assert_eq!(gs(&app).camera, camera, "j/k in the panel do not pan");
    tap(&mut app, KeyCode::Enter);
    assert_eq!(gs(&app).selected.as_deref(), Some(TRIAL));
    assert!(
        gs(&app).panel.is_some(),
        "the panel now inspects that lesson"
    );
    // Tab gives the keys back to the canvas: j pans again.
    tap(&mut app, KeyCode::Tab);
    tap(&mut app, KeyCode::Char('j'));
    assert!(gs(&app).camera.cy > camera.cy);
}

#[test]
fn d_on_a_lesson_asks_then_sends_forget() {
    let mut app = graph_app();
    select(&mut app, APPROVED);
    assert!(tagged(&tap(&mut app, KeyCode::Char('d'))).is_empty());
    let ask = gs(&app).question.clone().expect("a question");
    assert_eq!(
        ask.text(),
        "forget l-33333333 \"Templates live under web/templates\"? y/n"
    );
    // Keys other than the answer do nothing meanwhile.
    assert!(tagged(&tap(&mut app, KeyCode::Char('a'))).is_empty());
    // The selection moves under the question (as a reply could move it): the answer
    // still acts on the lesson it asked about.
    select(&mut app, KEPT);
    let (id, request) = one(&tap(&mut app, KeyCode::Char('y')));
    assert_eq!(
        request,
        LessonsRequest::Forget {
            dir: dir(),
            ids: vec![APPROVED.into()],
        }
    );
    assert_eq!(gs(&app).question, None);
    select(&mut app, APPROVED);
    let again = graph_request(&done(&mut app, id, "forgot 1"));
    assert_eq!(gs(&app).message.as_deref(), Some("forgot 1"));
    let mut gone = fixture::graph();
    gone.nodes.retain(|n| n.id != APPROVED);
    gone.edges.retain(|e| e.from != APPROVED);
    gone.lessons.retain(|l| l.id != APPROVED);
    graph_reply(&mut app, again, gone);
    assert_eq!(gs(&app).selected, None, "the forgotten node is gone");
    // `n` cancels.
    select(&mut app, KEPT);
    tap(&mut app, KeyCode::Char('d'));
    assert!(tagged(&tap(&mut app, KeyCode::Char('n'))).is_empty());
    assert_eq!(gs(&app).question, None);
}

#[test]
fn d_on_a_known_failure_asks_dismiss() {
    let mut app = graph_app();
    select(&mut app, KNOWN);
    tap(&mut app, KeyCode::Char('d'));
    let ask = gs(&app).question.clone().expect("a question");
    assert_eq!(ask, GraphAsk::Dismiss { id: KNOWN.into() });
    assert_eq!(ask.text(), "dismiss known failure kf-0badf00d? y/n");
    let (_, request) = one(&tap(&mut app, KeyCode::Char('y')));
    assert_eq!(
        request,
        LessonsRequest::Forget {
            dir: dir(),
            ids: vec![KNOWN.into()],
        }
    );
}

#[test]
fn d_on_a_module_toasts() {
    for hub in [BILLING, WEB, ROOT] {
        let mut app = graph_app();
        select(&mut app, hub);
        assert!(tagged(&tap(&mut app, KeyCode::Char('d'))).is_empty());
        assert_eq!(app.toast_text(), Some(NOT_REMOVABLE));
        assert_eq!(app.toast_text(), Some("a module cannot be removed"));
        assert_eq!(gs(&app).question, None);
    }
}

#[test]
fn a_and_r_on_a_proposed_lesson() {
    for id in [PROPOSED, TRIAL] {
        let mut app = graph_app();
        select(&mut app, id);
        let (_, request) = one(&tap(&mut app, KeyCode::Char('a')));
        let decide = |approve| LessonsRequest::Decide {
            dir: dir(),
            ids: vec![id.into()],
            approve,
        };
        assert_eq!(request, decide(true));
        assert!(tagged(&tap(&mut app, KeyCode::Char('r'))).is_empty());
        assert_eq!(gs(&app).question, Some(GraphAsk::Reject { id: id.into() }));
        assert_eq!(
            gs(&app).question.as_ref().unwrap().text(),
            format!("reject {id}? y/n")
        );
        let (_, request) = one(&tap(&mut app, KeyCode::Char('y')));
        assert_eq!(request, decide(false));
    }
    // An approved, kept or known lesson, and a hub: nothing to decide.
    for id in [APPROVED, KEPT, KNOWN, BILLING] {
        let mut app = graph_app();
        select(&mut app, id);
        assert!(
            tagged(&tap(&mut app, KeyCode::Char('a'))).is_empty(),
            "{id}"
        );
        assert!(
            tagged(&tap(&mut app, KeyCode::Char('r'))).is_empty(),
            "{id}"
        );
        assert_eq!(gs(&app).question, None, "{id}");
    }
}

#[test]
fn a_reply_rerequests_and_keeps_camera_and_selection() {
    let mut app = graph_app();
    select(&mut app, TRIAL);
    tap(&mut app, KeyCode::Enter);
    for code in [KeyCode::Char('+'), KeyCode::Char('+'), KeyCode::Char('l')] {
        tap(&mut app, code);
    }
    let camera = gs(&app).camera;
    let positions = gs(&app).positions.clone();
    let (id, _) = one(&tap(&mut app, KeyCode::Char('a')));
    let again = graph_request(&done(&mut app, id, "approved 1"));
    graph_reply(&mut app, again, fixture::graph());
    let s = gs(&app);
    assert_eq!(s.camera, camera);
    assert_eq!(s.selected.as_deref(), Some(TRIAL));
    assert_eq!(s.panel, Some(Panel::default()));
    assert_eq!(s.positions, positions, "the same graph lays out the same");
    assert_eq!(s.layouts, 2);
    // A refusal shows on the page in the daemon's words, and the graph is asked again.
    let (id, _) = one(&tap(&mut app, KeyCode::Char('a')));
    let words =
        "l-11111111 is approved; only a proposed or on-trial lesson can be approved or rejected";
    graph_request(&refused(&mut app, id, words));
    assert_eq!(gs(&app).error.as_deref(), Some(words));
    // A refused graph shows too, and is not asked again (no loop).
    let (id, _) = one(&tap(&mut app, KeyCode::Char('a')));
    let again = graph_request(&done(&mut app, id, "approved 1"));
    assert!(refused(&mut app, again, "not a git repository").is_empty());
    assert_eq!(gs(&app).error.as_deref(), Some("not a git repository"));
    assert_eq!(
        gs(&app).selected.as_deref(),
        Some(TRIAL),
        "the old graph stays"
    );
}

#[test]
fn esc_returns_to_the_profile_screen_lessons_page() {
    let mut app = graph_app();
    tap(&mut app, KeyCode::Char('/'));
    typed(&mut app, "web");
    tap(&mut app, KeyCode::Enter);
    select(&mut app, WEB);
    tap(&mut app, KeyCode::Enter);
    tap(&mut app, KeyCode::Esc);
    assert!(!gs(&app).search.active(), "the first Esc clears the query");
    assert!(gs(&app).panel.is_some());
    tap(&mut app, KeyCode::Esc);
    assert_eq!(gs(&app).panel, None, "the second closes the panel");
    let effects = tap(&mut app, KeyCode::Esc);
    let s = screen(&app);
    assert!(matches!(s.page, Some(ProfilePage::Lessons(_))));
    assert_eq!(s.dir, dir());
    // The page lists again: the graph may have changed it.
    let (id, request) = one(&effects);
    assert_eq!(request, list(false));
    lessons_reply(&mut app, id, three());
    assert_eq!(page(&app).lessons.as_ref().map(Vec::len), Some(3));
    // An outcome that comes after the page closed is toasted.
    let mut app = graph_app();
    select(&mut app, PROPOSED);
    let (id, _) = one(&tap(&mut app, KeyCode::Char('a')));
    tap(&mut app, KeyCode::Esc);
    assert!(done(&mut app, id, "approved 1").is_empty());
    assert_eq!(app.toast_text(), Some("approved 1"));
}

#[test]
fn selection_lights_neighbours_and_dims_the_rest() {
    let mut app = graph_app();
    let ids: Vec<String> = gs(&app)
        .graph
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .map(|n| n.id.clone())
        .collect();
    let lit = |app: &App| -> Vec<String> {
        (ids.iter().zip(gs(app).emphasis()))
            .filter(|(_, l)| *l)
            .map(|(id, _)| id.clone())
            .collect()
    };
    assert_eq!(lit(&app).len(), 8, "no selection: everything lit");
    select(&mut app, PROPOSED);
    // Its hub (touches), the lesson sharing files, and the known failure it came from.
    assert_eq!(lit(&app), [BILLING, KNOWN, TRIAL, PROPOSED]);
    select(&mut app, WEB);
    assert_eq!(lit(&app), [WEB, APPROVED]);
    // A filter in force lights its matches (and the selection) instead.
    tap(&mut app, KeyCode::Char('/'));
    typed(&mut app, "changelog");
    assert_eq!(lit(&app), [WEB, KEPT]);
    tap(&mut app, KeyCode::Enter);
    tap(&mut app, KeyCode::Char('n'));
    assert_eq!(
        gs(&app).selected.as_deref(),
        Some(KEPT),
        "n selects the match"
    );
    assert_eq!(lit(&app), [KEPT]);
}

/// Decision 46: arrows walk the graph; a click selects within one cell, else clears.
#[test]
fn arrows_and_clicks_select() {
    let mut app = graph_app();
    tap(&mut app, KeyCode::Right);
    let first = gs(&app)
        .selected
        .clone()
        .expect("an arrow selects the node nearest the centre");
    let s = gs(&app);
    let (cx, cy) = (s.camera.cx, s.camera.cy);
    let d = |p: (f64, f64)| (p.0 - cx).powi(2) + (p.1 - cy).powi(2);
    let nearest = (0..8)
        .min_by(|a, b| d(s.positions[*a]).total_cmp(&d(s.positions[*b])))
        .unwrap();
    assert_eq!(s.graph.as_ref().unwrap().nodes[nearest].id, first);
    // A click on a node's cell selects it.
    let layout = crate::ui::layout_for(&app, ratatui::layout::Rect::new(0, 0, 160, 48));
    let canvas = layout.lessons_canvas;
    assert!(canvas.width > 0 && canvas.height > 0);
    let view = crate::ui::lessons_graph::view(gs(&app), canvas);
    let i = gs(&app).index_of(WEB).unwrap();
    let (c, r) = view.cell(gs(&app).positions[i]).expect("web is in view");
    app.on_click(c, r, &layout);
    assert_eq!(gs(&app).selected.as_deref(), Some(WEB));
    tap(&mut app, KeyCode::Enter);
    // A click on empty canvas clears the selection and the panel.
    let cells: Vec<(u16, u16)> = gs(&app)
        .positions
        .iter()
        .filter_map(|p| view.cell(*p))
        .collect();
    let empty = (canvas.x..canvas.x + canvas.width)
        .flat_map(|x| (canvas.y..canvas.y + canvas.height).map(move |y| (x, y)))
        .find(|(x, y)| {
            cells
                .iter()
                .all(|(c, r)| c.abs_diff(*x) > 1 || r.abs_diff(*y) > 1)
        })
        .expect("an empty cell");
    app.on_click(empty.0, empty.1, &layout);
    assert_eq!(gs(&app).selected, None);
    assert_eq!(gs(&app).panel, None);
}

/// A graph request outstanding: approving `TRIAL` asks the graph again.
fn pending_graph(app: &mut App) -> u64 {
    select(app, TRIAL);
    let (act, _) = one(&tap(app, KeyCode::Char('a')));
    graph_request(&done(app, act, "approved 1"))
}

/// `fixture::graph()` without the nodes `ids`, their edges and their details.
fn graph_without(ids: &[&str]) -> LessonGraph {
    let mut graph = fixture::graph();
    graph.nodes.retain(|n| !ids.contains(&n.id.as_str()));
    (graph.edges).retain(|e| !ids.contains(&e.from.as_str()) && !ids.contains(&e.to.as_str()));
    graph.lessons.retain(|l| !ids.contains(&l.id.as_str()));
    graph
}

/// Task 15's review, M2: a graph reply that no longer holds the node a question names
/// drops the question (quietly), so `y` cannot act on a node that is not drawn; a
/// reply that still holds it keeps the question.
#[test]
fn a_reply_without_the_asked_node_drops_its_question() {
    let mut app = graph_app();
    let again = pending_graph(&mut app);
    select(&mut app, APPROVED);
    tap(&mut app, KeyCode::Char('d'));
    assert!(gs(&app).question.is_some());
    let next = pending_graph_under_question(&mut app);
    graph_reply(&mut app, again, fixture::graph());
    assert!(gs(&app).question.is_some(), "its node is still drawn");
    assert!(graph_reply(&mut app, next, graph_without(&[APPROVED])).is_empty());
    assert_eq!(gs(&app).question, None);
    assert!(tagged(&tap(&mut app, KeyCode::Char('y'))).is_empty());
}

/// A second graph request while a question is open: every key answers a question, so
/// the question is set aside for the action and put back.
fn pending_graph_under_question(app: &mut App) -> u64 {
    let ask = gs_mut(app).question.take();
    let id = pending_graph(app);
    gs_mut(app).question = ask;
    id
}

/// Task 15's review, M3: the panel's row starts again at 0 when the selection changes,
/// and a reply that shortens the list clamps it, so the marked row is the one `Enter`
/// acts on.
#[test]
fn the_panel_row_resets_on_a_new_selection_and_clamps_on_a_reply() {
    let mut app = graph_app();
    let again = pending_graph(&mut app);
    select(&mut app, BILLING);
    tap(&mut app, KeyCode::Enter);
    tap(&mut app, KeyCode::Tab);
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Char('j'));
    assert_eq!(gs(&app).panel.map(|p| p.row), Some(2));
    // Billing keeps one lesson: the row is clamped to it, and Enter selects it.
    graph_reply(&mut app, again, graph_without(&[KNOWN, PROPOSED]));
    assert_eq!(gs(&app).module_rows(BILLING), [TRIAL]);
    assert_eq!(gs(&app).panel.map(|p| p.row), Some(0));
    tap(&mut app, KeyCode::Enter);
    assert_eq!(gs(&app).selected.as_deref(), Some(TRIAL));

    // An arrow, `n` and a click that move the selection each start the row at 0.
    let mut app = graph_app();
    let moved = |app: &mut App, how: &dyn Fn(&mut App)| {
        select(app, BILLING);
        tap(app, KeyCode::Enter);
        gs_mut(app).panel = Some(Panel {
            focus: false,
            row: 2,
        });
        how(app);
        assert_ne!(gs(app).selected.as_deref(), Some(BILLING));
        assert_eq!(gs(app).panel.map(|p| p.row), Some(0));
    };
    moved(&mut app, &|app| {
        let before = gs(app).selected.clone();
        for code in [KeyCode::Right, KeyCode::Left, KeyCode::Up, KeyCode::Down] {
            tap(app, code);
            if gs(app).selected != before {
                break;
            }
        }
    });
    moved(&mut app, &|app| {
        tap(app, KeyCode::Char('/'));
        for c in "invoice".chars() {
            tap(app, KeyCode::Char(c));
        }
        tap(app, KeyCode::Enter);
        tap(app, KeyCode::Char('n'));
        assert_eq!(gs(app).selected.as_deref(), Some(PROPOSED));
        tap(app, KeyCode::Esc);
    });
    moved(&mut app, &|app| {
        let layout = crate::ui::layout_for(app, ratatui::layout::Rect::new(0, 0, 160, 48));
        let canvas = layout.lessons_canvas;
        let s = gs(app);
        let view = crate::ui::lessons_graph::view(s, canvas);
        let at = s.index_of(KEPT).unwrap();
        let (x, y) = view.cell(s.positions[at]).expect("KEPT is on the canvas");
        app.on_click(x, y, &layout);
    });
}
