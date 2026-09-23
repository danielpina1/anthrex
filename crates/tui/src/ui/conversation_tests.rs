//! Task M6.5.13's render tests. Every test draws into a `TestBackend` through
//! `conversation::render(frame, &app, area)` and reads the buffer — cell text and cell
//! styles — never a private helper. The app reaches the view the way a user's does:
//! `C-b m`, then the daemon's `ConversationSnapshot` through `App::on_daemon`.
//!
//! Fixtures use distinct values throughout: every tool has its own name, summary, input
//! and result; the Edit's old and new strings differ in length and content; the two drop
//! causes have different counts; the breadcrumb's three names differ.

use super::*;
use crate::conversation::{Cursor, DetailKind, Row};
use crate::settings::UiSettings;
use crate::theme;
use crate::ui::badge::BadgeSet;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{
    Block, Conversation, DaemonMsg, DegradeReason, DropCause, NoticeKind, Role, Runtime,
    ToolResult, ToolState, Turn, TurnState, WindowInfo,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use serde_json::json;
use unicode_width::UnicodeWidthStr;

const WINDOW: u32 = 7;
const OLD: &str = "fn parse_all(src: &str) {";
const NEW: &str = "fn parse_all(src: &str) -> Result<Ast> {";
/// Every glyph ASCII mode must replace.
const UNICODE_GLYPHS: [&str; 9] = ["▸", "▾", "⟐", "⚠", "⋯", "›", "✓", "✕", "⊘"];

fn window(runtime: Runtime) -> WindowInfo {
    WindowInfo {
        id: WINDOW,
        name: "orchestrator".into(),
        runtime,
        cwd: "/tmp/repo".into(),
        project: "/tmp/repo".into(),
        worktree: None,
        branch: None,
        status: proto::Status::Working,
        tool: None,
        since_secs: 0,
        last_output_secs: 0,
        session_id: None,
        model: Some("claude-opus-5".into()),
        subagents: vec![],
        exit: None,
    }
}

fn ascii_settings() -> UiSettings {
    UiSettings {
        badges: BadgeSet::from_config(&config::Badges::default(), true),
        ..UiSettings::default()
    }
}

fn key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Vec<crate::app::Effect> {
    app.on_key(KeyEvent::new(code, mods))
}

fn press(app: &mut App, code: KeyCode) {
    let effects = key(app, code, KeyModifiers::NONE);
    assert!(
        effects.iter().all(|e| matches!(
            e,
            crate::app::Effect::Send(proto::ClientMsg::SubscribeConversation { .. })
                | crate::app::Effect::Send(proto::ClientMsg::UnsubscribeConversation { .. })
        )),
        "the view is read-only: {effects:?}"
    );
}

/// An app sized for a `width`×`height` conversation area (the interior is two less each
/// way, exactly as `lib.rs` reports `main_inner` for `main`), with the view opened by
/// `C-b m` on the one window and `conversation` delivered as its snapshot.
fn app_showing(
    settings: UiSettings,
    runtime: Runtime,
    width: u16,
    height: u16,
    conversation: Conversation,
) -> App {
    let mut app = App::new(vec![window(runtime)], "/tmp".into(), settings);
    let _ = app.set_terminal_size(width - 2, height - 2);
    assert_eq!(app.focused, Some(WINDOW));
    key(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('m'), KeyModifiers::NONE);
    assert!(app.conversation.is_open());
    app.on_daemon(DaemonMsg::ConversationSnapshot {
        window_id: WINDOW,
        agent_id: conversation.agent_id.clone(),
        conversation,
    });
    app
}

fn draw(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render(frame, app, frame.area()))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

fn all_rows(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height).map(|y| row_text(buf, y)).collect()
}

fn text_of(buf: &Buffer) -> String {
    all_rows(buf).join("\n")
}

/// The first `(x, y)` where `needle` starts, reading cell by cell. Needles are ASCII or
/// one-column glyphs, so one cell is one char.
fn find(buf: &Buffer, needle: &str) -> Option<(u16, u16)> {
    for y in 0..buf.area.height {
        let cells: Vec<&str> = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
        let row: String = cells.concat();
        if let Some(byte) = row.find(needle) {
            let mut seen = 0;
            for (x, cell) in cells.iter().enumerate() {
                if seen == byte {
                    return Some((x as u16, y));
                }
                seen += cell.len();
            }
        }
    }
    None
}

fn text(text: &str) -> Block {
    Block::Text { text: text.into() }
}

fn tool(
    name: &str,
    summary: &str,
    input: serde_json::Value,
    state: ToolState,
    ms: Option<u32>,
    result: Option<ToolResult>,
) -> Block {
    Block::ToolCall {
        id: Some(format!("toolu_{}", name.to_lowercase())),
        name: name.into(),
        summary: summary.into(),
        input: Some(input),
        result,
        state,
        duration_ms: ms,
    }
}

