//! Milestone 9.15 decision 11's redaction, task M9.15.8: what a distill dossier must
//! never carry. Its scope is the brief's: PEM private keys, prefixed tokens, JWTs,
//! bearer credentials, and a URL's user information (as `host::remote::redact` does).
//! Portuguese IBAN/NIF and daemon tokens are not ported. Pure; the patterns compile
//! once.
//!
//! The text it sees may already have been clipped (a check's tail, a signature's
//! headline, decision 5) or rewritten by decision 5's normaliser, so each pattern also
//! takes the shapes a clip leaves: a key block missing either marker, a user's password
//! cut at a line's end, and an unambiguous token cut short or split by one of the
//! normaliser's placeholders. Task 13's review, I1 (the controller's ruling): an
//! unambiguous prefix (`ghp_`, `AKIA`, `glpat-`, `xoxb-`, `sk-ant-`…) matches anywhere,
//! with no left boundary, and is a token from four body characters on, so a clipped or
//! split one is replaced from its prefix onward whatever stands before it. The ruling
//! after that fix: the body must also hold an uppercase letter or a digit (real token
//! bodies are random base62), so `laughs_count` and `task-proj-name` stay words, and a
//! token clipped to an all-lowercase body is the accepted residual. The ambiguous `sk-`, `npm_` and
//! `Bearer` are judged by their body alone (a digit and a minimum length, task 12's
//! review, I2), so a short, clipped or all-letter one is not caught: redaction is
//! best-effort defence in depth, not a guarantee.
//!
//! [`scrub`] is the one safe entry point for text that may carry terminal escapes,
//! written out or real (task 10's review, I1 and I2; the controller's ruling at
//! M9.15.11): it decodes text-escaped control characters, redacts, strips the escapes
//! and redacts again. The escape stripper is private to this file, so no caller can
//! strip before it redacts (an escape's final can take a token's first character).

use std::sync::LazyLock;

use proto::safe_text::is_hidden_format;
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
/// The ambiguous `sk-` and `npm_` tokens' body characters, at least (task 12's review,
/// I2: the controller's ruling).
pub const AMBIGUOUS_MIN: usize = 20;
/// A bearer credential's characters, at least (the same ruling).
pub const BEARER_MIN: usize = 16;

#[cfg(test)]
thread_local! {
    /// Characters the body checks (the ambiguous prefixes' digit check and the
    /// unambiguous prefixes' [`body_end`]) read on this thread: the linearity tests' measure, with no wall-clock bound.
    pub(super) static BODY_READ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

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
/// Task 12's review, I2 and M7 (the controller's ruling): `Bearer` and whitespace,
/// anywhere, with no left boundary; [`bearer`] keeps it only when the credential is at
/// least [`BEARER_MIN`] characters holding a digit, so `Bearer authentication-required`
/// is prose and no escape spelling before `Bearer` can hide a credential.
static BEARER_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?i)bearer[ \t]+([A-Za-z0-9._~+/=-]{{{BEARER_MIN},}})"
    ))
});
/// Three dot-separated base64url segments, the first two JSON objects (`eyJ`); a clip
/// may have cut the signature. Task 9's review, I2 (the controller's ruling): the shape
/// is unambiguous, so whatever stands before it (an escape that ends in a letter, an
/// encoded colour code) cannot hide it.
static JWT: LazyLock<Regex> =
    LazyLock::new(|| re(r"eyJ[A-Za-z0-9_-]{2,}\.eyJ[A-Za-z0-9_-]{2,}(?:\.[A-Za-z0-9_-]*)?"));
