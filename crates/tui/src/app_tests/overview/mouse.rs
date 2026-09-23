//! Every mouse gesture over the graph, and what the canvas does under one:
//! clicks and double clicks hit-test against the frame's own geometry, the
//! wheel and a drag pan it, and neither a window list nor a degenerate
//! terminal may move it out from under the person driving it.

use super::*;

#[test]
fn hit_testing_selects_on_click() {
    let (mut app, layout) = opened_at(200, 50);
    let key = NodeKey::Window(4);
    let (x, y) = box_middle(&app, layout.main, &key);
    assert!(
        app.on_click(x, y, &layout).is_empty(),
        "a click only selects"
    );
    assert_eq!(app.tree.selected, Some(key));
    assert_eq!(app.focused, Some(1), "the focus did not move");
    assert!(app.overview, "the overview stays open");

    // A click on a gap between boxes selects nothing and changes nothing.
    let view = overview::view(&app, layout.main);
    let gap = (
        view.canvas.x + crate::graph::MIN_NODE_WIDTH + 1,
        view.canvas.y + view.canvas.height - 1,
    );
    assert_eq!(
        view.geometry().node_at(&view.layout, gap.0, gap.1),
        None,
        "the fixture's bottom-left corner should be empty canvas"
    );
    assert!(app.on_click(gap.0, gap.1, &layout).is_empty());
    assert_eq!(app.tree.selected, Some(NodeKey::Window(4)));
    assert!(app.overview);
}

#[test]
fn a_double_click_focuses() {
    let (mut app, layout) = opened_at(200, 50);
    let (x, y) = box_middle(&app, layout.main, &NodeKey::Window(4));
    assert!(app.on_click(x, y, &layout).is_empty());
    assert_eq!(
        app.on_click(x, y, &layout),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 4,
            cols: 80,
            rows: 24,
        })]
    );
    assert_eq!(app.focused, Some(4));
    assert_closed(&app);

    // A project toggles instead, and a third press starts a fresh gesture
    // rather than firing again.
    let (mut app, layout) = opened_at(200, 50);
    let project = NodeKey::Project("/r/shop".into());
    let (x, y) = box_middle(&app, layout.main, &project);
    assert!(app.on_click(x, y, &layout).is_empty());
    assert!(app.on_click(x, y, &layout).is_empty());
    assert!(app.tree.is_collapsed(&project));
    assert!(app.on_click(x, y, &layout).is_empty());
    assert!(
        app.tree.is_collapsed(&project),
        "the third press is a first"
    );
}

#[test]
fn the_wheel_scrolls_vertically_by_three() {
    let (mut app, layout) = opened();
    let canvas = overview::view(&app, layout.main).canvas;
    let (x, y) = (canvas.x + 1, canvas.y + 1);
    assert!(app.on_scroll(false, x, y, &layout).is_empty());
    assert_eq!(app.graph_pan, Pan { x: 0, y: 3 });
    assert!(app.on_scroll(false, x, y, &layout).is_empty());
    assert_eq!(app.graph_pan, Pan { x: 0, y: 6 });
    assert!(app.on_scroll(true, x, y, &layout).is_empty());
    assert_eq!(app.graph_pan, Pan { x: 0, y: 3 });
    // A same-size draw must not snap the pan back to the selection.
    app.set_graph_viewport(layout.main);
    assert_eq!(app.graph_pan, Pan { x: 0, y: 3 });
    // The wheel over the sidebar is still the sidebar's, not the canvas's.
    assert!(
        app.on_scroll(false, 2, layout.sidebar_list.y, &layout)
            .is_empty()
    );
    assert_eq!(app.graph_pan, Pan { x: 0, y: 3 });
}

