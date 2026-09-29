//! Milestone 9 (task M9.15; M9.6 review M-6): text an agent or the daemon wrote, made
//! safe to draw. Task notes, message lines, the orchestrator's summary, planner and epic
//! names, hold ids and task titles reach the terminal through this, so none of them can
//! carry an escape sequence, move the cursor, break a row, or reorder what a row shows.
//! Pure.

/// The bidi controls: U+200E and U+200F (the marks), U+202A–U+202E (embeddings and
/// overrides) and U+2066–U+2069 (isolates). `char::is_control` (category `Cc`) lets
/// them through, because they are category `Cf`.
pub fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// A line break a terminal or a text layout may honour: every `Cc` control character
/// (which holds U+0085 NEXT LINE), U+2028 LINE SEPARATOR and U+2029 PARAGRAPH
/// SEPARATOR.
fn is_break_or_control(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')
}

/// `text` on one line: every control character and line separator becomes a space, so
/// words stay apart, and every bidi control is dropped.
pub fn one_line(text: &str) -> String {
    text.chars()
        .filter(|c| !is_bidi_control(*c))
        .map(|c| if is_break_or_control(c) { ' ' } else { c })
        .collect()
}

/// Whether `text` holds nothing [`one_line`] would change.
pub fn is_safe(text: &str) -> bool {
    !text
        .chars()
        .any(|c| is_bidi_control(c) || is_break_or_control(c))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Every character M-6 names: an escape, a bell, a carriage return, a newline, a
    /// tab, NEXT LINE, the two separators and each bidi control.
    pub(crate) const HOSTILE: &[char] = &[
        '\u{1b}', '\u{7}', '\r', '\n', '\t', '\u{85}', '\u{2028}', '\u{2029}', '\u{200E}',
        '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}',
        '\u{2067}', '\u{2068}', '\u{2069}',
    ];

    /// `a<c>b` for every hostile `c`, joined: the text a test plants.
    pub(crate) fn hostile_text() -> String {
        HOSTILE.iter().map(|c| format!("a{c}b ")).collect()
    }

    #[test]
    fn one_line_leaves_no_control_separator_or_bidi_character() {
        let clean = one_line(&hostile_text());
        assert!(is_safe(&clean), "{clean:?}");
        assert!(
            clean.contains("a b"),
            "a control becomes a space: {clean:?}"
        );
        assert!(clean.contains("ab"), "a bidi control is dropped: {clean:?}");
    }

    #[test]
    fn ordinary_text_is_unchanged() {
        let text = "t1 noted a risk: 世界 — the café’s API · ✓";
        assert_eq!(one_line(text), text);
        assert!(is_safe(text));
        // U+200D ZERO WIDTH JOINER fuses an emoji; it is neither control nor bidi.
        assert!(is_safe("👩\u{200D}💻"));
    }
}
