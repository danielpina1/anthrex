//! Task M6.5.13: the runtime badge (spec decision 10: colour is never the only signal)
//! and ASCII mode (decision A5), which swaps every one of the view's own glyphs.

use super::*;

/// The title's first cells after the corner and its rule, unicode or ASCII.
fn title_start(buf: &Buffer) -> String {
    row_text(buf, 0)
        .trim_start_matches(['╭', '─', '+', '-', ' '])
        .to_owned()
}

#[test]
fn the_badge_is_drawn_and_is_not_only_colour() {
    let app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        main_conversation(),
    );
    let buf = draw(&app, 80, 24);
    let start = title_start(&buf);
    assert!(start.starts_with('◆'), "{start}");
    let (x, _) = find(&buf, "◆").unwrap();
    assert_eq!(buf[(x, 0)].fg, UiSettings::default().badges.claude.color);
    // The spawn row carries the window's badge too: a sub-agent has no runtime of its own.
    let spawn = all_rows(&buf)
        .into_iter()
        .find(|r| r.contains("spawned"))
        .unwrap();
    assert!(spawn.contains("◆"), "{spawn}");
    assert!(spawn.contains("haiku"), "{spawn}");

    let app = app_showing(
        ascii_settings(),
        Runtime::Claude,
        80,
        24,
        main_conversation(),
    );
    let buf = draw(&app, 80, 24);
    assert!(
        title_start(&buf).starts_with("[C]"),
        "{}",
        title_start(&buf)
    );

    // A Codex window: its own badge, in the title and on the spawn row — even though
    // this conversation says `Claude`, because the badge is the window's (review N4).
    let codex = main_conversation();
    assert_eq!(codex.runtime, Runtime::Claude);
    let app = app_showing(UiSettings::default(), Runtime::Codex, 80, 24, codex);
    let buf = draw(&app, 80, 24);
    assert!(title_start(&buf).starts_with('◇'), "{}", title_start(&buf));
    let spawn = all_rows(&buf)
        .into_iter()
        .find(|r| r.contains("spawned"))
        .unwrap();
    assert!(spawn.contains("◇") && !spawn.contains("◆"), "{spawn}");
}

/// Every glyph ASCII mode replaces, on one screen: dropped, degraded, a notice, folded
/// and unfolded calls in each finished state, a truncated result, a spawn, a breadcrumb.
fn every_glyph(app: &mut App) {
    let mut sub = conversation(
        Some("agent-explore"),
        77,
        vec![turn(
            50,
            Role::Assistant,
            35_000,
            vec![
                tool(
                    "Read",
                    "read lib.rs",
                    json!({"file_path": "lib.rs"}),
                    ToolState::Ok,
                    Some(100),
                    result("read 40 lines", None, true),
                ),
                tool(
                    "Write",
                    "wrote out.rs",
                    json!({"file_path": "out.rs", "content": "x"}),
                    ToolState::Failed,
                    Some(200),
                    None,
                ),
                tool(
                    "Bash",
                    "rm -rf target",
                    json!({"command": "rm -rf target"}),
                    ToolState::Denied,
                    None,
                    None,
                ),
                Block::Notice {
                    kind: NoticeKind::PermissionRequest,
                    text: "needs approval for Bash".into(),
                },
                spawn("agent-deeper", "Deeper", "haiku"),
            ],
        )],
    );
    sub.dropped_turns = 3;
    sub.dropped_by = Some(DropCause::Turns);
    sub.degraded = Some(DegradeReason::Unreadable);
    app.conversation.set_cursor(Cursor::Block(22, 4));
    press(app, KeyCode::Enter);
    app.on_daemon(DaemonMsg::ConversationSnapshot {
        window_id: WINDOW,
        agent_id: Some("agent-explore".into()),
        conversation: sub,
    });
    unfold(app, 50, 0);
}

