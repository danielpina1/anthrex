//! Task M6.5.12's view-state tests. Conversations are built by hand with distinct values
//! throughout — distinct turn ids, texts, tool names and inputs — so a fold, cursor or
//! search keyed by the wrong thing cannot pass by coincidence.

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{Block, ClientMsg, NoticeKind, Role, Runtime, ToolResult, ToolState, Turn, TurnState};
use serde_json::json;

const WINDOW: u32 = 4;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press(view: &mut ConversationView, code: KeyCode) -> Vec<Effect> {
    view.on_key(key(code))
}

fn type_str(view: &mut ConversationView, text: &str) {
    for c in text.chars() {
        assert!(press(view, KeyCode::Char(c)).is_empty());
    }
}

fn text(text: &str) -> Block {
    Block::Text { text: text.into() }
}

fn tool(name: &str, summary: &str, input: serde_json::Value, detail: Option<&str>) -> Block {
    Block::ToolCall {
        id: Some(format!("toolu_{}", name.to_lowercase())),
        name: name.into(),
        summary: summary.into(),
        input: Some(input),
        result: Some(ToolResult {
            ok: true,
            summary: format!("{name} finished"),
            detail: detail.map(str::to_owned),
            truncated: false,
        }),
        state: ToolState::Ok,
        duration_ms: Some(300),
    }
}

fn spawn(agent_id: &str, label: &str) -> Block {
    Block::SubagentSpawn {
        agent_id: agent_id.into(),
        kind: "Explore".into(),
        label: label.into(),
        model: Some("haiku".into()),
    }
}

fn turn(id: u64, role: Role, at: u64, blocks: Vec<Block>) -> Turn {
    Turn {
        id,
        role,
        at_unix_secs: at,
        state: TurnState::Complete,
        blocks,
    }
}

fn conversation(agent_id: Option<&str>, rev: u64, turns: Vec<Turn>) -> Conversation {
    Conversation {
        window_id: WINDOW,
        agent_id: agent_id.map(str::to_owned),
        session_id: Some("sess-a".into()),
        runtime: Runtime::Claude,
        rev,
        degraded: None,
        dropped_turns: 0,
        dropped_by: None,
        turns,
    }
}

fn user_turn() -> Turn {
    turn(11, Role::User, 1_000, vec![text("refactor the parser")])
}

fn assistant_turn() -> Turn {
    turn(
        12,
        Role::Assistant,
        1_060,
        vec![
            text("I'll map the call sites first."),
            tool(
                "Grep",
                "\"parse_\" → 34 matches",
                json!({"pattern": "parse_", "path": "crates/parse"}),
                Some("crates/parse/src/lib.rs:12"),
            ),
            tool(
                "Edit",
                "crates/parse/src/lib.rs",
                json!({
                    "file_path": "crates/parse/src/lib.rs",
                    "old_string": "fn parse_all(src: &str) {",
                    "new_string": "fn parse_all(src: &str) -> Result<Ast> {",
                }),
                None,
            ),
            spawn("agent-b", "find every call site"),
        ],
    )
}

fn main_conversation() -> Conversation {
    conversation(None, 5, vec![user_turn(), assistant_turn()])
}

fn open_with(conv: Conversation) -> ConversationView {
    let mut view = ConversationView::default();
    let effects = view.open(WINDOW);
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::SubscribeConversation {
            window_id: WINDOW,
            agent_id: None,
            from_rev: None,
        })]
    );
    assert!(view.on_snapshot(WINDOW, None, conv).is_empty());
    view
}

fn delta(view: &mut ConversationView, from_rev: u64, patches: Vec<TurnPatch>) -> Vec<Effect> {
    view.on_delta(
        WINDOW,
        None,
        from_rev,
        from_rev + 1,
        patches,
        Some("sess-a".into()),
        None,
        0,
        None,
    )
}

fn selected(view: &ConversationView) -> (usize, Row) {
    let rows = view.rows();
    let index = view.selected_row(&rows).expect("a selected row");
    (index, rows[index].clone())
}

fn subscribe(agent_id: Option<&str>) -> Effect {
    Effect::Send(ClientMsg::SubscribeConversation {
        window_id: WINDOW,
        agent_id: agent_id.map(str::to_owned),
        from_rev: None,
    })
}

fn unsubscribe(agent_id: Option<&str>) -> Effect {
    Effect::Send(ClientMsg::UnsubscribeConversation {
        window_id: WINDOW,
        agent_id: agent_id.map(str::to_owned),
    })
}

