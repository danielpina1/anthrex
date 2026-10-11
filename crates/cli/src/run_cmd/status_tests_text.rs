//! Milestone 9.11 decision 33 (D7): a model-written reason on a status line goes through
//! `proto::safe_text::one_line`, so a hidden format character (a bidi override) cannot
//! reorder what the user reads.

use super::one_line;

#[test]
fn one_line_drops_bidi_controls() {
    assert_eq!(one_line("a\u{202E}b"), "ab");
    assert_eq!(one_line("a\u{200B}b\u{2066}c"), "abc");
    // Control characters still become a space.
    assert_eq!(one_line("a\nb\x1bc"), "a b c");
}
