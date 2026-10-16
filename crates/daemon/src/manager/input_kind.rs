//! Milestone 9.18 decision 10: whether a client's `Input` is only mouse reports, which
//! a wake-up's submit does not wait out as typing. Pure: no I/O, no clock, no lock.
//!
//! The forms (xterm's ctlseqs): SGR `ESC [ <` b `;` x `;` y then `M` or `m`; URXVT
//! `ESC [` b `;` x `;` y `M`; the default `ESC [ M` then three bytes of 0x20 or more,
//! and its UTF-8 form, whose three characters may each take two bytes. A default
//! report's high byte is not UTF-8 on its own, and two of them can read as one UTF-8
//! character, so both readings are tried: `only_mouse_reports` asks whether some cut of
//! the whole input into reports exists, over the input's offsets (linear).
//!
//! Each `Input` is classified alone: a report split across two `Input`s is two halves,
//! each typing, which leaves a wake unsubmitted as before this rule.

const ESC: u8 = 0x1b;

/// Whether `bytes` is one or more complete mouse reports and nothing else. An empty
/// input, a truncated report, or a report beside any other byte is typing.
pub fn only_mouse_reports(bytes: &[u8]) -> bool {
    if bytes.first() != Some(&ESC) {
        return false;
    }
    let mut reachable = vec![false; bytes.len() + 1];
    reachable[0] = true;
    for start in 0..bytes.len() {
        if reachable[start] {
            report_ends(&bytes[start..], &mut |len| reachable[start + len] = true);
        }
    }
    reachable[bytes.len()]
}

/// Calls `end` with the length of every report `rest` can start with.
fn report_ends(rest: &[u8], end: &mut impl FnMut(usize)) {
    let Some(body) = rest.strip_prefix(&[ESC, b'['][..]) else {
        return;
    };
    match body.first() {
        Some(b'<') => {
            if let Some(n) = three_params(&body[1..])
                && matches!(body.get(1 + n), Some(b'M' | b'm'))
            {
                end(3 + n + 1);
            }
        }
        Some(b'M') => default_chars(&body[1..], 3, 3, end),
        Some(_) => {
            if let Some(n) = three_params(body)
                && body.get(n) == Some(&b'M')
            {
                end(2 + n + 1);
            }
        }
        None => {}
    }
}

/// The length of `digits ; digits ; digits` at the start of `bytes`, each run non-empty.
fn three_params(bytes: &[u8]) -> Option<usize> {
    let mut at = 0;
    for param in 0..3 {
        if param > 0 {
            (bytes.get(at) == Some(&b';')).then_some(())?;
            at += 1;
        }
        let digits = bytes[at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        (digits > 0).then_some(())?;
        at += digits;
    }
    Some(at)
}

/// `left` more of a default report's characters from `bytes`; `used` bytes are already
/// taken. A character is one byte of 0x20 or more, or a two-byte UTF-8 sequence.
fn default_chars(bytes: &[u8], used: usize, left: usize, end: &mut impl FnMut(usize)) {
    if left == 0 {
        end(used);
        return;
    }
    let Some(&first) = bytes.first() else {
        return;
    };
    if first >= 0x20 {
        default_chars(&bytes[1..], used + 1, left - 1, end);
    }
    if (0xc2..=0xdf).contains(&first) && bytes.get(1).is_some_and(|b| (0x80..=0xbf).contains(b)) {
        default_chars(&bytes[2..], used + 2, left - 1, end);
    }
}

#[cfg(test)]
#[path = "input_kind_tests.rs"]
mod tests;