#[test]
fn rows_render_a_turn_in_order() {
    let view = open_with(main_conversation());
    assert_eq!(
        view.rows(),
        vec![
            Row::TurnHeader {
                turn_id: 11,
                role: Role::User,
                at_unix_secs: 1_000
            },
            Row::Text {
                turn_id: 11,
                block: 0,
                line: 0,
                text: "refactor the parser".into()
            },
            Row::TurnHeader {
                turn_id: 12,
                role: Role::Assistant,
                at_unix_secs: 1_060
            },
            Row::Text {
                turn_id: 12,
                block: 0,
                line: 0,
                text: "I'll map the call sites first.".into()
            },
            Row::Tool {
                turn_id: 12,
                block: 1
            },
            Row::Tool {
                turn_id: 12,
                block: 2
            },
            Row::Spawn {
                turn_id: 12,
                block: 3,
                agent_id: "agent-b".into()
            },
        ]
    );
}

fn detail_rows_after(rows: &[Row], turn: u64, block: usize) -> Vec<String> {
    let start = rows
        .iter()
        .position(|r| {
            *r == Row::Tool {
                turn_id: turn,
                block,
            }
        })
        .expect("the tool row");
    rows[start + 1..]
        .iter()
        .map_while(|r| match r {
            Row::ToolDetail {
                turn_id,
                block: b,
                text,
                ..
            } => {
                assert_eq!((*turn_id, *b), (turn, block), "detail under the wrong call");
                Some(text.clone())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn unfolding_adds_detail_rows_for_that_block_only() {
    let mut view = open_with(main_conversation());
    view.set_cursor(Cursor::Block(12, 2));
    assert!(press(&mut view, KeyCode::Enter).is_empty());
    assert!(view.is_unfolded(12, 2));
    assert!(!view.is_unfolded(12, 1));

    let rows = view.rows();
    let edit = detail_rows_after(&rows, 12, 2).join("\n");
    assert!(
        edit.contains("fn parse_all(src: &str) -> Result<Ast> {"),
        "{edit}"
    );
    assert!(edit.contains("Edit finished"), "{edit}");
    assert!(
        !edit.contains("crates/parse\""),
        "the Grep input leaked in: {edit}"
    );
    assert!(detail_rows_after(&rows, 12, 1).is_empty());

    // `o` folds it again.
    assert!(press(&mut view, KeyCode::Char('o')).is_empty());
    assert!(!view.is_unfolded(12, 2));
    assert!(detail_rows_after(&view.rows(), 12, 2).is_empty());
}

#[test]
fn the_fold_set_survives_a_turn_being_dropped() {
    let mut view = open_with(main_conversation());
    view.set_cursor(Cursor::Block(12, 1));
    press(&mut view, KeyCode::Enter);
    assert!(delta(&mut view, 5, vec![TurnPatch::Drop { id: 11 }]).is_empty());

    assert!(view.is_unfolded(12, 1));
    assert!(!view.is_unfolded(12, 2));
    let rows = view.rows();
    let grep = detail_rows_after(&rows, 12, 1).join("\n");
    assert!(grep.contains("\"pattern\": \"parse_\""), "{grep}");
    assert!(detail_rows_after(&rows, 12, 2).is_empty());
}

#[test]
fn the_cursor_survives_a_delta_that_inserts_a_turn() {
    let mut view = open_with(main_conversation());
    view.set_cursor(Cursor::Block(12, 1));
    let (before, row) = selected(&view);
    assert_eq!(
        row,
        Row::Tool {
            turn_id: 12,
            block: 1
        }
    );

    // Turns only ever arrive with rising ids, so the new turn 13 lands after 12; the
    // same delta also grows turn 11 by a block, which is what moves every later row.
    let mut grown = user_turn();
    grown.blocks.push(text("and keep the public API stable"));
    let new_turn = turn(13, Role::User, 1_120, vec![text("now run the tests")]);
    assert!(
        delta(
            &mut view,
            5,
            vec![TurnPatch::Upsert(grown), TurnPatch::Upsert(new_turn)]
        )
        .is_empty()
    );

    let (after, row) = selected(&view);
    assert_eq!(
        row,
        Row::Tool {
            turn_id: 12,
            block: 1
        }
    );
    assert_ne!(before, after);
    assert!(view.rows().contains(&Row::TurnHeader {
        turn_id: 13,
        role: Role::User,
        at_unix_secs: 1_120
    }));
}

#[test]
fn the_cursor_moves_forward_when_its_turn_is_dropped() {
    let mut view = open_with(main_conversation());
    view.set_cursor(Cursor::Block(11, 0));
    assert_eq!(
        selected(&view).1,
        Row::Text {
            turn_id: 11,
            block: 0,
            line: 0,
            text: "refactor the parser".into()
        }
    );
    view.on_delta(
        WINDOW,
        None,
        5,
        6,
        vec![TurnPatch::Drop { id: 11 }],
        Some("sess-a".into()),
        None,
        1,
        Some(DropCause::Turns),
    );
    // Decision A4: the delta's own drop count and cause reach the first row.
    assert_eq!(
        view.rows()[0],
        Row::Dropped {
            count: 1,
            cause: DropCause::Turns
        }
    );
    assert_eq!(
        selected(&view).1,
        Row::TurnHeader {
            turn_id: 12,
            role: Role::Assistant,
            at_unix_secs: 1_060
        }
    );
}

/// A sub-agent's conversation. Each agent gets its own turn id and rev, so a mix-up
/// between two trail levels cannot pass by coincidence.
fn sub_agent(agent_id: &str, spawns: Option<(&str, &str)>) -> Conversation {
    let (turn_id, rev) = match agent_id {
        "agent-b" => (31, 2),
        "agent-c" => (41, 3),
        _ => (51, 4),
    };
    let mut blocks = vec![text(&format!("{agent_id} looking around"))];
    if let Some((child, label)) = spawns {
        blocks.push(spawn(child, label));
    }
    conversation(
        Some(agent_id),
        rev,
        vec![turn(turn_id, Role::Assistant, 2_000 + turn_id, blocks)],
    )
}

#[test]
fn a_stale_delta_forces_a_resubscribe() {
    let mut view = open_with(main_conversation());
    assert_eq!(view.rev(), Some(5));
    let rows = view.rows();
    let effects = view.on_delta(
        WINDOW,
        None,
        7,
        8,
        vec![TurnPatch::Drop { id: 11 }],
        Some("sess-a".into()),
        None,
        0,
        None,
    );
    assert_eq!(effects, vec![subscribe(None)]);
    assert_eq!(view.rows(), rows);
    assert_eq!(view.rev(), Some(5));
    // Further deltas in flight before the snapshot arrives are dropped quietly rather
    // than each asking again.
    assert!(delta(&mut view, 8, vec![TurnPatch::Drop { id: 12 }]).is_empty());
    assert_eq!(view.rows(), rows);
}

#[test]
fn a_delta_for_another_key_is_ignored() {
    let mut view = open_with(main_conversation());
    let rows = view.rows();
    let effects = view.on_delta(
        WINDOW,
        Some("agent-z".into()),
        5,
        6,
        vec![TurnPatch::Drop { id: 11 }],
        None,
        Some(DegradeReason::BadRecord),
        0,
        None,
    );
    assert!(effects.is_empty());
    assert_eq!(view.rows(), rows);
    assert_eq!(view.rev(), Some(5));
    // Nor is one for the same agent in another window.
    let effects = view.on_delta(
        WINDOW + 1,
        None,
        5,
        6,
        vec![TurnPatch::Drop { id: 11 }],
        None,
        None,
        0,
        None,
    );
    assert!(effects.is_empty());
    assert_eq!(view.rows(), rows);
}

#[test]
fn a_dropped_row_and_a_degraded_row_bracket_the_list() {
    let mut conv = main_conversation();
    conv.dropped_turns = 3;
    conv.dropped_by = Some(DropCause::Turns);
    conv.degraded = Some(DegradeReason::BadRecord);
    let view = open_with(conv);
    let rows = view.rows();
    assert_eq!(
        rows[0],
        Row::Dropped {
            count: 3,
            cause: DropCause::Turns
        }
    );
    assert_eq!(
        rows.last(),
        Some(&Row::Degraded {
            reason: DegradeReason::BadRecord
        })
    );
    assert_eq!(rows.len(), 9);
}

fn needle_conversation() -> Conversation {
    conversation(
        None,
        5,
        vec![turn(
            21,
            Role::Assistant,
            3_000,
            vec![
                text("the needle is in the lexer"),
                tool(
                    "Grep",
                    "needle across crates",
                    json!({"pattern": "hay"}),
                    None,
                ),
                tool(
                    "Read",
                    "src/main.rs",
                    json!({"file_path": "needle.rs"}),
                    None,
                ),
                tool(
                    "Bash",
                    "cargo test",
                    json!({"command": "cargo test"}),
                    Some("found a needle in output"),
                ),
                Block::Notice {
                    kind: NoticeKind::Error,
                    text: "needle notice".into(),
                },
            ],
        )],
    )
}

fn search_for(view: &mut ConversationView, query: &str) {
    assert!(press(view, KeyCode::Char('/')).is_empty());
    assert_eq!(
        view.search(),
        Some(&Search {
            query: String::new(),
            typing: true,
            hits: vec![],
            hit: 0
        })
    );
    type_str(view, query);
    assert!(press(view, KeyCode::Enter).is_empty());
}

#[path = "conversation_tests/trail.rs"]
mod trail;

#[path = "conversation_tests/search.rs"]
mod search;

#[path = "conversation_tests/deltas.rs"]
mod deltas;

#[test]
fn a_delta_before_the_first_snapshot_is_dropped_quietly() {
    let mut view = ConversationView::default();
    view.open(WINDOW);
    // The open's own subscribe is outstanding; asking again would only race it.
    assert!(delta(&mut view, 0, vec![TurnPatch::Upsert(user_turn())]).is_empty());
    assert!(view.rows().is_empty());
    assert!(
        view.on_snapshot(WINDOW, None, main_conversation())
            .is_empty()
    );
    assert_eq!(view.rev(), Some(5));
}
