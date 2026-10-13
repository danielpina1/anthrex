//! Milestone 9.15 decision 11's redaction, task M9.15.8: what a distill dossier must
//! never carry. Its scope is the brief's: PEM private keys, prefixed tokens, JWTs,
//! bearer credentials, and a URL's user information (as `host::remote::redact` does).
//! Portuguese IBAN/NIF and daemon tokens are not ported. Pure; the patterns compile
//! once.
//!
//! The text it sees may already have been clipped (a check's tail, a signature's
//! headline, decision 5) or rewritten by decision 5's normaliser, so each pattern also
//! takes the shapes a clip leaves: a key block missing either marker, and a token, a
//! bearer credential or a user's password cut at a line's end or split by one of the
//! normaliser's placeholders. Wrongly replacing ordinary text costs the distiller a
//! word; letting a piece of a secret through is not allowed.

use std::sync::LazyLock;

use regex::{Captures, Regex};

/// What a private key block becomes.
pub const PRIVATE_KEY: &str = "[redacted private key]";
/// What a prefixed token or a JWT becomes.
pub const TOKEN: &str = "[redacted token]";
/// What a bearer credential becomes, with its scheme.
pub const BEARER: &str = "Bearer [redacted]";
/// A URL's user information.
const USERINFO: &str = "***";

/// A prefixed token's characters after its prefix, at least (the brief).
pub const TOKEN_MIN: usize = 16;
/// A bearer credential's characters, at least, unless a line's end cut it.
const BEARER_MIN: usize = 8;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid redaction pattern")
}

/// A private key's marker line (`RSA `, `EC `, `OPENSSH `, `ENCRYPTED `, a PGP block…).
macro_rules! key_marker {
    ($edge:literal) => {
        concat!("-----", $edge, " [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----")
    };
}

/// A whole block, its newlines real or escaped (`\n` inside a JSON string).
static PEM: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        "(?s)",
        key_marker!("BEGIN"),
        ".*?",
        key_marker!("END")
    ))
});
/// A block a head clip cut: its BEGIN marker on, to the text's end.
static PEM_BEGIN: LazyLock<Regex> =
    LazyLock::new(|| re(concat!("(?s)", key_marker!("BEGIN"), ".*")));
/// A block a tail clip cut: the base64 lines (and escaped newlines) before its END.
static PEM_END: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(?:[A-Za-z0-9+/=\r\n]|\\[nr])*",
        key_marker!("END")
    ))
});
/// `<scheme>://<userinfo>@`: the user information runs to the authority's last `@`.
static URL_USERINFO: LazyLock<Regex> =
    LazyLock::new(|| re(r"([A-Za-z][A-Za-z0-9+.-]*://)[^\s/?#]*@"));
/// `<scheme>://<user>:<password>` a clip cut before its `@`.
static URL_CUT: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?m)([A-Za-z][A-Za-z0-9+.-]*://)[^\s/?#@:]*:([^\s/?#@]*)$"));
/// Task 8's review, I2: what may stand right before a token besides a character that
/// cannot be part of a word: a JSON escape (`\n`, `\r`, `\t`, `\uXXXX`) or a percent
/// encoding (`%3D`, `%20`), each of which ends in an alphanumeric character.
macro_rules! escape_boundary {
    () => {
        r"\\[nrt]|\\u[0-9A-Fa-f]{4}|%[0-9A-Fa-f]{2}"
    };
}

/// `Bearer <credential>`, after a non-word character or an escape.
static BEARER_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(?i)(^|[^A-Za-z0-9_]|",
        escape_boundary!(),
        r")(bearer[ \t]+[A-Za-z0-9._~+/=-]+)"
    ))
});
/// Three dot-separated base64url segments, the first two JSON objects (`eyJ`); a clip
/// may have cut the signature. After a non-word character or an escape.
static JWT: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(^|[^A-Za-z0-9_]|",
        escape_boundary!(),
        r")eyJ[A-Za-z0-9_-]{2,}\.eyJ[A-Za-z0-9_-]{2,}(?:\.[A-Za-z0-9_-]*)?"
    ))
});
/// A prefix with what stands before it (a non-alphanumeric character or an escape), and
/// the token characters after it, where decision 5's placeholders count as token
/// characters.
static PREFIXED: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(^|[^A-Za-z0-9]|",
        escape_boundary!(),
        r")",
        r"(github_pat_|gh[pousr]_|glpat-|sk-ant-|sk-|xox[abprs]-|AKIA|npm_)",
        r"((?:[A-Za-z0-9_-]|<(?:sha|id|dur|l|c|tmp|time|tid)>)*)"
    ))
});

/// Whether `text` ends at `end`, or a line does.
fn at_line_end(text: &str, end: usize) -> bool {
    matches!(text[end..].chars().next(), None | Some('\n' | '\r'))
}

fn prefixed(text: &str) -> String {
    PREFIXED
        .replace_all(text, |c: &Captures| {
            let (before, prefix, rest) = (&c[1], &c[2], &c[3]);
            // `sk-` inside a word (`task-sk-ip`) is a word.
            let in_word = prefix.starts_with("sk-") && matches!(before, "-" | "_");
            let end = c.get(0).map_or(0, |m| m.end());
            let token = rest.chars().count() >= TOKEN_MIN
                || rest.contains('<')
                || (!rest.is_empty() && at_line_end(text, end));
            if token && !in_word {
                format!("{before}{TOKEN}")
            } else {
                c[0].to_string()
            }
        })
        .into_owned()
}

fn bearer(text: &str) -> String {
    BEARER_RE
        .replace_all(text, |c: &Captures| {
            let m = c.get(0).expect("the whole match");
            let credential = c[2].split_whitespace().nth(1).unwrap_or("");
            if credential.len() >= BEARER_MIN || at_line_end(text, m.end()) {
                format!("{}{BEARER}", &c[1])
            } else {
                m.as_str().to_string()
            }
        })
        .into_owned()
}

fn jwt(text: &str) -> String {
    JWT.replace_all(text, |c: &Captures| format!("{}{TOKEN}", &c[1]))
        .into_owned()
}

fn url_cut(text: &str) -> String {
    URL_CUT
        .replace_all(text, |c: &Captures| {
            let after = &c[2];
            // `host:8080`: a port, not a password.
            if after.is_empty() || after.bytes().all(|b| b.is_ascii_digit()) {
                c[0].to_string()
            } else {
                format!("{}{USERINFO}", &c[1])
            }
        })
        .into_owned()
}

/// `text` with every secret decision 11's scope names replaced. Idempotent.
pub fn redact(text: &str) -> String {
    let text = PEM.replace_all(text, PRIVATE_KEY);
    let text = PEM_BEGIN.replace_all(&text, PRIVATE_KEY);
    let text = PEM_END.replace_all(&text, PRIVATE_KEY);
    let text = URL_USERINFO.replace_all(&text, format!("${{1}}{USERINFO}@"));
    let text = url_cut(&text);
    let text = bearer(&text);
    let text = jwt(&text);
    prefixed(&text)
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;
