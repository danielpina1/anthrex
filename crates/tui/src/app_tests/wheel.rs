//! Milestone 9.11 decision 26 (milestone 7's wheel and encoding items): the wheel over
//! the focused terminal follows the app. A mouse mode gets the report in the encoding
//! the app asked for (SGR, X10 bytes or UTF-8); an alternate screen without a mouse
//! mode gets three cursor keys, as xterm's `alternateScroll` sends; the primary screen
//! with neither scrolls the client's own scrollback.

use super::*;

/// The app over a `width`-column main pane whose inner area starts at the screen's
/// top-left corner, so a click's column and row are the report's `x - 1` and `y - 1`.
fn wheel_app(width: u16) -> (App, crate::ui::Layout) {
    let app = app_with(vec![win(1, "a", Status::Idle)]);
    let area = ratatui::layout::Rect::new(0, 0, width, 24);
    let mut layout = crate::ui::layout(area, 0, crate::app::alerts(&app).len());
    layout.main_inner = area;
    (app, layout)
}

fn input(bytes: &[u8]) -> Vec<Effect> {
    vec![Effect::Send(ClientMsg::Input {
        window_id: 1,
        bytes: bytes.to_vec(),
    })]
}

#[test]
fn the_wheel_in_sgr_mode_sends_sgr() {
    let (mut app, layout) = wheel_app(80);
    app.parser.process(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(app.on_scroll(true, 2, 1, &layout), input(b"\x1b[<64;3;2M"));
    assert_eq!(app.on_scroll(false, 2, 1, &layout), input(b"\x1b[<65;3;2M"));
}

#[test]
fn the_wheel_in_default_mode_sends_x10_bytes() {
    let (mut app, layout) = wheel_app(300);
    app.parser.process(b"\x1b[?1000h");
    // Wheel up at column 3, row 2 of the main pane: button 64, each value plus 32.
    assert_eq!(
        app.on_scroll(true, 2, 1, &layout),
        input(&[0x1b, b'[', b'M', 96, 32 + 3, 32 + 2])
    );
    assert_eq!(
        app.on_scroll(false, 2, 1, &layout),
        input(&[0x1b, b'[', b'M', 97, 32 + 3, 32 + 2])
    );
    // The largest coordinate X10 can carry, 223, is the byte 255.
    assert_eq!(
        app.on_scroll(true, 222, 1, &layout),
        input(&[0x1b, b'[', b'M', 96, 255, 32 + 2])
    );
}

#[test]
fn the_wheel_in_utf8_mode_sends_utf8() {
    let (mut app, layout) = wheel_app(300);
    app.parser.process(b"\x1b[?1000h\x1b[?1005h");
    // Small values are one byte, as in X10.
    assert_eq!(
        app.on_scroll(true, 2, 1, &layout),
        input(&[0x1b, b'[', b'M', 96, 32 + 3, 32 + 2])
    );
    // Column 250 is the code point 32 + 250 = 282, two bytes in UTF-8.
    let mut expected = b"\x1b[M".to_vec();
    expected.push(96);
    expected.extend("\u{11A}".as_bytes());
    expected.push(32 + 2);
    assert_eq!(app.on_scroll(true, 249, 1, &layout), input(&expected));
}

#[test]
fn a_coordinate_past_x10_sends_nothing() {
    let (mut app, layout) = wheel_app(300);
    app.parser.process(b"\x1b[?1000h");
    // Column 224: 32 + 224 does not fit a byte.
    assert!(app.on_scroll(true, 223, 1, &layout).is_empty());
    assert_eq!(app.scroll_offset, 0, "nor does it scroll the scrollback");
}

#[test]
fn the_wheel_in_an_alternate_screen_without_mouse_sends_arrows() {
    let (mut app, layout) = wheel_app(80);
    app.parser.process(b"\x1b[?1049h");
    assert_eq!(
        app.on_scroll(true, 2, 1, &layout),
        input(b"\x1b[A\x1b[A\x1b[A")
    );
    assert_eq!(
        app.on_scroll(false, 2, 1, &layout),
        input(b"\x1b[B\x1b[B\x1b[B")
    );
    // Application-cursor mode: the arrows are SS3 sequences.
    app.parser.process(b"\x1b[?1h");
    assert_eq!(
        app.on_scroll(true, 2, 1, &layout),
        input(b"\x1bOA\x1bOA\x1bOA")
    );
    assert_eq!(
        app.on_scroll(false, 2, 1, &layout),
        input(b"\x1bOB\x1bOB\x1bOB")
    );
}

#[test]
fn the_wheel_on_the_primary_screen_scrolls_back() {
    let (mut app, layout) = wheel_app(80);
    for i in 0..40 {
        app.parser.process(format!("line {i}\r\n").as_bytes());
    }
    assert!(app.on_scroll(true, 2, 1, &layout).is_empty());
    assert_eq!(app.scroll_offset, 3);
    assert!(app.on_scroll(false, 2, 1, &layout).is_empty());
    assert_eq!(app.scroll_offset, 0);
}