/// The daemon publishes a window list on every status flip and every output
/// event, several times a second while agents work. A list that changes
/// nothing the view depends on must leave the pan alone, or the canvas snaps
/// back to the selection as fast as a person can scroll away from it
/// (decision 16, spec §4.4).
#[test]
fn only_a_real_change_in_the_window_list_reveals() {
    let (mut app, layout) = opened();
    let canvas = overview::view(&app, layout.main).canvas;
    let (x, y) = (canvas.x + 1, canvas.y + 1);
    assert!(app.on_scroll(false, x, y, &layout).is_empty());
    assert!(app.on_scroll(false, x, y, &layout).is_empty());
    let scrolled = Pan { x: 0, y: 6 };
    assert_eq!(app.graph_pan, scrolled);

    assert!(
        app.on_daemon(DaemonMsg::WindowsChanged {
            windows: app.windows.clone(),
        })
        .is_empty()
    );
    assert_eq!(
        app.graph_pan, scrolled,
        "an identical window list must not move the view"
    );

    // A list with one more window changes the visible rows, so the reveal
    // fires and pulls the selection back inside the viewport.
    let mut extra = app.windows[0].clone();
    extra.id = 9;
    extra.name = "extra".into();
    extra.subagents.clear();
    let mut grown = app.windows.clone();
    grown.push(extra);
    assert!(
        app.on_daemon(DaemonMsg::WindowsChanged { windows: grown })
            .is_empty()
    );
    assert_ne!(app.graph_pan, scrolled, "a changed row list reveals");
    let view = overview::view(&app, layout.main);
    let selected = app.tree.selected.clone().expect("the overview selects");
    let rect = view.layout.node(&selected).expect("and places it").rect;
    assert!(
        rect.y >= view.pan.y && rect.bottom() <= view.pan.y + view.canvas.height,
        "{rect:?} is not wholly inside the viewport at {:?}",
        view.pan
    );
}

#[test]
fn dragging_pans_both_axes() {
    let (mut app, layout) = opened();
    let canvas = overview::view(&app, layout.main).canvas;
    // Well inside the canvas, which the inspector panel took rows from
    // (milestone 4.7): a press outside it is not a drag anchor at all, and the
    // drag below would then be measuring nothing.
    let (x, y) = (canvas.x + 20, canvas.y + 10);
    assert!(app.on_click(x, y, &layout).is_empty());
    // Dragging up and to the left pulls the canvas with the cursor, so the
    // viewport moves down and to the right.
    assert!(app.on_drag(x - 8, y - 5, &layout).is_empty());
    assert_eq!(app.graph_pan, Pan { x: 8, y: 5 });
    assert!(app.on_drag(x - 10, y - 6, &layout).is_empty());
    assert_eq!(app.graph_pan, Pan { x: 10, y: 6 });
    // Dragging back the other way returns it, and the canvas edge holds.
    assert!(app.on_drag(x + 40, y + 40, &layout).is_empty());
    assert_eq!(app.graph_pan, Pan::default());

    // A press outside the canvas ends the gesture, so the next drag has no
    // anchor to pan from and leaves the canvas where it is.
    assert!(app.on_click(2, layout.sidebar_footer.y, &layout).is_empty());
    assert!(app.on_drag(x - 6, y - 6, &layout).is_empty());
    assert_eq!(app.graph_pan, Pan::default());
}

#[test]
fn overview_clicks_ignore_modals_borders_and_the_sidebar_stays_live() {
    let (mut app, layout) = opened_at(200, 50);
    let inside = box_middle(&app, layout.main, &NodeKey::Window(4));
    for (x, y) in [
        (layout.main.x, inside.1),
        (layout.main.right() - 1, inside.1),
        (inside.0, layout.main.y),
        (inside.0, layout.main.bottom() - 1),
    ] {
        assert!(app.on_click(x, y, &layout).is_empty());
        assert_eq!(app.tree.selected, Some(NodeKey::Window(1)));
    }
    // The sidebar is still clickable while the overview is open.
    assert_eq!(
        app.on_click(2, layout.sidebar_list.y + 5, &layout),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 2,
            cols: 80,
            rows: 24,
        })]
    );
    assert_eq!(app.focused, Some(2));

    // A viewport too small for the canvas, so a real drag actually moves the
    // pan (at 200x50 the whole canvas fits and `Pan::clamped` pins every pan
    // back to zero regardless, which would make the guard below untestable —
    // the same coordinates `dragging_pans_both_axes` below uses to move the
    // pan to `Pan { x: 8, y: 5 }` when nothing blocks the drag).
    let (mut app, layout) = opened();
    let canvas = overview::view(&app, layout.main).canvas;
    let (x, y) = (canvas.x + 20, canvas.y + 10);
    // Press inside the canvas first, while there is no modal, so `drag_from`
    // holds a real anchor. With a fresh app `drag_from` is `None` regardless,
    // and `on_drag`'s anchor check alone would return empty whether or not
    // the modal guard below exists — this earlier press is what makes the
    // guard load-bearing.
    assert!(app.on_click(x, y, &layout).is_empty());
    let selected = app.tree.selected.clone();
    app.modal = Some(Modal::Help);
    assert!(app.on_click(x, y, &layout).is_empty());
    assert!(app.on_drag(x - 8, y - 5, &layout).is_empty());
    assert!(app.on_scroll(false, x, y, &layout).is_empty());
    // The drag never ran: with a real anchor in place, only the modal guard
    // kept the pan from moving. The selection is unchanged by the modal
    // clicks, whatever the real click above landed on.
    assert_eq!(app.graph_pan, Pan::default());
    assert_eq!(app.tree.selected, selected);
}

