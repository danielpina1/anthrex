//! Text an agent or the daemon wrote, made safe to draw (milestone 9's M-6, moved here
//! from `tui/src/safe_text.rs` by milestone 9.0.5 decision 27 so the daemon can sanitise
//! what it stores with the same rules the client draws with). Task notes, message lines,
//! the orchestrator's summary, briefs, a worker's activity and summary reach the terminal
//! through this, so none of them can carry an escape sequence, move the cursor, break a
//! row, or reorder what a row shows. Pure, no dependency.

/// The invisible format characters a row must not carry: the bidi controls — U+200E
/// and U+200F (the marks), U+202A–U+202E (embeddings and overrides), U+2066–U+2069
/// (isolates), U+061C (the Arabic letter mark) — and U+200B–U+200D (zero-width space,
/// non-joiner, joiner) and U+FEFF (the byte-order mark). `char::is_control` (category
/// `Cc`) lets them all through, because they are category `Cf`.
///
/// Milestone 9.2 (M9.2.6 fix round 1, ruling I1) adds every other invisible carrier a
/// text can hide data or a second meaning in: the tag block U+E0000–U+E007F, the
/// variation selectors U+FE00–U+FE0F and U+E0100–U+E01EF, U+2060–U+2064 (word joiner
/// and the invisible operators), U+206A–U+206F (the deprecated format characters),
/// U+00AD (soft hyphen), U+180E (Mongolian vowel separator), U+FFF9–U+FFFB (the
/// interlinear annotation characters) and the Hangul fillers U+115F, U+1160, U+3164
/// and U+FFA0; and (fix round 2) U+034F (combining grapheme joiner), U+17B4–U+17B5
/// (the Khmer inherent vowels), U+180B–U+180D and U+180F (the Mongolian free variation
/// selectors) and U+1D173–U+1D17A (the musical symbol format characters). U+2800, the
/// blank braille cell, is a visible character and kept. Dropping a variation selector
/// or a joiner can change how an emoji is drawn (`❤️` becomes `❤`), never what the
/// text says.
pub fn is_hidden_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E007F}'
            | '\u{E0100}'..='\u{E01EF}'
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

/// Keeps `\n` (a `\r\n` or lone `\r` becomes `\n`); every other control character and
/// U+2028/U+2029 becomes a space and every hidden format character is dropped, as in
/// `one_line`.
pub fn multi_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => out.push('\n'),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            c if is_hidden_format(c) => {}
            c if is_break_or_control(c) => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written out here, apart from the code under test, so a narrowed predicate goes
    /// red: every character that must become a space (all of C0 — ESC, BEL, CR, LF,
    /// TAB — DEL, all of C1 with U+0085 NEXT LINE, and the two separators)…
    /// (`tui/src/safe_text.rs` keeps its own copy for the client's render tests.)
    fn spaced() -> Vec<char> {
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
    const DROPPED: &[char] = &[
        '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}',
        '\u{2068}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}', '\u{200B}', '\u{200C}',
        '\u{200D}', '\u{FEFF}',
    ];

    /// Every hostile character, from the two literal lists.
    fn hostile() -> Vec<char> {
        let mut all = spaced();
        all.extend_from_slice(DROPPED);
        all
    }

    /// `a<c>b ` for every hostile `c`, joined: the text a test plants.
    fn hostile_text() -> String {
        hostile().iter().map(|c| format!("a{c}b ")).collect()
    }

    /// The first hostile character in `text`, by the literal lists, not by the
    /// predicate under test.
    fn first_hostile(text: &str) -> Option<char> {
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

    /// Milestone 9.2's M9.2.6 fix round 1 (ruling I1): the invisible carriers a text can
    /// hide data or a second meaning in, written out as ranges apart from the predicate
    /// under test: the tag block, both variation-selector blocks, the invisible
    /// operators and word joiner, the deprecated format characters, the soft hyphen, the
    /// Mongolian vowel separator, the interlinear annotation characters and the Hangul
    /// fillers.
    const INVISIBLE: &[(u32, u32)] = &[
        (0xE0000, 0xE007F),
        (0xFE00, 0xFE0F),
        (0xE0100, 0xE01EF),
        (0x2060, 0x2064),
        (0x206A, 0x206F),
        (0x00AD, 0x00AD),
        (0x180E, 0x180E),
        (0xFFF9, 0xFFFB),
        (0x115F, 0x1160),
        (0x3164, 0x3164),
        (0xFFA0, 0xFFA0),
        // Fix round 2: the combining grapheme joiner, the Khmer inherent vowels, the
        // Mongolian free variation selectors and U+180F, the musical symbol format
        // characters.
        (0x034F, 0x034F),
        (0x17B4, 0x17B5),
        (0x180B, 0x180D),
        (0x180F, 0x180F),
        (0x1D173, 0x1D17A),
    ];

    fn invisible() -> Vec<char> {
        INVISIBLE
            .iter()
            .flat_map(|(a, b)| *a..=*b)
            .filter_map(char::from_u32)
            .collect()
    }

    #[test]
    fn invisible_carriers_are_dropped() {
        let all = invisible();
        assert_eq!(
            all.len(),
            128 + 16 + 240 + 5 + 6 + 1 + 1 + 3 + 2 + 1 + 1 + 1 + 2 + 3 + 1 + 8
        );
        for c in &all {
            assert!(is_hidden_format(*c), "U+{:04X}", *c as u32);
            assert_eq!(one_line(&format!("a{c}b")), "ab", "U+{:04X}", *c as u32);
            assert_eq!(multi_line(&format!("a{c}b")), "ab", "U+{:04X}", *c as u32);
        }
        // Their neighbours are ordinary text, and so is U+2800, a visible braille cell.
        for c in [
            '\u{E0080}',
            '\u{E01F0}',
            '\u{2800}',
            '\u{034E}',
            '\u{1D172}',
            '\u{1D17B}',
            '\u{DFFFF}',
            '\u{FDFF}',
            '\u{FE10}',
            '\u{205F}',
            '\u{2065}',
            '\u{00AC}',
            '\u{FFFC}',
            '\u{3165}',
        ] {
            assert!(!is_hidden_format(c), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn ordinary_text_is_unchanged() {
        let text = "t1 noted a risk: 世界 — the café’s API · ✓ שלום";
        assert_eq!(one_line(text), text);
        assert_eq!(multi_line(text), text);
    }

    #[test]
    fn multi_line_keeps_breaks_and_drops_the_rest() {
        let cleaned = multi_line(&hostile_text());
        let left: Vec<char> = cleaned.chars().filter(|c| hostile().contains(c)).collect();
        assert_eq!(left, ['\n', '\n'], "only LF survives, and CR became LF");
        for c in spaced() {
            let expected = match c {
                '\n' | '\r' => "a\nb",
                _ => "a b",
            };
            assert_eq!(
                multi_line(&format!("a{c}b")),
                expected,
                "U+{:04X}",
                c as u32
            );
        }
        for c in DROPPED {
            assert_eq!(multi_line(&format!("a{c}b")), "ab", "U+{:04X}", *c as u32);
        }
        assert_eq!(
            multi_line("one\r\ntwo\rthree\nfour"),
            "one\ntwo\nthree\nfour"
        );
        assert_eq!(multi_line("a\r\n\r\nb"), "a\n\nb");
        assert_eq!(multi_line("\x1b[2Jtab\there"), " [2Jtab here");
        for line in cleaned.split('\n') {
            assert_eq!(first_hostile(line), None);
        }
    }
}
