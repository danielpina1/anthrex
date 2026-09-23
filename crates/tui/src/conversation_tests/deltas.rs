//! What a delta carries besides turns (decision A4), and how the cursor and search hits
//! keep pointing at the same block across one.

use super::*;

#[test]
fn a_delta_carries_the_dropped_and_degraded_rows_and_the_session() {
    let mut view = open_with(main_conversation());
    view.on_delta(
        WINDOW,
        None,
        5,
        6,
        vec![TurnPatch::Drop { id: 11 }],
        Some("sess-b".into()),
        Some(DegradeReason::Misaligned),
        1,
        Some(DropCause::Turns),
    );
    let rows = view.rows();
    assert_eq!(
        rows[0],
        Row::Dropped {
            count: 1,
            cause: DropCause::Turns
        }
    );
    assert_eq!(
        rows.last(),
        Some(&Row::Degraded {
            reason: DegradeReason::Misaligned
        })
    );
    assert_eq!(
        view.conversation().unwrap().session_id.as_deref(),
        Some("sess-b")
    );

    // A second delta changes all three again: a different count, cause and session,
    // and the degradation cleared.
    view.on_delta(
        WINDOW,
        None,
        6,
        7,
        vec![],
        Some("sess-c".into()),
        None,
        2,
        Some(DropCause::Bytes),
    );
    let rows = view.rows();
    assert_eq!(
        rows[0],
        Row::Dropped {
            count: 2,
            cause: DropCause::Bytes
        }
    );
    assert!(!matches!(rows.last(), Some(Row::Degraded { .. })));
    assert_eq!(
        view.conversation().unwrap().session_id.as_deref(),
        Some("sess-c")
    );
}

fn edit_without_input() -> Turn {
    let mut shrunk = assistant_turn();
    if let Block::ToolCall { input, .. } = &mut shrunk.blocks[2] {
        *input = None;
    }
    shrunk
}

#[test]
fn a_cursor_on_a_vanished_detail_line_stays_on_its_block() {
    let mut view = open_with(main_conversation());
    view.set_cursor(Cursor::Block(12, 2));
    press(&mut view, KeyCode::Enter);
    for _ in 0..3 {
        press(&mut view, KeyCode::Char('j'));
    }
    assert_eq!(view.cursor(), Some(&Cursor::Line(12, 2, 3)));

    // The Edit call's input goes away: its detail shrinks to the result summary alone.
    assert!(delta(&mut view, 5, vec![TurnPatch::Upsert(edit_without_input())]).is_empty());
    let (_, row) = selected(&view);
    assert!(
        matches!(
            row,
            Row::ToolDetail {
                turn_id: 12,
                block: 2,
                ..
            }
        ),
        "the selection left the Edit call: {row:?}"
    );
    assert!(
        press(&mut view, KeyCode::Enter).is_empty(),
        "Enter must not descend into the spawn below"
    );
    assert_eq!(view.trail(), []);
}

#[test]
fn a_cursor_on_a_folded_blocks_line_resolves_to_its_tool_row() {
    let mut view = open_with(main_conversation());
    view.set_cursor(Cursor::Line(12, 2, 3));
    assert_eq!(
        selected(&view).1,
        Row::Tool {
            turn_id: 12,
            block: 2
        }
    );
    // Enter folds or unfolds that call; it never descends.
    assert!(press(&mut view, KeyCode::Enter).is_empty());
    assert_eq!(view.trail(), []);
    assert!(view.is_unfolded(12, 2));
}

fn two_needle_turns() -> Conversation {
    let mut conv = needle_conversation();
    conv.turns.push(turn(
        22,
        Role::User,
        3_100,
        vec![text("thread the needle again")],
    ));
    conv
}

#[test]
fn the_current_hit_follows_its_block_across_a_delta() {
    let mut view = open_with(two_needle_turns());
    search_for(&mut view, "needle");
    assert_eq!(
        view.search().unwrap().hits,
        vec![
            Cursor::Block(21, 0),
            Cursor::Block(21, 1),
            Cursor::Block(22, 0)
        ]
    );
    press(&mut view, KeyCode::Char('N'));
    assert_eq!(view.cursor(), Some(&Cursor::Block(22, 0)));

    // Turn 21 grows a matching block, which becomes a hit ahead of the current one.
    let mut grown = needle_conversation().turns.remove(0);
    grown.blocks.push(text("one more needle"));
    assert!(delta(&mut view, 5, vec![TurnPatch::Upsert(grown)]).is_empty());

    let search = view.search().unwrap();
    assert_eq!(
        search.hits,
        vec![
            Cursor::Block(21, 0),
            Cursor::Block(21, 1),
            Cursor::Block(21, 5),
            Cursor::Block(22, 0)
        ]
    );
    assert_eq!(search.hits[search.hit], Cursor::Block(22, 0));
    press(&mut view, KeyCode::Char('N'));
    assert_eq!(view.cursor(), Some(&Cursor::Block(21, 5)));
}

#[test]
fn when_every_hit_is_dropped_n_stays_put() {
    let mut view = open_with(needle_conversation());
    search_for(&mut view, "needle");
    assert_eq!(view.cursor(), Some(&Cursor::Block(21, 0)));
    assert!(delta(&mut view, 5, vec![TurnPatch::Drop { id: 21 }]).is_empty());
    assert_eq!(view.search().unwrap().hits, vec![]);

    press(&mut view, KeyCode::Char('n'));
    press(&mut view, KeyCode::Char('N'));
    assert_eq!(
        view.cursor(),
        Some(&Cursor::Block(21, 0)),
        "n/N must not walk onto a dropped block"
    );
}
