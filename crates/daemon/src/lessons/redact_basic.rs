//! Milestone 9.18 decision 6, task M9.18.4: a `Basic` credential, beside [`super`]'s
//! rules. Moved out of `redact_more.rs` by fix round 2 for task 9's review (I2), which
//! reads the credential through escapes as the AWS rules read a secret
//! ([`super::runs`]).

use std::sync::LazyLock;

use regex::{Captures, Regex};

use super::runs::{Part, Run, read, run_at};
use super::{BASIC, re};

/// A `Basic` credential's base64 characters, at least.
pub const BASIC_MIN: usize = 8;

/// A `Basic` credential's characters: base64's, and its padding.
fn basic_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')
}

/// `Basic` (any case, with no left boundary, as `Bearer`) and whitespace, then a
/// base64 run, which [`basic`] needs to be at least [`BASIC_MIN`] characters (fix
/// round 2: the pattern counts an escape as one element, so it takes half as many).
/// Task 6's review, M1: the run is
/// read through escapes ([`super::ESCAPE_FORMS`]), whose final may be its first
/// character, and (fix round 2's I2) through hidden characters
/// ([`super::HIDDEN_FORMS`]).
static BASIC_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?i)basic[ \t]+((?:[A-Za-z0-9+/]|{}|{}){{{},}}={{0,2}})",
        super::ESCAPE_FORMS,
        super::HIDDEN_FORMS,
        BASIC_MIN / 2
    ))
});

/// A `Basic` credential whose decoding holds a `:` becomes [`BASIC`]; `Basic usage`
/// and a base64 word with no colon are left. Task 4's review, I2: a clipped credential
/// counts by its longest valid prefix. Task 6's review, M1, and fix round 2's I2: the
/// credential is read through its escapes ([`holds_colon`]).
pub(super) fn basic(text: &str) -> String {
    BASIC_RE
        .replace_all(text, |c: &Captures| {
            read(c[1].len());
            let run = run_at(&c[1], 0, basic_char, true);
            let all = run.readings(0).all;
            if all.trim_end_matches('=').len() >= BASIC_MIN && holds_colon(&run) {
                BASIC.to_string()
            } else {
                c[0].to_string()
            }
        })
        .into_owned()
}

/// Ruling C7: standard base64, by hand (no dependency); `None` for padding or any
/// other byte.
fn sextet(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Where a base64 decoder can be: its place in a four-character group, and the last
/// character's six bits (which the next byte needs). 4 × 64 states, as a bit set.
#[derive(Clone, Copy, Default)]
struct States([u64; 4]);

impl States {
    fn start() -> Self {
        let mut s = Self::default();
        s.0[0] = 1;
        s
    }

    fn is_empty(self) -> bool {
        self.0 == [0; 4]
    }

    fn union(self, other: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i] | other.0[i]))
    }

    /// Every state after one more character `b`, and whether a byte it completes is a
    /// `:` in any of them. Padding or a byte outside base64 ends every state.
    fn feed(self, b: u8) -> (Self, bool) {
        let mut next = Self::default();
        let Some(v) = sextet(b) else {
            return (next, false);
        };
        for (place, word) in self.0.iter().enumerate() {
            let mut bits = *word;
            while bits != 0 {
                let prev = bits.trailing_zeros() as u8;
                bits &= bits - 1;
                let byte = match place {
                    1 => Some(prev << 2 | v >> 4),
                    2 => Some((prev & 15) << 4 | v >> 2),
                    3 => Some((prev & 3) << 6 | v),
                    _ => None,
                };
                if byte == Some(b':') {
                    return (next, true);
                }
                let (to, last) = if place == 3 { (0, 0) } else { (place + 1, v) };
                next.0[to] |= 1 << last;
            }
        }
        (next, false)
    }
}

/// Whether some reading of the credential decodes to bytes holding a `:` (task 6's
/// review, M2: UTF-8 or not, so a Latin-1 password still counts). Fix round 2's I2:
/// the readings are those of an AWS secret, and more. An escape ate a suffix of its
/// bytes of the alphabet from the credential (its own parameters come first), so each
/// escape is read with any number of its first such bytes left out: none of them is the
/// `all` reading, all but the final the `kept` one, all of them the visible one, and the
/// ones between restore the base64's alignment after an escape whose own parameters
/// stood before the credential's characters it took. One pass, each byte fed to at most
/// 256 decoder states.
fn holds_colon(run: &Run<'_>) -> bool {
    let mut states = States::start();
    for part in &run.parts {
        match *part {
            Part::Visible(a, b) => {
                for byte in run.piece(a, b).bytes() {
                    let (next, colon) = states.feed(byte);
                    if colon {
                        return true;
                    }
                    states = next;
                }
            }
            Part::Invisible(a, b) => {
                // Still before the credential's characters it took, or already in them.
                let skipping = states;
                let mut taking = States::default();
                for byte in run.eaten_of(a, b) {
                    let (next, colon) = taking.union(skipping).feed(byte);
                    if colon {
                        return true;
                    }
                    taking = next;
                }
                states = taking.union(skipping);
            }
        }
        if states.is_empty() {
            return false;
        }
    }
    false
}