/// Task 9's review, I2 (the controller's ruling): the unambiguous prefixes, whole
/// tokens of at least [`TOKEN_MIN`] characters, with no left boundary: no list of what
/// may stand before a token (`\u001b[31m`, `\x1b`, `\b`, `\U…`, `%253D`) is ever
/// complete. AWS's key id in its full shape (and the token characters after it). Task
/// 10's review, I1: OpenAI's `sk-proj-`, `sk-svcacct-` and `sk-admin-`. A clipped or
/// placeholder-split token of these prefixes is [`PREFIXED`]'s. The controller's
/// ruling after task 13's fix: a body with no uppercase letter and no digit is a word
/// ([`unambiguous`]).
static UNAMBIGUOUS: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?:github_pat_|gh[pousr]_|glpat-|sk-ant-|sk-proj-|sk-svcacct-|sk-admin-|xox[abprs]-)([A-Za-z0-9_-]{16,})|AKIA[0-9A-Z]{16}[A-Za-z0-9_-]*",
    )
});
/// Task 12's review, I2 and M7 (the controller's ruling): the ambiguous `sk-` (any
/// variant: `sk-proj-` and the rest) and `npm_`, anywhere, with no left boundary: no
/// list of what may stand before a prefix is ever complete (`\33[?25l`, `^[[38:5:196m`,
/// `\33(B`, `&#27;&#91;`…), and a word glued to it (`tokensk-…`) is no defence either.
/// The body is judged instead ([`ambiguous`]): at least [`AMBIGUOUS_MIN`] token
/// characters holding a digit, so `ask-for-help-with-the-thing`, `task-sk-ipped` and
/// `npm_install_script_name` are words. The residual gap: a secret with no digit, or a
/// shorter one, a clipped one, or one split by a placeholder.
static AMBIGUOUS: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"(?:sk-|npm_)[A-Za-z0-9_-]{{{AMBIGUOUS_MIN},}}")));
/// The least body a clipped or placeholder-split unambiguous token keeps (task 13's
/// review, I1: the controller's ruling).
pub const UNAMBIGUOUS_BODY_MIN: usize = 4;

/// Task 13's review, I1 (the controller's ruling): every unambiguous prefix, anywhere,
/// with no left boundary (no list of what may stand before one is ever complete:
/// `^[[31m`, `\33[31m`, `&#27;[31m`, `%1b%5B31m` each end in a letter). [`prefixed`]
/// reads its body.
static PREFIXED: LazyLock<Regex> = LazyLock::new(|| {
    re(r"github_pat_|gh[pousr]_|glpat-|sk-ant-|sk-proj-|sk-svcacct-|sk-admin-|xox[abprs]-|AKIA")
});
/// A real escape sequence at the start of the text, in [`ESCAPE`]'s forms.
static ESCAPE_HERE: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"^(?:\x1b\[[0-?]*[ -/]*[@-~]|\x1b[\]PX^_][^\x07\x1b]*(?:\x07|\x1b\\)?|\x1b[ -/]*[0-~]?",
        r"|\x{9b}[0-?]*[ -/]*[@-~]|\x{9b})"
    ))
});
/// Decision 5's placeholders, which count as a token's characters.
const PLACEHOLDERS: [&str; 8] = [
    "<sha>", "<id>", "<dur>", "<l>", "<c>", "<tmp>", "<time>", "<tid>",
];