#[test]
fn the_overview_click_works_with_a_filter_and_a_hidden_sidebar() {
    let (mut app, _) = opened_at(200, 50);
    app.sidebar_visible = false;
    let layout = crate::ui::layout(Rect::new(0, 0, 200, 50), 0);
    app.set_graph_viewport(layout.main);
    press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
    assert!(app.on_paste("billing".into()).is_empty());
    assert_eq!(app.tree_input, Some(TreeInput::Filter));
    let (x, y) = box_middle(&app, layout.main, &NodeKey::Window(2));
    assert!(app.on_click(x, y, &layout).is_empty());
    assert_eq!(
        app.on_click(x, y, &layout),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 2,
            cols: 80,
            rows: 24,
        })]
    );
    assert_closed(&app);
}

/// The aligned-row overview had `wide_overview_stays_inside_tiny_main_areas`;
/// the graph replaced that renderer and nothing replaced the guard. Drawing,
/// clicking, dragging and scrolling at every degenerate size — with pans far
/// past any canvas, and with the sidebar both ways — must not panic, and
/// whatever the graph draws must stay inside its own block.
#[test]
fn the_graph_stays_inside_tiny_main_areas() {
    for sidebar_visible in [false, true] {
        let (mut app, _) = opened_at(200, 50);
        app.sidebar_visible = sidebar_visible;
        // Every width from a single column to sixty, and the heights either
        // side of the ones that change the shape: no block, a block with no
        // interior, a canvas one row tall with no footer, and room for both.
        for width in 1..=60 {
            for height in [1, 2, 3, 4, 5, 9, 24] {
                let sidebar_width = if sidebar_visible {
                    app.sidebar_width
                } else {
                    0
                };
                let layout = crate::ui::layout(Rect::new(0, 0, width, height), sidebar_width);
                app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
                app.set_graph_viewport(layout.main);
                for pan in [Pan::default(), Pan { x: 60000, y: 60000 }] {
                    app.graph_pan = pan;
                    let buffer = drawn(&app, width, height);
                    let main = layout.main;
                    if main.width >= 2 && main.height >= 3 {
                        for y in main.y + 1..main.bottom() - 1 {
                            assert_eq!(
                                buffer[(main.right() - 1, y)].symbol(),
                                "│",
                                "{width}x{height} at {pan:?}: the graph crossed its own \
                                 right border on row {y}"
                            );
                        }
                    }
                    for (x, y) in [
                        (0, 0),
                        (width / 2, height / 2),
                        (width - 1, height - 1),
                        (main.x, main.y),
                    ] {
                        app.on_click(x, y, &layout);
                        app.on_drag(x.saturating_sub(3), y.saturating_add(2), &layout);
                        app.on_scroll(true, x, y, &layout);
                        app.on_scroll(false, x, y, &layout);
                    }
                    // A double click on a node leaves the overview; reopen it
                    // so the next size is drawn as a graph and not a terminal.
                    if !app.overview {
                        assert!(toggle(&mut app).is_empty());
                    }
                }
            }
        }
    }
}

