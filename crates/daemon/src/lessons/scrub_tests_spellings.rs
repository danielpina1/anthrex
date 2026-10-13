//! Task 12's review, I2 and M7 (the controller's ruling after M9.15.13, replacing the
//! colour-code-tail boundary of task 11's review): the ambiguous `sk-`, `npm_` and
//! `Bearer` have no left boundary at all; their body is judged instead. `sk-` (any
//! variant) and `npm_` need at least 20 token characters holding a digit; `Bearer`
//! and whitespace need a credential of at least 16 characters holding a digit. So no
//! escape spelling, listed or not, can hide one, and a word glued to the prefix
//! cannot either. The residual gap is a secret with no digit, or a shorter one. The
//! single-byte CSI (U+009B) is still an ESC, and percent encoding is still decoded
//! until it stops changing (at most eight passes). Every fake secret is built at run
//! time.

use super::{BEARER, TOKEN, scrub};

/// Task 11's review, I1, and task 12's review, I2: escape spellings, each placed right
/// before the three ambiguous prefixes.
const SPELLINGS: [&str; 30] = [
    // Task 11's review, I1.
    r"\33[31m",
    r"\E[31m",
    r"^[[31m",
    "&#27;[31m",
    "&#x1b;[31m",
    "&#X1B;[1m",
    r"\U0000001b[31m",
    "\u{241b}[31m",
    "%1b%5B31m",
    "%1B%5b1;31m",
    "\u{9b}31m",
    r"\u009b31m",
    r"\u{9b}31m",
    "%9B31m",
    "%2525251B[31m",
    "%252525251B[31m",
    "%25252525251B%5B31m",
    // Task 12's review, I2.
    r"\33[?25l",
    r"^[[?25l",
    r"\E[?1049h",
    r"^[[38:5:196m",
    r"\33[38:2:255:0:0m",
    r"\33[>4;1m",
    "%1b%5b%3f25l",
    r"\33(B",
    r"^[(B",
    r"\e(B",
    "^]8;;http://x^G",
    "&#27;&#91;31m",
    "\u{fffd}31m",
];

/// The ambiguous secrets: (the secret, a fragment that must not survive).
fn secrets() -> Vec<(String, String)> {
    let body = "FAKE0fake1".repeat(4);
    let credential = "zFAKE0cred".repeat(4);
    vec![
        (format!("sk-{body}"), body.clone()),
        (format!("sk-proj-{body}"), body.clone()),
        (format!("npm_{body}"), body),
        (format!("Bearer {credential}"), credential.clone()),
        (format!("bearer {credential}"), credential),
    ]
}

fn assert_scrubbed(input: &str, fragment: &str) {
    let out = scrub(input);
    assert!(
        out.contains(TOKEN) || out.contains(BEARER),
        "{input:?} -> {out:?}"
    );
    assert!(!out.contains(fragment), "{input:?} -> {out:?}");
    assert!(!out.contains(['\u{1b}', '\u{9b}']), "{input:?} -> {out:?}");
    assert_eq!(scrub(&out), out, "{input:?}: idempotent");
}

#[test]
fn every_escape_spelling_before_every_ambiguous_prefix_is_scrubbed() {
    for (secret, fragment) in secrets() {
        for form in SPELLINGS {
            for input in [
                format!("{form}{secret}"),
                format!("key{form}{secret}"),
                format!("+ echo -e '{form}{secret}'"),
                format!("{{\"msg\":\"{form}{secret}\\u001b[0m\"}}"),
            ] {
                assert_scrubbed(&input, &fragment);
            }
        }
    }
}

/// The old residual gap: a word glued to the prefix no longer hides the token.
#[test]
fn a_word_glued_to_the_prefix_is_scrubbed() {
    for (secret, fragment) in secrets() {
        for word in ["token", "x", "API_KEY=", "the", "9"] {
            assert_scrubbed(&format!("{word}{secret} rest"), &fragment);
        }
    }
}

/// The single-byte CSI, real or written out, is stripped as an ESC's CSI is.
#[test]
fn the_single_byte_csi_is_stripped() {
    for text in ["a\u{9b}31mb", r"a\u009b31mb", r"a\u{9b}1;31mb", "a%9B31mb"] {
        assert_eq!(scrub(text), "ab", "{text:?}");
    }
    // A placeholder right after it keeps its first characters.
    let token = format!("ghp_{}", "FAKE".repeat(9));
    let out = scrub(&format!("\u{9b}31m{token}"));
    assert_eq!(out, TOKEN);
}

/// Percent encoding is decoded until it stops changing, up to eight passes; a long
/// input of nested encodings decodes whole (each pass is one linear regex pass).
#[test]
fn percent_encoding_decodes_until_stable() {
    let seven = format!("a%{}1B[31mb", "25".repeat(6));
    assert_eq!(scrub(&seven), "ab", "{seven}");
    let long = format!("%{}1B[31m", "25".repeat(6)).repeat(5_000);
    assert_eq!(scrub(&long), "");
}

/// Task 12's review, M7: with no boundary to misread, words, markdown links, a bearer
/// scheme in prose and SHAs stay. A body with no digit, or a short one, is a word.
#[test]
fn words_links_and_shas_are_not_over_redacted() {
    let sha = "3f2a9c01".repeat(5);
    for text in [
        "ask-for-help-with-the-thing".to_string(),
        "[ask-for-help](url)".to_string(),
        "see [ask-for-confirmation-before-delete](docs/x.md)".to_string(),
        "%5Bask-for-confirmation-dialog-component%5D".to_string(),
        "Bearer authentication-required".to_string(),
        "[1mBearer authentication-required now".to_string(),
        "task-sk-ipped".to_string(),
        "npm_install_script_name".to_string(),
        "[Xnpm_aaaaaaaaaaaaaaaaaaaaaa]".to_string(),
        format!(
            "commit {sha}; abbrev {}; sk-{} npm_{}",
            &sha[..7],
            &sha[..7],
            &sha[..9]
        ),
        "[31mskipping 3 tests[0m; [1mnpm_config_cache=/tmp".to_string(),
        "[31mBearer token auth, as RFC 6750 says".to_string(),
        "%5B31mtask-sk-ipping the slow ones".to_string(),
        "see [the guide](https://example.com/sk-guide) and [npm_ok](docs/npm_ok.md)".to_string(),
        "[text](url) [a][b] arr[3] and [x]: https://example.com".to_string(),
    ] {
        assert_eq!(scrub(&text), text, "{text:?}");
    }
}

/// The body check reads each match once and matches never overlap, so the characters
/// it reads never exceed the text's length, whatever the input (counted, not timed).
#[test]
fn the_body_check_is_linear() {
    let n = 2_000;
    for unit in [
        format!("sk-{}", "a".repeat(30)),
        format!("npm_{}-", "b".repeat(25)),
        format!("Bearer {} ", "c".repeat(20)),
        "sk-sk-sk-npm_".to_string(),
        format!("bearer\t{}", "d".repeat(40)),
    ] {
        let text = unit.repeat(n);
        super::BODY_READ.with(|c| c.set(0));
        assert_eq!(scrub(&text), text, "no digit: nothing is redacted");
        let read = super::BODY_READ.with(|c| c.get());
        // `scrub` redacts twice; each pass reads at most the text once.
        assert!(read <= 2 * text.len() as u64, "{read} for {}", text.len());
        assert!(read > 0, "the check ran");
    }
}
