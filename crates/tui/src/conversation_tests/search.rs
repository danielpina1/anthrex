//! `/` search: scope, case, typing, hit walking, and `Esc` precedence.

use super::*;

#[test]
fn search_matches_prose_and_summaries_only() {
    let mut view = open_with(needle_conversation());
    // Unfold every call so a matcher that searched detail rows would find more.
    for block in 1..=3 {
        view.set_cursor(Cursor::Block(21, block));
        press(&mut view, KeyCode::Enter);
    }
    search_for(&mut view, "needle");
    let search = view.search().unwrap();
    assert!(!search.typing);
    assert_eq!(
        search.hits,
        vec![Cursor::Block(21, 0), Cursor::Block(21, 1)]
    );

    let rows = view.rows();
    let hit_rows: Vec<Row> = search
        .hits
        .iter()
        .map(|hit| {
            let mut probe = view.clone();
            probe.set_cursor(hit.clone());
            rows[probe.selected_row(&rows).unwrap()].clone()
        })
        .collect();
    assert!(matches!(
        hit_rows[0],
        Row::Text {
            turn_id: 21,
            block: 0,
            ..
        }
    ));
    assert_eq!(
        hit_rows[1],
        Row::Tool {
            turn_id: 21,
            block: 1
        }
    );
}

#[test]
fn search_is_case_insensitive_and_n_wraps() {
    let mut view = open_with(needle_conversation());
    search_for(&mut view, "NEEDLE");
    assert_eq!(
        view.search().unwrap().hits,
        vec![Cursor::Block(21, 0), Cursor::Block(21, 1)]
    );
    // Committing moves the cursor to the first hit.
    assert_eq!(view.cursor(), Some(&Cursor::Block(21, 0)));
    assert!(press(&mut view, KeyCode::Char('n')).is_empty());
    assert_eq!(view.cursor(), Some(&Cursor::Block(21, 1)));
    press(&mut view, KeyCode::Char('n'));
    assert_eq!(
        view.cursor(),
        Some(&Cursor::Block(21, 0)),
        "n wraps to the first"
    );
    press(&mut view, KeyCode::Char('N'));
    assert_eq!(
        view.cursor(),
        Some(&Cursor::Block(21, 1)),
        "N wraps to the last"
    );
    press(&mut view, KeyCode::Char('N'));
    assert_eq!(view.cursor(), Some(&Cursor::Block(21, 0)));
}

#[test]
fn typing_edits_the_query_and_recomputes_hits() {
    let mut view = open_with(needle_conversation());
    press(&mut view, KeyCode::Char('/'));
    type_str(&mut view, "lexerz");
    assert!(view.search().unwrap().hits.is_empty());
    press(&mut view, KeyCode::Backspace);
    assert_eq!(view.search().unwrap().query, "lexer");
    assert_eq!(view.search().unwrap().hits, vec![Cursor::Block(21, 0)]);
    // `n` while typing is a character of the query, not a hit walk.
    press(&mut view, KeyCode::Char('n'));
    assert_eq!(view.search().unwrap().query, "lexern");
    // Esc while typing leaves search entirely and keeps the cursor.
    view.set_cursor(Cursor::Block(21, 2));
    assert!(press(&mut view, KeyCode::Esc).is_empty());
    assert_eq!(view.search(), None);
    assert!(view.is_open());
    assert_eq!(view.cursor(), Some(&Cursor::Block(21, 2)));
}

#[test]
fn esc_leaves_search_before_it_leaves_the_view() {
    let mut view = open_with(main_conversation());
    press(&mut view, KeyCode::Char('G'));
    press(&mut view, KeyCode::Enter);
    view.on_snapshot(WINDOW, Some("agent-b".into()), sub_agent("agent-b", None));
    search_for(&mut view, "looking");
    assert_eq!(view.search().unwrap().hits, vec![Cursor::Block(31, 0)]);

    assert!(press(&mut view, KeyCode::Esc).is_empty());
    assert_eq!(view.search(), None);
    assert_eq!(view.trail().len(), 1);
    assert_eq!(view.cursor(), Some(&Cursor::Block(31, 0)));

    assert_eq!(
        press(&mut view, KeyCode::Esc),
        vec![unsubscribe(Some("agent-b"))]
    );
    assert!(view.trail().is_empty());
    assert_eq!(press(&mut view, KeyCode::Esc), vec![unsubscribe(None)]);
    assert!(!view.is_open());
}