/// A conversation for window 1 with enough prose rows for the wheel to move through.
fn prose_snapshot() -> DaemonMsg {
    DaemonMsg::ConversationSnapshot {
        window_id: 1,
        agent_id: None,
        conversation: proto::Conversation {
            window_id: 1,
            agent_id: None,
            session_id: None,
            runtime: proto::Runtime::Shell,
            rev: 3,
            degraded: None,
            dropped_turns: 0,
            dropped_by: None,
            turns: vec![proto::Turn {
                id: 30,
                role: proto::Role::Assistant,
                at_unix_secs: 0,
                state: proto::TurnState::Complete,
                blocks: (0..10)
                    .map(|i| proto::Block::Text {
                        text: format!("line {i}"),
                    })
                    .collect(),
            }],
        },
    }
}

/// Review M3: the open conversation view covers the main area, so nothing under it —
/// the overview's graph, the terminal's scrollback, or an agent in mouse mode — gets a
/// click, a double click, a drag or the wheel. The wheel moves the view's cursor.
#[test]
fn the_mouse_never_reaches_what_the_conversation_view_covers() {
    let (mut app, layout) = opened_at(200, 50);
    assert_eq!(app.focused, Some(1));
    app.toggle_conversation();
    assert!(app.conversation.is_open());
    app.on_daemon(prose_snapshot());
    let selected = app.tree.selected.clone();
    let pan = app.graph_pan;

    // A double click over a box the overview would have focused.
    let (x, y) = box_middle(&app, layout.main, &NodeKey::Window(4));
    assert!(app.on_click(x, y, &layout).is_empty());
    assert!(app.on_click(x, y, &layout).is_empty());
    assert_eq!(app.focused, Some(1), "a double click moved the focus");
    assert_eq!(
        app.tree.selected, selected,
        "a click selected a hidden node"
    );
    assert!(app.on_drag(x + 5, y + 3, &layout).is_empty());
    assert_eq!(app.graph_pan, pan, "a drag panned the hidden graph");

    // The wheel moves the view's cursor, three rows a notch, and nothing else.
    app.conversation
        .set_cursor(crate::conversation::Cursor::Turn(30));
    assert!(app.on_scroll(false, x, y, &layout).is_empty());
    assert_eq!(app.graph_pan, pan, "the wheel panned the hidden graph");
    assert_eq!(
        app.conversation.cursor(),
        Some(&crate::conversation::Cursor::Block(30, 2))
    );
    assert!(app.on_scroll(true, x, y, &layout).is_empty());
    assert_eq!(
        app.conversation.cursor(),
        Some(&crate::conversation::Cursor::Turn(30))
    );

    // Over the terminal instead, with the agent in SGR mouse mode: no report is
    // forwarded, and the local scrollback does not move.
    app.overview = false;
    app.parser.process(b"\x1b[?1000h\x1b[?1006h");
    assert_ne!(
        app.parser.screen().mouse_protocol_mode(),
        vt100::MouseProtocolMode::None
    );
    let (x, y) = (layout.main_inner.x + 3, layout.main_inner.y + 3);
    assert!(app.on_scroll(true, x, y, &layout).is_empty());
    assert!(app.on_click(x, y, &layout).is_empty());
    assert!(app.on_click(x, y, &layout).is_empty());
    assert_eq!(app.scroll_offset, 0);
    assert_eq!(app.focused, Some(1));
}

/// Re-review finding: a drag that began on the graph before the view opened must not
/// pan the hidden graph once it is open. The press is made with the view closed, so
/// `drag_from` is still set when the drag arrives and only `on_drag`'s own guard stops
/// it. At 120×30 the example graph is larger than its canvas, so an unguarded drag
/// really does pan (to `Pan { x: 8, y: 5 }`); at 200×50 it fits and the pan clamps to 0.
#[test]
fn a_drag_begun_before_the_view_opened_does_not_pan_the_hidden_graph() {
    let (mut app, layout) = opened();
    let (x, y) = box_middle(&app, layout.main, &NodeKey::Window(4));
    assert!(app.on_click(x, y, &layout).is_empty());
    app.toggle_conversation();
    assert!(app.conversation.is_open());
    let pan = app.graph_pan;
    assert!(app.on_drag(x - 8, y - 5, &layout).is_empty());
    assert_eq!(app.graph_pan, pan, "the drag panned the hidden graph");
}
