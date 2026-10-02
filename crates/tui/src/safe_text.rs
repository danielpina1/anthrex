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

    /// …and every one that must be dropped: every range of
    /// `proto::safe_text::is_hidden_format`, written out (milestone 9.2's M9.2.6 fix
    /// rounds 1 and 2 widened it; task M9.2.15 widened this list to match): the soft
    /// hyphen, the combining grapheme joiner, the Arabic letter mark, the Hangul and
    /// halfwidth fillers, the Khmer inherent vowels, the Mongolian free variation
    /// selectors and vowel separator, the zero-width space, non-joiner and joiner, the
    /// left-to-right and right-to-left marks, the bidi embeddings, overrides and
    /// isolates, the word joiner and the invisible operators, the deprecated format
    /// characters, both variation-selector blocks, the byte-order mark, the
    /// interlinear annotation characters, the musical symbol format characters and the
    /// tag block. Inclusive ranges.
    pub(crate) const DROPPED_RANGES: &[(char, char)] = &[
        ('\u{00AD}', '\u{00AD}'),
        ('\u{034F}', '\u{034F}'),
        ('\u{061C}', '\u{061C}'),
        ('\u{115F}', '\u{1160}'),
        ('\u{17B4}', '\u{17B5}'),
        ('\u{180B}', '\u{180F}'),
        ('\u{200B}', '\u{200F}'),
        ('\u{202A}', '\u{202E}'),
        ('\u{2060}', '\u{2064}'),
        ('\u{2066}', '\u{206F}'),
        ('\u{3164}', '\u{3164}'),
        ('\u{FE00}', '\u{FE0F}'),
        ('\u{FEFF}', '\u{FEFF}'),
        ('\u{FFA0}', '\u{FFA0}'),
        ('\u{FFF9}', '\u{FFFB}'),
        ('\u{1D173}', '\u{1D17A}'),
        ('\u{E0000}', '\u{E007F}'),
        ('\u{E0100}', '\u{E01EF}'),
    ];

    /// Every character of [`DROPPED_RANGES`].
    pub(crate) fn dropped() -> Vec<char> {
        DROPPED_RANGES.iter().flat_map(|&(a, b)| a..=b).collect()
    }

    /// Every hostile character, from the two literal lists.
    pub(crate) fn hostile() -> Vec<char> {
        let mut all = spaced();
        all.extend(dropped());
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

    /// The list and the predicate agree on every scalar value: a range the daemon's
    /// predicate gains goes red here until the list names it, and one it loses too.
    #[test]
    fn the_dropped_list_is_every_hidden_format_range() {
        let listed = dropped();
        assert_eq!(
            listed.len(),
            1 + 1 + 1 + 2 + 2 + 5 + 5 + 5 + 5 + 10 + 1 + 16 + 1 + 1 + 3 + 8 + 128 + 240
        );
        let hidden: Vec<char> = (0..=0x10FFFFu32)
            .filter_map(char::from_u32)
            .filter(|c| super::is_hidden_format(*c))
            .collect();
        assert_eq!(hidden, listed);
        for c in listed {
            assert_eq!(
                super::one_line(&format!("a{c}b")),
                "ab",
                "U+{:04X}",
                c as u32
            );
            assert_eq!(
                super::multi_line(&format!("a{c}b")),
                "ab",
                "U+{:04X}",
                c as u32
            );
        }
        assert_eq!(first_hostile(&super::one_line(&hostile_text())), None);
    }
}
