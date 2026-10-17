//! Milestone 9.18 decision 6, task M9.18.4: the redaction residuals the 9.15 review
//! found, beside [`super`]'s rules (it is 9.15 decision 11's redaction, best-effort
//! still). An AWS secret access key after its key name or on its key id's line, and a
//! URL's user information spelled JSON-escaped (`:\/\/`) or percent-encoded
//! (`%3A%2F%2F`, `%40`); the `Basic` credential is in `redact_basic.rs`. Pure; each
//! rule is one pass over the text, and the bytes its checks read count in
//! [`super::BODY_READ`] (ruling C6).

use std::sync::LazyLock;

use regex::Regex;

use super::runs::{invisible_at, kept_final, read, run_at};
use super::{TOKEN, USERINFO, re};

/// An AWS secret access key's length, exactly.
pub const AWS_SECRET_LEN: usize = 40;

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

/// Whether the first [`AWS_SECRET_LEN`] characters of `run` are a secret access key:
/// task 9's review, A-M1 (`<id> <secret>abcdef…` on an id's line).
fn starts_in_secret(run: &str) -> bool {
    run.len() > AWS_SECRET_LEN && is_aws_secret(&run[..AWS_SECRET_LEN])
}

/// The fewest visible characters a run needs for [`restores_a_secret`], and for its
/// restored readings' first or last 40 characters to be judged: fix round 2
/// for task 9's review, m3. A run with fewer leaves no piece of a secret this long
/// (milestone 9.18's leak bound, the property tests' 30), so restoring it can only
/// redact text: a colour code's own parameters count toward 40, and one visible
/// character after enough of them was a "secret" (`ls --color` paths on an id's line).
pub const RESTORE_VISIBLE_MIN: usize = 30;

/// Whether restoring some of what a run's escapes ate makes its visible characters a
/// secret: task 9's review, A-I1. Fewer than [`AWS_SECRET_LEN`] are visible, but at
/// least [`RESTORE_VISIBLE_MIN`], and the [`aws_char`]s its escapes ate (`all` holds
/// them with the visible ones) reach it. An escape's own parameters can sit before the
/// secret's characters it took (`ESC[5;7` then `M…`), and its final can be the text
/// after the secret (`ESC,` then `+/"}`), so which of its bytes were the secret's is
/// not known, only how many could be.
fn restores_a_secret(visible: &str, all: &str) -> bool {
    (RESTORE_VISIBLE_MIN..AWS_SECRET_LEN).contains(&visible.len())
        && all.len() >= AWS_SECRET_LEN
        && all.bytes().any(|b| b.is_ascii_digit())
        && all.bytes().any(|b| b.is_ascii_uppercase())
        && all.bytes().any(|b| b.is_ascii_lowercase())
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
    /// after the key name once something stands before the run.
    Tail,
    /// [`Judge::Tail`], and also a longer run whose first 40 characters are a secret
    /// (task 9's review, A-M1): on an id's line.
    Ends,
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

/// `text` from `start` onto `out`, each secret access key replaced; returns where it
/// stopped. Task 4's review, I1: a run of [`aws_char`]s is read through escape
/// sequences and hidden format characters, which `scrub` strips only after this first
/// pass has replaced the id ([`secret_run`]). Task 6's review, I1: a run also starts at
/// an escape whose final byte is an [`aws_char`]. A `value` (rule (a)) stops at
/// whitespace, a quote, `,` or `}`, and a run that starts at its first character is
/// judged [`Judge::Exact`]; every other run of a value is judged [`Judge::Tail`], and
/// every run on an id's line [`Judge::Ends`]. Each byte is read at most three times: a run's
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
            } else if value {
                Judge::Tail
            } else {
                Judge::Ends
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
/// walk ended, so the scan stays linear. The run is a secret when one of three readings
/// ([`super::runs::Readings`]) is: its visible characters; its characters with each escape's eaten final kept
/// (task 6's review, I1); or its characters with every [`aws_char`] byte of each
/// escape kept (task 9's review, A-I1: a CSI takes a secret's leading digits as
/// parameters, and an escape its `/` or `+` as intermediates). Otherwise each visible
/// piece that is a secret is replaced (an id glued to its secret by an escape).
/// [`Judge::Tail`] also takes a run or piece whose last 40 characters are a secret, and
/// [`Judge::Ends`] one whose first 40 are.
fn secret_run(line: &str, start: usize, out: &mut String, judge: Judge) -> usize {
    let run = run_at(line, start, aws_char, false);
    let at = run.end;
    let pieces: Vec<(usize, usize)> = run.pieces().collect();
    let r = run.readings(start);
    let (visible, kept, all) = (&r.visible, &r.kept, &r.all);
    let (kept_end, all_end) = (r.kept_end, r.all_end);
    let tail = judge != Judge::Exact;
    let head = judge == Judge::Ends;
    let visible_end = r.visible_end;
    if let Some(end) = visible_end.filter(|_| is_aws_secret(visible)) {
        out.push_str(TOKEN);
        return end;
    }
    for (reading, end) in [(kept, kept_end), (all, all_end)] {
        if is_aws_secret(reading) {
            out.push_str(TOKEN);
            return end;
        }
    }
    if restores_a_secret(visible, all) {
        out.push_str(TOKEN);
        return all_end;
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
        let secret = is_aws_secret(word)
            || (tail && ends_in_secret(word))
            || (head && starts_in_secret(word));
        any |= secret;
        each.push_str(if secret { TOKEN } else { word });
        last = b;
    }
    if !any && tail {
        // Fix round 2's m3: the restored readings only with [`RESTORE_VISIBLE_MIN`]
        // visible characters, as [`restores_a_secret`].
        let restored = visible.len() >= RESTORE_VISIBLE_MIN;
        let readings = [(visible, visible_end), (kept, kept_end), (all, all_end)];
        for (i, (reading, end)) in readings.into_iter().enumerate() {
            if (i == 0 || restored)
                && (ends_in_secret(reading) || (head && starts_in_secret(reading)))
            {
                out.push_str(TOKEN);
                return end;
            }
        }
    }
    out.push_str(&each);
    visible_end
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
