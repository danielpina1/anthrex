//! Milestone 9 (task M9.15; M9.6 review M-6): text an agent or the daemon wrote, made
//! safe to draw. Task notes, message lines, the orchestrator's summary, planner and epic
//! names, hold ids and task titles reach the terminal through this, so none of them can
//! carry an escape sequence, move the cursor, break a row, or reorder what a row shows.
//! Pure.
//!
//! Milestone 9.0.5 decision 27 moved the functions to `proto::safe_text`, so the daemon
//! sanitises what it stores with the same rules; they are re-exported here, and the
//! hostile-text helpers the client's render tests plant stay here.

pub use proto::safe_text::{is_hidden_format, multi_line, one_line};

#[cfg(test)]
pub(crate) mod tests {
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
}
