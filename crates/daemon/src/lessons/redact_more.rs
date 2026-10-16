//! Milestone 9.18 decision 6, task M9.18.4: the redaction residuals the 9.15 review
//! found, beside [`super`]'s rules (it is 9.15 decision 11's redaction, best-effort
//! still). An AWS secret access key after its key name or on its key id's line, a
//! `Basic` credential, and a URL's user information spelled JSON-escaped (`:\/\/`) or
//! percent-encoded (`%3A%2F%2F`, `%40`). Pure; each rule is one pass over the text,
//! and the bytes its checks read count in [`super::BODY_READ`] (ruling C6).

use std::sync::LazyLock;

use proto::safe_text::is_hidden_format;
use regex::{Captures, Regex};

use super::{BASIC, TOKEN, USERINFO, re};

/// An AWS secret access key's length, exactly.
pub const AWS_SECRET_LEN: usize = 40;
/// A `Basic` credential's base64 characters, at least.
pub const BASIC_MIN: usize = 8;

/// Counts `n` bytes a check read (the linearity tests' measure).
fn read(n: usize) {
    #[cfg(test)]
    super::BODY_READ.with(|c| c.set(c.get() + n as u64));
    #[cfg(not(test))]
    let _ = n;
}

/// An AWS secret access key's alphabet.
fn aws_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'/' || b == b'+'
}

/// A whole run of [`aws_char`]s that is a secret access key: exactly
/// [`AWS_SECRET_LEN`] characters holding a digit, an uppercase and a lowercase letter
/// (so a hex sha stays).
fn is_aws_secret(run: &str) -> bool {
    run.len() == AWS_SECRET_LEN
        && run.bytes().any(|b| b.is_ascii_digit())
        && run.bytes().any(|b| b.is_ascii_uppercase())
        && run.bytes().any(|b| b.is_ascii_lowercase())
}

/// Whether the last [`AWS_SECRET_LEN`] characters of `run` are a secret access key:
/// task 6's review, I2. A written-out escape that `decode` leaves as text (`^[[1m`,
/// `\e[1m`, `&#27;[1m`, `%1b%5B1m`) glues its tail (`1m`, `B1m`) to the secret's run.
fn ends_in_secret(run: &str) -> bool {
    run.len() > AWS_SECRET_LEN && is_aws_secret(&run[run.len() - AWS_SECRET_LEN..])
}

/// Rule (a): a key name (any case), then `=` or `:` with optional spaces and quotes.
/// [`aws_named`] reads the value after it.
static AWS_NAMED: LazyLock<Regex> = LazyLock::new(|| {
    re(r#"(?i)(?:secret_access_key|secretaccesskey)(?:\\?["'])?[ \t]*[=:][ \t]*(?:\\?["'])?"#)
});
/// An AWS access key id's prefix, for rule (b): a long-term `AKIA` or an STS `ASIA`
/// (task 4's review, I3). [`super::body_end`] reads its body.
static AWS_KEY_ID: LazyLock<Regex> = LazyLock::new(|| re(r"A[KS]IA"));

/// How a run of the secret's alphabet is judged ([`secret_run`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Judge {
    /// Exactly [`AWS_SECRET_LEN`] characters: the whole value right after a key name,
    /// so a longer value is not cut to 40.
    Exact,
    /// Also a longer run whose last 40 characters are a secret (task 6's review, I2):
    /// on an id's line, and after the key name once something stands before the run.
    Tail,
}

/// Rule (a): the value after a secret access key's name, the name kept. Task 6's
/// review, I1 and I2: the value is read as [`secrets_in`] reads an id's line, up to
/// whitespace, a quote, `,` or `}`, so an escape before the secret (real, or written
/// out) cannot hide it. Linear: the search resumes where the value ends.
pub(super) fn aws_named(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while let Some(m) = AWS_NAMED.find_at(text, from) {
        out.push_str(&text[from..m.end()]);
        from = secrets_in(text, m.end(), &mut out, true);
    }
    out.push_str(&text[from..]);
    out
}

/// Rule (b), ruling C5: on a line holding an `AKIA` or `ASIA` key id, every secret
/// access key. It runs before `unambiguous`, which replaces the id. Task 4's review,
/// I1: the id's body is read through escapes and hidden characters (as `prefixed`
/// reads it). Task 6's review, I1: the line counts whenever `prefixed` would replace
/// the id (at least [`super::UNAMBIGUOUS_BODY_MIN`] body characters holding an
/// uppercase letter or a digit), so an escape that takes one of the id's characters
/// cannot leave its secret unscanned. Only a line with an id is scanned, once; the
/// search resumes at its end.
pub(super) fn aws_beside_id(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    let mut search = 0;
    while let Some(m) = AWS_KEY_ID.find_at(text, search) {
        let (body_end, count, strong) = super::body_end(text, m.end(), |c| {
            c.is_ascii_uppercase() || c.is_ascii_digit()
        });
        if count < super::UNAMBIGUOUS_BODY_MIN || !strong {
            search = body_end.max(m.end());
            continue;
        }
        let start = text[from..m.start()]
            .rfind('\n')
            .map_or(from, |i| from + i + 1);
        let end = text[m.end()..]
            .find('\n')
            .map_or(text.len(), |i| m.end() + i);
        out.push_str(&text[from..start]);
        secrets_in(&text[start..end], 0, &mut out, false);
        from = end;
        search = end;
    }
    out.push_str(&text[from..]);
    out
}

