//! Task 11's review, I1 (the controller's ruling after M9.15.12): escape spellings the
//! decoder does not know still end in a colour-code-shaped tail (`[31m`, `%5B31m`), so
//! that tail is a left boundary for the ambiguous `sk-`, `npm_` and `Bearer` whatever
//! stands before it; the single-byte CSI (U+009B) is an ESC; percent encoding is
//! decoded until it stops changing (at most eight passes). Every fake secret is built
//! at run time.

use super::{BEARER, TOKEN, scrub};

/// I1's spellings, each before the three ambiguous prefixes.
const SPELLINGS: [&str; 17] = [
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
];

/// The ambiguous secrets: (the secret, a fragment that must not survive).
fn secrets() -> Vec<(String, String)> {
    let body = "FAKE0fake1".repeat(4);
    let credential = "zFAKEcred".repeat(4);
    vec![
        (format!("sk-{body}"), body.clone()),
        (format!("npm_{body}"), body),
        (format!("Bearer {credential}"), credential),
    ]
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
                let out = scrub(&input);
                assert!(
                    out.contains(TOKEN) || out.contains(BEARER),
                    "{input:?} -> {out:?}"
                );
                assert!(!out.contains(&fragment), "{input:?} -> {out:?}");
                assert!(!out.contains(['\u{1b}', '\u{9b}']), "{input:?} -> {out:?}");
                assert_eq!(scrub(&out), out, "{input:?}: idempotent");
            }
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

/// Pinning: a colour tail before ordinary prose, and markdown links, are not secrets.
#[test]
fn colour_tails_and_markdown_links_are_not_over_redacted() {
    for text in [
        "[31mskipping 3 tests[0m; [1mnpm_config_cache=/tmp",
        "[31mBearer token auth, as RFC 6750 says",
        "%5B31mtask-sk-ipping the slow ones",
        "see [the guide](https://example.com/sk-guide) and [npm_ok](docs/npm_ok.md)",
        "[text](url) [a][b] arr[3] and [x]: https://example.com",
    ] {
        assert_eq!(scrub(text), text, "{text:?}");
    }
}
