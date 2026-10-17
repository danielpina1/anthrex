//! A run of a secret's alphabet read through escape sequences and hidden format
//! characters, and its readings: what the rules that judge such a run share (task 6's
//! review, I1 and M1; task 9's review, A-I1; fix round 2's I2). The AWS rules, `Basic`
//! and `Bearer` each judge the same parts: the visible pieces, and each escape's bytes
//! that the strip eats with it. Which of those bytes were the secret's is not known,
//! only that they are the escape's bytes of the secret's alphabet. Pure and linear.

use proto::safe_text::is_hidden_format;

/// Counts `n` bytes a check read (the linearity tests' measure, ruling C6).
pub(super) fn read(n: usize) {
    #[cfg(test)]
    super::BODY_READ.with(|c| c.set(c.get() + n as u64));
    #[cfg(not(test))]
    let _ = n;
}

/// The length of the escape sequence or hidden format character at `at`, if one is
/// there: what `scrub`'s strip and the line breakers remove.
pub(super) fn invisible_at(line: &str, at: usize) -> Option<usize> {
    let c = line[at..].chars().next()?;
    if c == '\u{1b}' || c == '\u{9b}' {
        Some(
            super::ESCAPE_HERE
                .find(&line[at..])
                .map_or(c.len_utf8(), |m| m.end().max(1)),
        )
    } else if is_hidden_format(c) {
        Some(c.len_utf8())
    } else {
        None
    }
}

/// The bytes after its lead of the escape `line[at..at + len]` that can eat a secret's
/// characters: task 6's review, I1, and task 9's review, A-I1. ECMA-48's general form
/// and a CSI take the next printable characters as their parameters, intermediates and
/// final, so the strip eats a secret's characters with the escape. An OSC, DCS, SOS, PM
/// or APC string has no such bytes, and a hidden format character is no escape.
fn eaten(line: &str, at: usize, len: usize) -> Option<&[u8]> {
    let escape = &line.as_bytes()[at..at + len];
    let lead = if escape.first() == Some(&0x1b) {
        1
    } else if line[at..].starts_with('\u{9b}') {
        2
    } else {
        return None;
    };
    if lead == 1 && matches!(escape.get(1), Some(b']' | b'P' | b'X' | b'^' | b'_')) {
        return None;
    }
    (escape.len() > lead).then(|| &escape[lead..])
}

/// The final byte of the escape `line[at..at + len]` ([`eaten`]) when `alphabet` holds
/// it: task 6's review, I1.
pub(super) fn kept_final(
    line: &str,
    at: usize,
    len: usize,
    alphabet: fn(u8) -> bool,
) -> Option<u8> {
    let last = *eaten(line, at, len)?.last()?;
    alphabet(last).then_some(last)
}

/// One part of a [`Run`], as a byte range of its text.
#[derive(Clone, Copy)]
pub(super) enum Part {
    /// Characters of the alphabet, which the strip leaves.
    Visible(usize, usize),
    /// An escape sequence or a hidden format character, which the strip removes.
    Invisible(usize, usize),
}

/// A run: its parts in order, and where its walk ended.
pub(super) struct Run<'a> {
    text: &'a str,
    alphabet: fn(u8) -> bool,
    pub(super) parts: Vec<Part>,
    pub(super) end: usize,
}

/// The run of `alphabet` that starts at `start` in `text`, read through escapes and
/// hidden characters. It ends at the first other character, or (`skip_other`, for a
/// pattern's capture) at the text's end, other characters skipped. One pass.
pub(super) fn run_at(
    text: &str,
    start: usize,
    alphabet: fn(u8) -> bool,
    skip_other: bool,
) -> Run<'_> {
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    let mut at = start;
    while at < text.len() {
        let piece = bytes[at..].iter().take_while(|b| alphabet(**b)).count();
        if piece > 0 {
            parts.push(Part::Visible(at, at + piece));
            at += piece;
        } else if let Some(step) = invisible_at(text, at) {
            parts.push(Part::Invisible(at, at + step));
            at += step;
        } else if skip_other {
            at += text[at..].chars().next().map_or(1, char::len_utf8);
        } else {
            break;
        }
    }
    Run {
        text,
        alphabet,
        parts,
        end: at,
    }
}

/// A run's three readings, each with where the last part it read ends.
pub(super) struct Readings {
    /// The visible characters (what the strip leaves), and where the last piece ends.
    pub(super) visible: String,
    pub(super) visible_end: Option<usize>,
    /// The visible characters and each escape's final byte of the alphabet (task 6's
    /// review, I1: the escape took the secret's next character as its final).
    pub(super) kept: String,
    pub(super) kept_end: usize,
    /// The visible characters and every byte of the alphabet each escape ate (task 9's
    /// review, A-I1: a CSI takes a secret's leading digits as parameters, and an escape
    /// its `/` or `+` as intermediates).
    pub(super) all: String,
    pub(super) all_end: usize,
}

impl Run<'_> {
    /// The bytes of the alphabet that the invisible part `at..end` eats with it.
    pub(super) fn eaten_of(&self, at: usize, end: usize) -> impl Iterator<Item = u8> + '_ {
        eaten(self.text, at, end - at)
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|b| (self.alphabet)(*b))
    }

    /// The text of the part `a..b`.
    pub(super) fn piece(&self, a: usize, b: usize) -> &str {
        &self.text[a..b]
    }

    /// The visible pieces' ranges.
    pub(super) fn pieces(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.parts.iter().filter_map(|p| match *p {
            Part::Visible(a, b) => Some((a, b)),
            Part::Invisible(..) => None,
        })
    }

    pub(super) fn readings(&self, start: usize) -> Readings {
        let mut r = Readings {
            visible: String::new(),
            visible_end: None,
            kept: String::new(),
            kept_end: start,
            all: String::new(),
            all_end: start,
        };
        for part in &self.parts {
            match *part {
                Part::Visible(a, b) => {
                    let piece = &self.text[a..b];
                    for reading in [&mut r.visible, &mut r.kept, &mut r.all] {
                        reading.push_str(piece);
                    }
                    (r.visible_end, r.kept_end, r.all_end) = (Some(b), b, b);
                }
                Part::Invisible(a, b) => {
                    if let Some(f) = kept_final(self.text, a, b - a, self.alphabet) {
                        r.kept.push(char::from(f));
                        r.kept_end = b;
                    }
                    let before = r.all.len();
                    r.all.extend(self.eaten_of(a, b).map(char::from));
                    if r.all.len() > before {
                        r.all_end = b;
                    }
                }
            }
        }
        r
    }
}