/// The end of an unambiguous token's body that starts at `start`, how many body
/// characters it holds, and whether it holds an uppercase letter, a digit or a
/// placeholder (the controller's ruling after task 13's fix: real token bodies are
/// random base62; a placeholder stands for normalised digits or hex): the prefix's own alphabet (`AKIA`'s is `[0-9A-Z]`, the others'
/// `[A-Za-z0-9_-]`) and decision 5's placeholders count; a real escape sequence (which
/// [`scrub`] strips) and a hidden format character (which `multi_line` and `one_line`
/// drop) are read through without counting, so a token they split is replaced whole and
/// leaves no remainder once they are gone. One pass; each character is read once.
fn body_end(text: &str, start: usize, aws: bool) -> (usize, usize, bool) {
    let mut at = start;
    let mut count = 0;
    let mut strong = false;
    while let Some(c) = text[at..].chars().next() {
        let rest = &text[at..];
        let step = if (aws && (c.is_ascii_uppercase() || c.is_ascii_digit()))
            || (!aws && (c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        {
            count += 1;
            strong |= c.is_ascii_uppercase() || c.is_ascii_digit();
            1
        } else if let Some(p) = PLACEHOLDERS.iter().find(|p| rest.starts_with(**p)) {
            count += p.len();
            strong = true;
            p.len()
        } else if c == '\u{1b}' || c == '\u{9b}' {
            ESCAPE_HERE
                .find(rest)
                .map_or(c.len_utf8(), |m| m.end().max(1))
        } else if is_hidden_format(c) {
            c.len_utf8()
        } else {
            break;
        };
        at += step;
    }
    #[cfg(test)]
    BODY_READ.with(|n| n.set(n.get() + (at - start) as u64));
    (at, count, strong)
}

/// Task 13's review, I1 (the controller's ruling): an unambiguous prefix with at least
/// [`UNAMBIGUOUS_BODY_MIN`] body characters ([`body_end`]), holding an uppercase letter,
/// a digit or a placeholder (the controller's ruling after that fix), is a token,
/// replaced from its prefix onward; any other is kept, so `laughs_count` and
/// `task-proj-name` are words. Linear: the search resumes after each body.
fn prefixed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while let Some(m) = PREFIXED.find_at(text, from) {
        let (end, count, strong) = body_end(text, m.end(), m.as_str() == "AKIA");
        out.push_str(&text[from..m.start()]);
        if count >= UNAMBIGUOUS_BODY_MIN && strong {
            out.push_str(TOKEN);
        } else {
            out.push_str(&text[m.start()..end]);
        }
        from = end;
    }
    out.push_str(&text[from..]);
    out
}

/// Whether `body` holds a digit; it is read once (counted in the tests).
fn has_digit(body: &str) -> bool {
    #[cfg(test)]
    BODY_READ.with(|c| c.set(c.get() + body.len() as u64));
    body.bytes().any(|b| b.is_ascii_digit())
}

/// A whole unambiguous token ([`UNAMBIGUOUS`]) whose body holds an uppercase letter or
/// a digit (`AKIA`'s always does); an all-lowercase one is a word.
fn unambiguous(text: &str) -> String {
    UNAMBIGUOUS
        .replace_all(text, |c: &Captures| match c.get(1) {
            Some(body)
                if !body
                    .as_str()
                    .bytes()
                    .any(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) =>
            {
                c[0].to_string()
            }
            _ => TOKEN.to_string(),
        })
        .into_owned()
}

fn ambiguous(text: &str) -> String {
    AMBIGUOUS
        .replace_all(text, |c: &Captures| {
            if has_digit(&c[0]) {
                TOKEN.to_string()
            } else {
                c[0].to_string()
            }
        })
        .into_owned()
}

fn bearer(text: &str) -> String {
    BEARER_RE
        .replace_all(text, |c: &Captures| {
            if has_digit(&c[1]) {
                BEARER.to_string()
            } else {
                c[0].to_string()
            }
        })
        .into_owned()
}

fn jwt(text: &str) -> String {
    JWT.replace_all(text, TOKEN).into_owned()
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
    let text = unambiguous(&text);
    let text = ambiguous(&text);
    prefixed(&text)
}

/// Text-escaped characters: a JSON surrogate pair (kept whole), JSON's `\uXXXX`,
/// Rust's `\u{…}`, `\xNN`, octal `\0NN`/`\NNN`, and percent encoding.
static TEXT_ESCAPE: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"\\u[dD][89abAB][0-9a-fA-F]{2}\\u[dD][c-fC-F][0-9a-fA-F]{2}",
        r"|\\u([0-9a-fA-F]{4})|\\u\{([0-9a-fA-F]{1,6})\}|\\x([0-9a-fA-F]{2})",
        r"|\\(0?[0-7]{3})|%([0-9a-fA-F]{2})"
    ))
});

