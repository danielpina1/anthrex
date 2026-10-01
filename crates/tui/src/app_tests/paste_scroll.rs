//! The focused window's paste (bracketed or not, markers stripped) and the local
//! scrollback under the mouse wheel. Moved unchanged from `app/tests.rs` (rule 8).

use super::*;

#[test]
fn paste_uses_bracketed_mode_when_the_program_asked_for_it() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    assert_eq!(
        app.on_paste("ab\ncd".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"ab\rcd".to_vec()
        })]
    );
    app.parser.process(b"\x1b[?2004h");
    assert_eq!(
        app.on_paste("x".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"\x1b[200~x\x1b[201~".to_vec()
        })]
    );
}

#[test]
fn paste_strips_bracketed_paste_markers() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    assert_eq!(
        app.on_paste("a\x1b[201~b\x1b[200~c".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"abc".to_vec()
        })]
    );
    assert_eq!(
        app.on_paste("a\x1b[20\x1b[201~1~b".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"ab".to_vec()
        })],
        "removing an embedded marker must not manufacture a new end marker"
    );

    app.parser.process(b"\x1b[?2004h");
    assert_eq!(
        app.on_paste("a\x1b[201~b\x1b[200~c".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"\x1b[200~abc\x1b[201~".to_vec()
        })]
    );
    assert_eq!(
        app.on_paste("a\x1b[20\x1b[201~1~b".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"\x1b[200~ab\x1b[201~".to_vec()
        })]
    );
}

#[test]
fn wheel_scrolls_the_local_scrollback_and_any_key_snaps_back() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    for i in 0..40 {
        app.parser.process(format!("line {i}\r\n").as_bytes());
    }
    let area = ratatui::layout::Rect::new(0, 0, 80, 24);
    let mut layout = crate::ui::layout(area, 0, crate::app::alerts(&app).len());
    layout.main_inner = area;
    assert!(app.on_scroll(true, 5, 5, &layout).is_empty());
    assert_eq!(app.scroll_offset, 3);
    press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(app.scroll_offset, 0);
    app.parser.process(b"\x1b[?1000h\x1b[?1006h");
    let effects = app.on_scroll(false, 5, 5, &layout);
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"\x1b[<65;6;6M".to_vec()
        })]
    );
}
