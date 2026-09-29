//! Milestone 9 (task M9.15; M9.6 review M-6): text an agent or the daemon wrote, made
//! safe to draw. Task notes, message lines, the orchestrator's summary, planner and epic
//! names, hold ids and task titles reach the terminal through this, so none of them can
//! carry an escape sequence, move the cursor, break a row, or reorder what a row shows.
//! Pure.

/// The invisible format characters a row must not carry: the bidi controls — U+200E
/// and U+200F (the marks), U+202A–U+202E (embeddings and overrides), U+2066–U+2069
/// (isolates), U+061C (the Arabic letter mark) — and U+200B–U+200D (zero-width space,
/// non-joiner, joiner) and U+FEFF (the byte-order mark). `char::is_control` (category
/// `Cc`) lets them all through, because they are category `Cf`.
pub fn is_hidden_format(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// A line break a terminal or a text layout may honour: every `Cc` control character
/// (which holds U+0085 NEXT LINE), U+2028 LINE SEPARATOR and U+2029 PARAGRAPH
/// SEPARATOR.
fn is_break_or_control(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')
}

/// `text` on one line: every control character and line separator becomes a space, so
/// words stay apart, and every hidden format character is dropped.
pub fn one_line(text: &str) -> String {
    text.chars()
        .filter(|c| !is_hidden_format(*c))
        .map(|c| if is_break_or_control(c) { ' ' } else { c })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Written out here, apart from the code under test, so a narrowed predicate goes
    /// red: every character that must become a space (all of C0 — ESC, BEL, CR, LF,
    /// TAB — DEL, all of C1 with U+0085 NEXT LINE, and the two separators)…
    pub(crate) fn spaced() -> Vec<char> {
        let mut chars: Vec<char> = (0x00u32..=0x1f)
            .chain(0x7f..=0x9f)
            .filter_map(char::from_u32)
            .collect();
        chars.extend(['\u{2028}', '\u{2029}']);
        chars
    }

    /// …and every one that must be dropped: the bidi embeddings, overrides, isolates
    /// and marks, the Arabic letter mark, the zero-width space, non-joiner and joiner,
    /// and the byte-order mark.
    pub(crate) const DROPPED: &[char] = &[
        '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}',
        '\u{2068}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}', '\u{200B}', '\u{200C}',
        '\u{200D}', '\u{FEFF}',
    ];

    /// Every hostile character, from the two literal lists.
    pub(crate) fn hostile() -> Vec<char> {
        let mut all = spaced();
        all.extend_from_slice(DROPPED);
        all
    }

    /// `a<c>b ` for every hostile `c`, joined: the text a test plants.
    pub(crate) fn hostile_text() -> String {
        hostile().iter().map(|c| format!("a{c}b ")).collect()
    }

    /// The first hostile character in `text`, by the literal lists, not by the
    /// predicate under test.
    pub(crate) fn first_hostile(text: &str) -> Option<char> {
        let hostile = hostile();
        text.chars().find(|c| hostile.contains(c))
    }

    #[test]
    fn every_listed_character_is_spaced_or_dropped() {
        for c in spaced() {
            assert_eq!(one_line(&format!("a{c}b")), "a b", "U+{:04X}", c as u32);
        }
        for c in DROPPED {
            assert_eq!(one_line(&format!("a{c}b")), "ab", "U+{:04X}", *c as u32);
        }
        assert_eq!(first_hostile(&one_line(&hostile_text())), None);
        assert_eq!(one_line("tab\there\nnext"), "tab here next");
    }

    #[test]
    fn ordinary_text_is_unchanged() {
        let text = "t1 noted a risk: 世界 — the café’s API · ✓ שלום";
        assert_eq!(one_line(text), text);
    }
}