/// Decoding passes at most: until the text stops changing (`%25253D` takes three).
/// Task 11's review, I1 (the controller's ruling): eight, so percent encoding nested
/// seven levels deep still decodes; each pass is linear, so the whole stays linear.
const DECODE_ROUNDS: usize = 8;

/// `text` with every text-escaped control character (an ESC, written `\u001b`,
/// `\u{1b}`, `\x1b`, `\033` or `%1B`) decoded, and `%25` decoded to `%`, until stable
/// or [`DECODE_ROUNDS`]. Anything else written out is kept as it is, so `%20`, `\u00e9`
/// and a path's `\x` stay, and a redaction's placeholder (letters, brackets, `*`) is
/// never touched. Each round is one linear pass.
fn decode(text: &str) -> String {
    let mut text = text.to_string();
    for _ in 0..DECODE_ROUNDS {
        let next = TEXT_ESCAPE
            .replace_all(&text, |c: &Captures| {
                let whole = &c[0];
                let value = [(1, 16), (2, 16), (3, 16), (4, 8), (5, 16)]
                    .iter()
                    .find_map(|&(group, radix)| {
                        let digits = c.get(group)?.as_str();
                        Some((group, u32::from_str_radix(digits, radix).ok()?))
                    });
                match value.and_then(|(group, v)| Some((group, char::from_u32(v)?))) {
                    Some((_, ch)) if ch.is_control() => ch.to_string(),
                    Some((5, '%')) => "%".to_string(),
                    _ => whole.to_string(),
                }
            })
            .into_owned();
        if next == text {
            break;
        }
        text = next;
    }
    text
}

/// Ruling C10 (a): terminal escape sequences. Task 9's review, I1 (the controller's
/// ruling): a CSI; an OSC, DCS, SOS, PM or APC string up to its BEL or ST; ECMA-48's
/// general form `ESC <intermediates 0x20-0x2F> <final 0x30-0x7E>` (nF escapes such as
/// `tput sgr0`'s `ESC ( B`, and `ESC # 8`); a lone ESC; and (task 11's review, I1) the
/// single-byte CSI (U+009B), with its sequence or alone. The general form can take a
/// token's first character as its final (`ESC g…`), so it is private: only [`scrub`]
/// strips, after it redacts.
static ESCAPE: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b[\]PX^_][^\x07\x1b]*(?:\x07|\x1b\\)?|\x1b[ -/]*[0-~]?",
        r"|\x{9b}[0-?]*[ -/]*[@-~]|\x{9b}"
    ))
});

/// `text` without its escape sequences ([`ESCAPE`]). An ESC right before a redaction's
/// placeholder (`[redacted …]`, `Bearer [redacted]`, which a redaction before the strip
/// leaves) goes alone, so the strip cannot eat the placeholder's first characters.
fn strip_escapes(text: &str) -> String {
    ESCAPE
        .replace_all(text, |c: &Captures| {
            let m = c.get(0).expect("the whole match");
            // ESC is one byte, the single-byte CSI two.
            let lead = m.as_str().chars().next().map_or(1, char::len_utf8);
            let after = &text[m.start() + lead..];
            if after.starts_with("[redacted") || after.starts_with("Bearer [redacted") {
                m.as_str()[lead..].to_string()
            } else {
                String::new()
            }
        })
        .into_owned()
}

/// The one safe entry point (task 10's review, I1 and I2): text-escaped control
/// characters decoded, then redacted, escapes stripped, and redacted again. A caller
/// that needs one line or marker breaking applies them to its result, and redacts once
/// more after `multi_line`/`one_line`, which drop hidden characters that could split a
/// token. Idempotent on what it returns, pure, and linear in the text.
pub fn scrub(text: &str) -> String {
    redact(&strip_escapes(&redact(&decode(text))))
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "scrub_tests.rs"]
mod scrub_tests;

#[cfg(test)]
#[path = "scrub_tests_spellings.rs"]
mod scrub_tests_spellings;

#[cfg(test)]
#[path = "scrub_tests_unambiguous.rs"]
mod scrub_tests_unambiguous;