fn result(summary: &str, detail: Option<&str>, truncated: bool) -> Option<ToolResult> {
    Some(ToolResult {
        ok: true,
        summary: summary.into(),
        detail: detail.map(str::to_owned),
        truncated,
    })
}

fn spawn(agent_id: &str, label: &str, model: &str) -> Block {
    Block::SubagentSpawn {
        agent_id: agent_id.into(),
        kind: "Explore".into(),
        label: label.into(),
        model: Some(model.into()),
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
        session_id: Some("sess-r".into()),
        runtime: Runtime::Claude,
        rev,
        degraded: None,
        dropped_turns: 0,
        dropped_by: None,
        turns,
    }
}

/// Turn 22's blocks: 0 prose, 1 Grep, 2 Edit, 3 Bash (truncated result), 4 spawn.
fn main_conversation() -> Conversation {
    conversation(
        None,
        214,
        vec![
            turn(
                21,
                Role::User,
                34_920,
                vec![text("refactor the parser into its own module")],
            ),
            turn(
                22,
                Role::Assistant,
                34_980,
                vec![
                    text("I'll map the call sites first."),
                    tool(
                        "Grep",
                        "\"parse_\" → 34 matches",
                        json!({"pattern": "parse_needle", "path": "crates/parse"}),
                        ToolState::Ok,
                        Some(300),
                        result("34 matches in 6 files", None, false),
                    ),
                    tool(
                        "Edit",
                        "crates/parse/src/lib.rs",
                        json!({"file_path": "crates/parse/src/lib.rs", "old_string": OLD, "new_string": NEW}),
                        ToolState::Ok,
                        Some(1_100),
                        result("edited the parser", None, false),
                    ),
                    tool(
                        "Bash",
                        "cargo test -p parse",
                        json!({"command": "cargo test -p parse_bash_cmd"}),
                        ToolState::Failed,
                        Some(2_450),
                        result(
                            "412 passed, 3 failed",
                            Some("failures: parse::nested"),
                            true,
                        ),
                    ),
                    spawn("agent-explore", "Explore", "haiku"),
                ],
            ),
        ],
    )
}

fn unfold(app: &mut App, turn: u64, block: usize) {
    app.conversation.set_cursor(Cursor::Block(turn, block));
    press(app, KeyCode::Enter);
    assert!(app.conversation.is_unfolded(turn, block));
}

#[test]
fn a_folded_tool_call_is_one_line_with_its_state_and_duration() {
    let app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        main_conversation(),
    );
    let buf = draw(&app, 80, 24);
    let rows = all_rows(&buf);
    let grep = rows
        .iter()
        .find(|r| r.contains("Grep"))
        .unwrap_or_else(|| panic!("no Grep row:\n{}", text_of(&buf)));
    assert!(grep.contains("▸ Grep  \"parse_\" → 34 matches"), "{grep}");
    assert!(grep.contains("✓ 0.3s"), "{grep}");
    let bash = rows.iter().find(|r| r.contains("Bash")).unwrap();
    assert!(bash.contains("✕ 2.5s"), "{bash}");
    let out = text_of(&buf);
    assert!(!out.contains("parse_needle"), "the input leaked:\n{out}");
    assert!(!out.contains("\"pattern\""), "the input leaked:\n{out}");
    assert!(
        !out.contains("34 matches in 6 files"),
        "the result leaked:\n{out}"
    );
}

#[test]
fn a_pending_call_spins_and_has_no_duration() {
    let mut conv = main_conversation();
    let Block::ToolCall {
        state, duration_ms, ..
    } = &mut conv.turns[1].blocks[1]
    else {
        unreachable!()
    };
    *state = ToolState::Pending;
    *duration_ms = None;
    let mut app = app_showing(UiSettings::default(), Runtime::Claude, 80, 24, conv);
    app.spinner_frame = 13;
    let buf = draw(&app, 80, 24);
    let grep = all_rows(&buf)
        .into_iter()
        .find(|r| r.contains("Grep"))
        .unwrap();
    assert!(grep.contains(theme::SPINNER[3]), "{grep}");
    assert!(!grep.contains("0.3s"), "{grep}");
    assert!(
        grep.trim_end_matches(['│', ' '])
            .ends_with(theme::SPINNER[3]),
        "{grep}"
    );
}

/// Descends from the root into `agent-explore` ("Explore"), then into `agent-review`
/// ("Review"), delivering each sub-agent's snapshot as the daemon would.
fn descend_twice(app: &mut App) {
    app.conversation.set_cursor(Cursor::Block(22, 4));
    press(app, KeyCode::Enter);
    app.on_daemon(DaemonMsg::ConversationSnapshot {
        window_id: WINDOW,
        agent_id: Some("agent-explore".into()),
        conversation: conversation(
            Some("agent-explore"),
            31,
            vec![turn(
                40,
                Role::Assistant,
                35_000,
                vec![spawn("agent-review", "Review", "sonnet")],
            )],
        ),
    });
    app.conversation.set_cursor(Cursor::Block(40, 0));
    press(app, KeyCode::Enter);
    app.on_daemon(DaemonMsg::ConversationSnapshot {
        window_id: WINDOW,
        agent_id: Some("agent-review".into()),
        conversation: conversation(
            Some("agent-review"),
            52,
            vec![turn(
                60,
                Role::Assistant,
                35_100,
                vec![text("reviewing the split")],
            )],
        ),
    });
    assert_eq!(app.conversation.trail().len(), 2);
}