#[test]
fn ascii_mode_uses_no_box_drawing_glyphs() {
    let mut unicode = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        30,
        main_conversation(),
    );
    every_glyph(&mut unicode);
    let out = text_of(&draw(&unicode, 80, 30));
    for glyph in UNICODE_GLYPHS {
        assert!(
            out.contains(glyph),
            "the fixture never draws {glyph}:\n{out}"
        );
    }

    let mut ascii = app_showing(
        ascii_settings(),
        Runtime::Claude,
        80,
        30,
        main_conversation(),
    );
    every_glyph(&mut ascii);
    let out = text_of(&draw(&ascii, 80, 30));
    for glyph in UNICODE_GLYPHS {
        assert!(!out.contains(glyph), "ASCII mode drew {glyph}:\n{out}");
    }
    assert!(out.contains("orchestrator > Explore"), "{out}");
    assert!(
        out.contains("... 3 earlier turns dropped (conversation.max_turns)"),
        "{out}"
    );
    assert!(
        out.contains("! transcript unreadable - timeline only"),
        "{out}"
    );
    assert!(out.contains("v Read  read lib.rs"), "{out}");
    assert!(out.contains("ok 0.1s"), "{out}");
    assert!(out.contains("> Write  wrote out.rs"), "{out}");
    assert!(out.contains("x 0.2s"), "{out}");
    let bash = out.lines().find(|l| l.contains("rm -rf target")).unwrap();
    assert!(bash.trim_end_matches(['|', ' ']).ends_with(" -"), "{bash}");
    assert!(out.contains("! needs approval for Bash"), "{out}");
    assert!(out.contains("* spawned  Explore - Deeper"), "{out}");
    assert!(
        out.contains("... truncated (conversation.max_result_bytes)"),
        "{out}"
    );
}

/// Review N2: the view's border is `theme::border_focused(accent)`, with the configured
/// accent, not the dimmed unfocused border.
#[test]
fn the_border_uses_the_focused_accent() {
    let accent = Color::Rgb(0x12, 0x34, 0x56);
    let settings = UiSettings {
        accent,
        ..UiSettings::default()
    };
    let app = app_showing(settings, Runtime::Claude, 60, 12, main_conversation());
    let buf = draw(&app, 60, 12);
    let expected = theme::border_focused(accent).fg;
    assert_eq!(expected, Some(accent));
    for (x, y) in [(0, 0), (0, 5), (59, 5), (30, 11), (59, 11)] {
        assert_eq!(Some(buf[(x, y)].fg), expected, "cell ({x}, {y})");
    }
}

/// `main_conversation` with ASCII agent text, and summaries carrying the daemon's own
/// punctuation (`summary.rs`: `—`, `·` and `…`) — so that everything left non-ASCII on
/// screen could only be the view's own drawing.
fn ascii_agent_text() -> Conversation {
    let mut conv = main_conversation();
    let Block::ToolCall { summary, .. } = &mut conv.turns[1].blocks[1] else {
        unreachable!()
    };
    *summary = "crates/parse/src/lib.rs — 1 hunk".into();
    let Block::ToolCall { summary, .. } = &mut conv.turns[1].blocks[3] else {
        unreachable!()
    };
    *summary = "run the tests · cargo test -p pa…".into();
    conv
}

fn non_ascii(out: &str) -> Vec<char> {
    out.chars().filter(|c| !c.is_ascii()).collect()
}

/// Review M4 (decision A5): in ASCII mode the view draws nothing but ASCII — its border,
/// its separators and footers, and the daemon's summary punctuation included. The fixture
/// has no Pending call: the spinner is the one exception, left to a follow-up.
#[test]
fn ascii_mode_draws_only_ascii() {
    let mut app = app_showing(
        ascii_settings(),
        Runtime::Claude,
        80,
        30,
        ascii_agent_text(),
    );
    let root = text_of(&draw(&app, 80, 30));
    assert_eq!(non_ascii(&root), Vec::<char>::new(), "{root}");
    assert!(root.starts_with('+'), "{root}");
    assert!(root.contains("orchestrator - claude-opus-5"), "{root}");
    assert!(root.contains("crates/parse/src/lib.rs - 1 hunk"), "{root}");
    assert!(
        root.contains("run the tests - cargo test -p pa..."),
        "{root}"
    );
    assert!(root.contains("* spawned  Explore - Explore"), "{root}");

    every_glyph(&mut app);
    let sub = text_of(&draw(&app, 80, 30));
    assert_eq!(non_ascii(&sub), Vec::<char>::new(), "{sub}");
    assert!(
        sub.contains("! transcript unreadable - timeline only"),
        "{sub}"
    );
    assert!(
        sub.contains("! sub-agent transcript not read - timeline only"),
        "{sub}"
    );
    let bottom = sub.lines().last().unwrap();
    assert!(bottom.starts_with('+') && bottom.ends_with('+'), "{bottom}");
    assert!(bottom.trim_matches(['+', '-']).is_empty(), "{bottom}");
}
