//! M9.0.7.4, decision 30: the title without ` rev N `, a user turn led by the sequence
//! separator `›`, and no `0.0s` duration.

use super::*;

#[test]
fn the_title_has_no_rev() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        main_conversation(),
    );
    let title = row_text(&draw(&app, 80, 24), 0);
    assert!(title.contains("orchestrator · claude-opus-5"), "{title}");
    assert!(!title.contains("rev"), "{title}");
    assert!(!title.contains("214"), "{title}");
    descend_twice(&mut app);
    let title = row_text(&draw(&app, 80, 24), 0);
    assert!(title.contains("orchestrator › Explore › Review"), "{title}");
    assert!(!title.contains("rev"), "{title}");
}

#[test]
fn a_user_turn_leads_with_the_separator() {
    let conv = conversation(
        None,
        3,
        vec![turn(1, Role::User, 36_000, vec![text("fix the bug")])],
    );
    let app = app_showing(UiSettings::default(), Runtime::Claude, 80, 24, conv.clone());
    let out = text_of(&draw(&app, 80, 24));
    assert!(out.contains("› fix the bug"), "{out}");
    assert!(!out.contains("▸ fix the bug"), "{out}");
    let header = out.lines().find(|l| l.contains("10:00")).unwrap();
    assert_eq!(
        header.trim_start_matches('│').split_whitespace().next(),
        Some("you"),
        "{header}"
    );
    let app = app_showing(ascii_settings(), Runtime::Claude, 80, 24, conv);
    let out = text_of(&draw(&app, 80, 24));
    assert!(out.contains("> fix the bug"), "{out}");
}

#[test]
fn zero_durations_are_hidden() {
    let call = |ms| {
        conversation(
            None,
            3,
            vec![turn(
                1,
                Role::Assistant,
                36_000,
                vec![tool(
                    "Read",
                    "src/lib.rs",
                    json!({"file_path": "src/lib.rs"}),
                    ToolState::Ok,
                    Some(ms),
                    None,
                )],
            )],
        )
    };
    let read_row = |ms| {
        let app = app_showing(UiSettings::default(), Runtime::Claude, 80, 24, call(ms));
        let buf = draw(&app, 80, 24);
        all_rows(&buf)
            .into_iter()
            .find(|r| r.contains("Read"))
            .unwrap_or_else(|| panic!("no Read row:\n{}", text_of(&buf)))
    };
    let fast = read_row(20);
    assert!(!fast.contains("0.0s"), "{fast}");
    assert!(fast.trim_end_matches(['│', ' ']).ends_with('✓'), "{fast}");
    assert!(!read_row(0).contains("0.0s"));
    assert!(!read_row(49).contains("0.0s"));
    assert!(read_row(50).contains("✓ 0.1s"));
    let slow = read_row(1200);
    assert!(slow.contains("✓ 1.2s"), "{slow}");
}

/// Final fix wave M3: one selection idiom. The selected row is reversed and the `▌` bar
/// (`>` in ASCII) in the accent stands in the column left of it, the frame's left
/// border, as a graph box's (decision 20) and the compact list's.
#[test]
fn the_selected_row_wears_the_selection_bar() {
    for settings in [UiSettings::default(), ascii_settings()] {
        let ascii = settings.badges.ascii;
        let app = app_showing(settings, Runtime::Claude, 80, 24, main_conversation());
        let buf = draw(&app, 80, 24);
        let reversed = |y: u16| buf[(5, y)].modifier.contains(Modifier::REVERSED);
        let y = (1..23).find(|&y| reversed(y)).expect("a selected row");
        let bar = if ascii { ">" } else { "▌" };
        assert_eq!(buf[(0, y)].symbol(), bar, "ascii {ascii}");
        let accent = theme::role(theme::Role::Accent, app.palette()).fg;
        assert_eq!(Some(buf[(0, y)].fg), accent);
        let side = if ascii { "|" } else { "│" };
        for other in (1..23).filter(|&o| o != y) {
            assert_eq!(buf[(0, other)].symbol(), side, "row {other}");
        }
    }
}

/// Final fix wave M7: the search's bottom title is muted, as every right or bottom
/// title is, never the accented border's colour.
#[test]
fn the_search_title_is_muted() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        main_conversation(),
    );
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('q'));
    let buf = draw(&app, 80, 24);
    let (x, y) = find(&buf, "/q").expect("the search title");
    assert_eq!(y, 23);
    let muted = theme::role(theme::Role::Muted, app.palette()).fg;
    for cx in x..x + 2 {
        assert_eq!(Some(buf[(cx, y)].fg), muted, "column {cx}");
    }
}

/// The cells drawing `▌` in a whole frame, as `(x, y, accented)`.
fn bars(app: &App, w: u16, h: u16) -> Vec<(u16, u16, bool)> {
    let buf = crate::ui::audit::draw(app, w, h);
    let accent = theme::role(theme::Role::Accent, app.palette()).fg;
    let muted = theme::role(theme::Role::Muted, app.palette()).fg;
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let cell = &buf[(x, y)];
            if cell.symbol() == "▌" {
                let fg = Some(cell.fg);
                assert!(fg == accent || fg == muted, "({x}, {y}) {:?}", cell.fg);
                out.push((x, y, fg == accent));
            }
        }
    }
    out
}

/// Follow-up to the final fix wave's M3 (decisions 1 and 20): the selection bar wears
/// the accent only where the keys are, `Muted` elsewhere, the row still reversed.
#[test]
fn the_bar_is_accented_only_where_the_keys_are() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        main_conversation(),
    );
    // The conversation has the keys: its bar is the one accented.
    let shown = bars(&app, 80, 24);
    assert_eq!(shown.iter().filter(|b| b.2).count(), 1, "{shown:?}");
    // `C-b t` with the view open: the tree is selected too, but the view keeps the
    // keys (the keymap's precedence, `App::key_region`), so the tree's bar is muted.
    key(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('t'), KeyModifiers::NONE);
    assert!(app.tree_input.is_some());
    assert_eq!(
        app.key_region(),
        crate::app::region::KeyRegion::Conversation
    );
    let shown = bars(&app, 80, 24);
    assert_eq!(shown.len(), 2, "both selections are drawn: {shown:?}");
    let accented: Vec<_> = shown.iter().filter(|b| b.2).collect();
    assert_eq!(accented.len(), 1, "{shown:?}");
    assert!(
        accented[0].0 > 0,
        "the conversation's, not the sidebar's: {shown:?}"
    );
    // The help over both: neither bar is accented.
    key(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);
    assert!(app.modal.is_some());
    let shown = bars(&app, 80, 24);
    assert!(!shown.is_empty(), "a bar beside the help");
    assert!(shown.iter().all(|b| !b.2), "{shown:?}");
}