#[test]
fn the_breadcrumb_shows_where_you_are() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        main_conversation(),
    );
    let title = row_text(&draw(&app, 80, 24), 0);
    assert!(title.contains("orchestrator · claude-opus-5"), "{title}");
    assert!(title.contains("rev 214"), "{title}");
    assert!(!title.contains('›'), "{title}");

    descend_twice(&mut app);
    let buf = draw(&app, 80, 24);
    let title = row_text(&buf, 0);
    assert!(title.contains("orchestrator › Explore › Review"), "{title}");
    assert!(title.contains("rev 52"), "{title}");
    assert!(text_of(&buf).contains("reviewing the split"));
}

#[test]
fn the_selected_row_is_reversed() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        main_conversation(),
    );
    app.conversation.set_cursor(Cursor::Block(22, 1));
    let buf = draw(&app, 80, 24);
    let (_, y) = find(&buf, "Grep").unwrap();
    for x in 1..79 {
        assert!(
            buf[(x, y)].modifier.contains(Modifier::REVERSED),
            "cell {x} of the selected row is not reversed"
        );
    }
    for other in 0..24 {
        for x in 0..80 {
            if other != y {
                assert!(
                    !buf[(x, other)].modifier.contains(Modifier::REVERSED),
                    "cell ({x}, {other}) is reversed:\n{}",
                    text_of(&buf)
                );
            }
        }
    }
}

#[test]
fn the_search_bar_shows_the_query_while_typing() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        main_conversation(),
    );
    let bottom = row_text(&draw(&app, 80, 24), 23);
    assert!(!bottom.contains('/'), "{bottom}");

    press(&mut app, KeyCode::Char('/'));
    for c in "parse".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    let buf = draw(&app, 80, 24);
    let bottom = row_text(&buf, 23);
    assert!(bottom.starts_with('╰') && bottom.ends_with('╯'), "{bottom}");
    let at = bottom.find("/parse").unwrap_or_else(|| panic!("{bottom}"));
    let col = bottom[..at].chars().count();
    assert!((30..=45).contains(&col), "not centred: {bottom}");

    press(&mut app, KeyCode::Enter);
    let bottom = row_text(&draw(&app, 80, 24), 23);
    assert!(!bottom.contains("/parse"), "{bottom}");
}

#[test]
fn a_narrow_terminal_still_renders() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        20,
        8,
        main_conversation(),
    );
    unfold(&mut app, 22, 2);
    descend_twice(&mut app);
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('x'));
    for (w, h) in [(20, 8), (3, 3), (12, 4)] {
        let buf = draw(&app, w, h);
        let top = row_text(&buf, 0);
        let bottom = row_text(&buf, h - 1);
        assert!(top.starts_with('╭') && top.ends_with('╮'), "{w}x{h}: {top}");
        assert!(
            bottom.starts_with('╰') && bottom.ends_with('╯'),
            "{w}x{h}: {bottom}"
        );
        for y in 1..h - 1 {
            assert_eq!(buf[(0, y)].symbol(), "│");
            assert_eq!(buf[(w - 1, y)].symbol(), "│");
        }
    }
}

#[test]
fn the_turn_header_shows_the_role_and_local_time() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        main_conversation(),
    );
    // 34_920 s is 09:42 UTC; 34_980 s is 09:43.
    let out = text_of(&draw(&app, 80, 24));
    let you = out.lines().find(|l| l.contains("you")).unwrap();
    assert!(you.trim_end_matches(['│', ' ']).ends_with("09:42"), "{you}");
    let assistant = out.lines().find(|l| l.contains("assistant")).unwrap();
    assert!(assistant.contains("◆"), "{assistant}");
    assert!(
        assistant.trim_end_matches(['│', ' ']).ends_with("09:43"),
        "{assistant}"
    );
    assert!(
        out.contains("▸ refactor the parser into its own module"),
        "{out}"
    );

    app.utc_offset_secs = -5 * 3600;
    let out = text_of(&draw(&app, 80, 24));
    let you = out.lines().find(|l| l.contains("you")).unwrap();
    assert!(you.trim_end_matches(['│', ' ']).ends_with("04:42"), "{you}");
}

#[path = "conversation_tests/glyphs.rs"]
mod glyphs;

#[path = "conversation_tests/tools.rs"]
mod tools;

#[path = "conversation_tests/wrap.rs"]
mod wrap;
