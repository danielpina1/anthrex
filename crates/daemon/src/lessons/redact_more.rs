//! Milestone 9.18 decision 6, task M9.18.4: the redaction residuals the 9.15 review
//! found, beside [`super`]'s rules (it is 9.15 decision 11's redaction, best-effort
//! still). An AWS secret access key after its key name or on its key id's line, a
//! `Basic` credential, and a URL's user information spelled JSON-escaped (`:\/\/`) or
//! percent-encoded (`%3A%2F%2F`, `%40`). Pure; each rule is one pass over the text,
//! and the bytes its checks read count in [`super::BODY_READ`] (ruling C6).

use std::sync::LazyLock;

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
    re(r#"(?i)(secret_access_key|secretaccesskey)(["']?[ \t]*[=:][ \t]*["']?)([A-Za-z0-9/+]+)"#)
});
/// An AWS access key id, for rule (b).
static AWS_KEY_ID: LazyLock<Regex> = LazyLock::new(|| re(r"AKIA[0-9A-Z]{16}"));

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

/// Rule (b), ruling C5: on a line holding an `AKIA` key id, every secret access key.
/// It runs before `unambiguous`, which replaces the id. Only a line with an id is
/// scanned, once; the search resumes at its end.
pub(super) fn aws_beside_id(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while let Some(m) = AWS_KEY_ID.find_at(text, from) {
        let start = text[from..m.start()]
            .rfind('\n')
            .map_or(from, |i| from + i + 1);
        let end = text[m.end()..]
            .find('\n')
            .map_or(text.len(), |i| m.end() + i);
        out.push_str(&text[from..start]);
        secrets_in_line(&text[start..end], &mut out);
        from = end;
    }
    out.push_str(&text[from..]);
    out
}

/// `line` onto `out`, each whole run that [`is_aws_secret`] replaced. Runs are ASCII,
/// so every cut is a character boundary.
fn secrets_in_line(line: &str, out: &mut String) {
    read(line.len());
    let mut at = 0;
    while at < line.len() {
        let run = line[at..].bytes().take_while(|b| aws_char(*b)).count();
        if run > 0 {
            let word = &line[at..at + run];
            out.push_str(if is_aws_secret(word) { TOKEN } else { word });
            at += run;
        } else {
            let gap = line[at..].bytes().take_while(|b| !aws_char(*b)).count();
            out.push_str(&line[at..at + gap]);
            at += gap;
        }
    }
}

/// `Basic` (any case, with no left boundary, as `Bearer`) and whitespace, then a
/// base64 run of at least [`BASIC_MIN`] characters.
static BASIC_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?i)basic[ \t]+([A-Za-z0-9+/]{{{BASIC_MIN},}}={{0,2}})"
    ))
});

/// A `Basic` credential whose decoding is UTF-8 holding a `:` becomes [`BASIC`];
/// `Basic usage` and a base64 word with no colon are left.
pub(super) fn basic(text: &str) -> String {
    BASIC_RE
        .replace_all(text, |c: &Captures| {
            read(c[1].len());
            let decoded = base64(&c[1]).and_then(|bytes| String::from_utf8(bytes).ok());
            if decoded.is_some_and(|s| s.contains(':')) {
                BASIC.to_string()
            } else {
                c[0].to_string()
            }
        })
        .into_owned()
}

/// Ruling C7: standard base64, by hand (no dependency), bounded by `run`'s length.
/// Padding is optional; a length that leaves one character over is not base64.
fn base64(run: &str) -> Option<Vec<u8>> {
    let body = run.trim_end_matches('=');
    if body.len() % 4 == 1 {
        return None;
    }
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
