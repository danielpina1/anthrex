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

/// Rule (a): a key name (any case), `=` or `:` with optional spaces and quotes, then a
/// run of the secret's alphabet (taken whole, so a longer run is not cut to 40).
static AWS_NAMED: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r#"(?i)(secret_access_key|secretaccesskey)((?:\\?["'])?[ \t]*[=:][ \t]*(?:\\?["'])?)([A-Za-z0-9/+]+)"#,
    )
});
/// An AWS access key id's prefix, for rule (b): a long-term `AKIA` or an STS `ASIA`
/// (task 4's review, I3). [`key_id_end`] reads its body.
static AWS_KEY_ID: LazyLock<Regex> = LazyLock::new(|| re(r"A[KS]IA"));
/// An AWS access key id's body characters after its prefix.
const AWS_KEY_ID_BODY: usize = 16;

/// Rule (a): the value after a secret access key's name, the name kept.
pub(super) fn aws_named(text: &str) -> String {
    AWS_NAMED
        .replace_all(text, |c: &Captures| {
            read(c[3].len());
            if is_aws_secret(&c[3]) {
                format!("{}{}{TOKEN}", &c[1], &c[2])
            } else {
                c[0].to_string()
            }
        })
        .into_owned()
}

/// Rule (b), ruling C5: on a line holding an `AKIA` or `ASIA` key id, every secret
/// access key. It runs before `unambiguous`, which replaces the id. Task 4's review,
/// I1: the id's body is read through escapes and hidden characters (as `prefixed`
/// reads it), because the first pass's `prefixed` replaces a split id too. Only a line
/// with an id is scanned, once; the search resumes at its end.
pub(super) fn aws_beside_id(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    let mut search = 0;
    while let Some(m) = AWS_KEY_ID.find_at(text, search) {
        let (body_end, count, _) = super::body_end(text, m.end(), |c| {
            c.is_ascii_uppercase() || c.is_ascii_digit()
        });
        if count < AWS_KEY_ID_BODY {
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
        secrets_in_line(&text[start..end], &mut out);
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

/// `line` onto `out`, each secret access key replaced. Task 4's review, I1: a run of
/// [`aws_char`]s is read through escape sequences and hidden format characters, which
/// `scrub` strips only after this first pass has replaced the id. A run whose
/// characters, the invisible ones left out, are a secret becomes one [`TOKEN`];
/// otherwise each of its visible pieces that is a secret does (an id glued to its
/// secret by an escape). An escape is never read as a run's characters (`ESC[1m`'s
/// `1m`). Each byte is read at most twice: a run's trailing escapes are read again as
/// gap.
fn secrets_in_line(line: &str, out: &mut String) {
    read(line.len());
    let bytes = line.as_bytes();
    let mut at = 0;
    while at < line.len() {
        if aws_char(bytes[at]) {
            at = secret_run(line, at, out);
        } else {
            let step = invisible_at(line, at)
                .unwrap_or_else(|| line[at..].chars().next().map_or(1, char::len_utf8));
            out.push_str(&line[at..at + step]);
            at += step;
        }
    }
}

/// The run that starts at `start` (an [`aws_char`]) onto `out`, as
/// [`secrets_in_line`] says; returns where its last visible piece ends.
fn secret_run(line: &str, start: usize, out: &mut String) -> usize {
    let bytes = line.as_bytes();
    let mut pieces: Vec<(usize, usize)> = Vec::new();
    let mut at = start;
    loop {
        let piece = bytes[at..].iter().take_while(|b| aws_char(**b)).count();
        if piece > 0 {
            pieces.push((at, at + piece));
            at += piece;
            continue;
        }
        match invisible_at(line, at) {
            Some(step) => at += step,
            None => break,
        }
    }
    let end = pieces.last().map_or(start, |p| p.1);
    let visible: String = pieces.iter().map(|&(a, b)| &line[a..b]).collect();
    if is_aws_secret(&visible) {
        out.push_str(TOKEN);
        return end;
    }
    let mut last = start;
    for &(a, b) in &pieces {
        out.push_str(&line[last..a]);
        let word = &line[a..b];
        out.push_str(if is_aws_secret(word) { TOKEN } else { word });
        last = b;
    }
    end
}

/// `Basic` (any case, with no left boundary, as `Bearer`) and whitespace, then a
/// base64 run of at least [`BASIC_MIN`] characters.
static BASIC_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?i)basic[ \t]+([A-Za-z0-9+/]{{{BASIC_MIN},}}={{0,2}})"
    ))
});

/// A `Basic` credential whose decoding is UTF-8 holding a `:` becomes [`BASIC`];
/// `Basic usage` and a base64 word with no colon are left. Task 4's review, I2: a
/// clipped credential counts by its longest valid prefix ([`base64`], [`holds_colon`]).
pub(super) fn basic(text: &str) -> String {
    BASIC_RE
        .replace_all(text, |c: &Captures| {
            read(c[1].len());
            if base64(&c[1]).is_some_and(|bytes| holds_colon(&bytes)) {
                BASIC.to_string()
            } else {
                c[0].to_string()
            }
        })
        .into_owned()
}

/// Whether `bytes`, read as UTF-8 up to a multibyte character a clip cut at their end,
/// hold a `:`. Bytes that are invalid before the end are not a credential.
fn holds_colon(bytes: &[u8]) -> bool {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) if e.error_len().is_none() => {
            std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or_default()
        }
        Err(_) => return false,
    };
    text.contains(':')
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