/// The length of the escape sequence or hidden format character at `at`, if one is
/// there: what `scrub`'s strip and the line breakers remove.
fn invisible_at(line: &str, at: usize) -> Option<usize> {
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

/// The final byte of the escape `line[at..at + len]` when `alphabet` holds it: task 6's
/// review, I1. ECMA-48's general form and a CSI take the next printable character as
/// their final, so the strip eats a secret's character with the escape. An OSC, DCS,
/// SOS, PM or APC string has no such final, and a hidden format character is no
/// escape.
fn kept_final(line: &str, at: usize, len: usize, alphabet: fn(u8) -> bool) -> Option<u8> {
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
    let last = *escape.last()?;
    (escape.len() > lead && alphabet(last)).then_some(last)
}

/// `run`'s characters of `alphabet` with its escapes and hidden characters left out
/// (what the strip leaves), and with each escape's final byte that `alphabet` holds
/// kept (what the strip ate): task 6's review, I1 and M1, for `Basic` and `Bearer`.
pub(super) fn visible_and_kept(run: &str, alphabet: fn(u8) -> bool) -> (String, String) {
    let (mut visible, mut kept) = (String::new(), String::new());
    let mut at = 0;
    while at < run.len() {
        let b = run.as_bytes()[at];
        if alphabet(b) {
            visible.push(char::from(b));
            kept.push(char::from(b));
            at += 1;
        } else if let Some(len) = invisible_at(run, at) {
            if let Some(f) = kept_final(run, at, len, alphabet) {
                kept.push(char::from(f));
            }
            at += len;
        } else {
            at += run[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    (visible, kept)
}

/// `text` from `start` onto `out`, each secret access key replaced; returns where it
/// stopped. Task 4's review, I1: a run of [`aws_char`]s is read through escape
/// sequences and hidden format characters, which `scrub` strips only after this first
/// pass has replaced the id ([`secret_run`]). Task 6's review, I1: a run also starts at
/// an escape whose final byte is an [`aws_char`]. A `value` (rule (a)) stops at
/// whitespace, a quote, `,` or `}`, and a run that starts at its first character is
/// judged [`Judge::Exact`]; every other run is judged [`Judge::Tail`]. Each byte is read at most three times: a run's
/// trailing escapes are read again as gap, or as a run with no visible piece, which
/// returns where its walk ended.
fn secrets_in(text: &str, start: usize, out: &mut String, value: bool) -> usize {
    let bytes = text.as_bytes();
    let mut at = start;
    while at < text.len() {
        let b = bytes[at];
        if value && (b.is_ascii_whitespace() || matches!(b, b'"' | b'\'' | b',' | b'}')) {
            break;
        }
        let invisible = invisible_at(text, at);
        if aws_char(b) || invisible.is_some_and(|n| kept_final(text, at, n, aws_char).is_some()) {
            // An escape before the value's first character is something before it.
            let judge = if value && at == start && aws_char(b) {
                Judge::Exact
            } else {
                Judge::Tail
            };
            at = secret_run(text, at, out, judge);
        } else {
            let step =
                invisible.unwrap_or_else(|| text[at..].chars().next().map_or(1, char::len_utf8));
            out.push_str(&text[at..at + step]);
            at += step;
        }
    }
    read(at - start);
    at
}

/// The run that starts at `start` (an [`aws_char`], or an escape whose final is one)
/// onto `out`, as [`secrets_in`] says; returns where the part that made it a secret
/// ends, else where its last visible piece ends, else (no visible piece) where its
/// walk ended, so the scan stays linear. Task 6's review, I1: the run is a secret when
/// its visible characters are, or its characters with each escape's eaten final kept
/// are; otherwise each visible piece that is a secret is replaced (an id glued to its
/// secret by an escape). [`Judge::Tail`] also takes a run or piece whose last 40
/// characters are a secret.
fn secret_run(line: &str, start: usize, out: &mut String, judge: Judge) -> usize {
    let bytes = line.as_bytes();
    let mut pieces: Vec<(usize, usize)> = Vec::new();
    let (mut visible, mut kept) = (String::new(), String::new());
    let mut kept_end = start;
    let mut at = start;
    loop {
        let piece = bytes[at..].iter().take_while(|b| aws_char(**b)).count();
        if piece > 0 {
            pieces.push((at, at + piece));
            visible.push_str(&line[at..at + piece]);
            kept.push_str(&line[at..at + piece]);
            at += piece;
            kept_end = at;
            continue;
        }
        let Some(step) = invisible_at(line, at) else {
            break;
        };
        if let Some(f) = kept_final(line, at, step, aws_char) {
            kept.push(char::from(f));
            kept_end = at + step;
        }
        at += step;
    }
    let tail = judge == Judge::Tail;
    let visible_end = pieces.last().map(|p| p.1);
    if let Some(end) = visible_end.filter(|_| is_aws_secret(&visible)) {
        out.push_str(TOKEN);
        return end;
    }
    if is_aws_secret(&kept) {
        out.push_str(TOKEN);
        return kept_end;
    }
    let Some(visible_end) = visible_end else {
        out.push_str(&line[start..at]);
        return at;
    };
    let mut each = String::new();
    let mut any = false;
    let mut last = start;
    for &(a, b) in &pieces {
        each.push_str(&line[last..a]);
        let word = &line[a..b];
        let secret = is_aws_secret(word) || (tail && ends_in_secret(word));
        any |= secret;
        each.push_str(if secret { TOKEN } else { word });
        last = b;
    }
    if !any && tail && ends_in_secret(&visible) {
        out.push_str(TOKEN);
        return visible_end;
    }
    if !any && tail && ends_in_secret(&kept) {
        out.push_str(TOKEN);
        return kept_end;
    }
    out.push_str(&each);
    visible_end
}

/// A `Basic` credential's characters: base64's, and its padding.
fn basic_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')
}

/// `Basic` (any case, with no left boundary, as `Bearer`) and whitespace, then a
/// base64 run of at least [`BASIC_MIN`] characters. Task 6's review, M1: the run is
/// read through escapes ([`super::ESCAPE_FORMS`]), whose final may be its first
/// character.
static BASIC_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?i)basic[ \t]+((?:[A-Za-z0-9+/]|{}){{{BASIC_MIN},}}={{0,2}})",
        super::ESCAPE_FORMS
    ))
});

