//! Milestone 9.18 decision 10: which client `Input` is only mouse reports.

use super::only_mouse_reports;

#[test]
fn sgr_wheel_reports_are_mouse_only() {
    assert!(only_mouse_reports(b"\x1b[<64;10;5M"));
    assert!(only_mouse_reports(&b"\x1b[<64;10;5M".repeat(3)));
    assert!(only_mouse_reports(b"\x1b[<65;1;1m"));
}

#[test]
fn default_and_urxvt_reports_are_mouse_only() {
    // The default (X10) encoding: wheel up (64 + 32) at 1,1 (1 + 32).
    assert!(only_mouse_reports(&[0x1b, b'[', b'M', 96, 33, 33]));
    // Its UTF-8 form, as the client's `wheel_report` writes it, at a column past 95.
    let mut utf8 = String::from("\x1b[M");
    for v in [64u32, 200, 5] {
        utf8.push(char::from_u32(32 + v).unwrap());
    }
    assert!(utf8.len() > 6, "a two-byte character is in the report");
    assert!(only_mouse_reports(utf8.as_bytes()));
    assert!(only_mouse_reports(b"\x1b[64;10;5M"));
}

#[test]
fn anything_else_is_typing() {
    assert!(!only_mouse_reports(b"a"));
    assert!(!only_mouse_reports(b"\r"));
    assert!(!only_mouse_reports(b"\x1b[A"));
    assert!(!only_mouse_reports(b"\x1b[<64;10;5Ma"));
    assert!(!only_mouse_reports(b"\x1b[<64;10"));
    assert!(!only_mouse_reports(b""));
}

/// The task's own cases (M9.18.8): the edges each form's grammar draws.
#[test]
fn the_edges_of_each_form_are_typing() {
    // A report then typing, typing then a report: the input holds typing.
    assert!(!only_mouse_reports(b"a\x1b[<64;10;5M"));
    assert!(!only_mouse_reports(b"\x1b[<64;10;5M\x1b[A"));
    // Each half of a report split across two `Input`s is typing on its own.
    assert!(!only_mouse_reports(b"\x1b[<64;10;"));
    assert!(!only_mouse_reports(b"5M"));
    assert!(!only_mouse_reports(&[0x1b, b'[', b'M', 96, 33]));
    // An empty parameter, a fourth one, or another final byte is not a report.
    assert!(!only_mouse_reports(b"\x1b[<;10;5M"));
    assert!(!only_mouse_reports(b"\x1b[<64;10;5;1M"));
    assert!(!only_mouse_reports(b"\x1b[<64;10;5~"));
    // URXVT has no release form, and the modified cursor keys have two parameters.
    assert!(!only_mouse_reports(b"\x1b[64;10;5m"));
    assert!(!only_mouse_reports(b"\x1b[1;5A"));
    assert!(!only_mouse_reports(b"\x1b[1;5M"));
    // The default encoding's three bytes are never controls.
    assert!(!only_mouse_reports(&[0x1b, b'[', b'M', 96, 33, b'\r']));
}

/// A default-encoding coordinate past 95 is one byte of 0x80 or more, which is not
/// UTF-8 on its own; two of them may happen to read as one UTF-8 character. Either
/// reading is accepted.
#[test]
fn default_reports_with_high_bytes_are_mouse_only() {
    assert!(only_mouse_reports(&[0x1b, b'[', b'M', 96, 0xff, 0x80]));
    assert!(only_mouse_reports(&[0x1b, b'[', b'M', 96, 0xc3, 0xa0]));
    let mut two = vec![0x1b, b'[', b'M', 96, 0xc3, 0xa0];
    two.extend_from_slice(b"\x1b[<65;1;1M");
    assert!(only_mouse_reports(&two));
}

/// Mixed encodings back to back, as one coalesced read could carry them.
#[test]
fn mixed_reports_back_to_back_are_mouse_only() {
    let mut bytes = b"\x1b[<64;10;5M".to_vec();
    bytes.extend_from_slice(&[0x1b, b'[', b'M', 97, 40, 40]);
    bytes.extend_from_slice(b"\x1b[65;3;4M");
    assert!(only_mouse_reports(&bytes));
}