/// A `Basic` credential whose decoding holds a `:` becomes [`BASIC`]; `Basic usage`
/// and a base64 word with no colon are left. Task 4's review, I2: a clipped credential
/// counts by its longest valid prefix ([`base64`], [`holds_colon`]). Task 6's review,
/// M1: the credential with its escapes left out, or with their eaten finals kept.
pub(super) fn basic(text: &str) -> String {
    BASIC_RE
        .replace_all(text, |c: &Captures| {
            read(c[1].len());
            let (visible, kept) = visible_and_kept(&c[1], basic_char);
            let credential = |run: &String| {
                run.trim_end_matches('=').len() >= BASIC_MIN
                    && base64(run).is_some_and(|bytes| holds_colon(&bytes))
            };
            if credential(&visible) || (kept != visible && credential(&kept)) {
                BASIC.to_string()
            } else {
                c[0].to_string()
            }
        })
        .into_owned()
}

/// Whether the decoded `bytes` hold a `:`. Task 6's review, M2: whether or not they
/// are UTF-8, so a Latin-1 password (RFC 7617's legacy charset) still counts.
fn holds_colon(bytes: &[u8]) -> bool {
    bytes.contains(&b':')
}

/// Ruling C7: standard base64, by hand (no dependency), bounded by `run`'s length.
/// Padding is optional. Task 4's review, I2: a clip may leave one character over (its
/// 6 bits make no byte), so that character is dropped, not the whole run.
fn base64(run: &str) -> Option<Vec<u8>> {
    let body = run.trim_end_matches('=');
    let body = if body.len() % 4 == 1 {
        &body[..body.len() - 1]
    } else {
        body
    };
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in body.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// A scheme then `://` spelled JSON-escaped (`:\/\/`) or percent-encoded (any case).
static URL_ENCODED: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)[a-z][a-z0-9+.-]*(?::\\/\\/|%3A%2F%2F)"));

/// A URL's user information in [`URL_ENCODED`]'s spellings, replaced as
/// `URL_USERINFO` replaces the plain one: to the authority's last `@` or `%40`, the
/// scheme and the at-sign kept as spelled. Linear: the search resumes at each
/// authority's end.
pub(super) fn encoded_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while let Some(m) = URL_ENCODED.find_at(text, from) {
        let (end, at) = authority(text, m.end());
        out.push_str(&text[from..m.end()]);
        match at {
            Some(at) => {
                out.push_str(USERINFO);
                out.push_str(&text[at..end]);
            }
            None => out.push_str(&text[m.end()..end]),
        }
        from = end;
    }
    out.push_str(&text[from..]);
    out
}

/// The end of the authority that starts at `start` (whitespace, `/`, `?`, `#` or
/// their `%2F`, `%3F`, `%23`), and where its last `@` or `%40` is.
fn authority(text: &str, start: usize) -> (usize, Option<usize>) {
    let bytes = text.as_bytes();
    let mut at = start;
    let mut last = None;
    while at < bytes.len() {
        let b = bytes[at];
        if b.is_ascii_whitespace() || matches!(b, b'/' | b'?' | b'#') {
            break;
        }
        if b == b'@' {
            last = Some(at);
        } else if b == b'%' {
            let code = bytes.get(at + 1..at + 3).map(<[u8]>::to_ascii_uppercase);
            match code.as_deref() {
                Some(b"2F" | b"3F" | b"23") => break,
                Some(b"40") => last = Some(at),
                _ => {}
            }
        }
        at += 1;
    }
    read(at - start);
    (at, last)
}
