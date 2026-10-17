//! Fix round 2 for task 9's review, I2: an escape before or inside a `Basic` or
//! `Bearer` credential whose parameters or intermediates are the credential's own
//! characters. The strip eats them with the escape, so neither the visible reading nor
//! the one that keeps only the final byte is the credential, and the password's base64
//! (or the bearer token) was left. Every key-shaped input is built at run time (ruling
//! C9).

use super::scrub_tests_residuals::base64;
use super::{BASIC, BEARER, scrub};

/// `<user>:hunter2hunter2` in base64.
fn basic_for(user: &[u8]) -> String {
    base64(&[user, b":", b"hunter2", b"hunter2"].concat())
}

/// The review's I2 table: (input, what `scrub` must make of it).
fn table() -> Vec<(String, String)> {
    let cyrillic = basic_for("Юрий".as_bytes());
    let latin1 = basic_for(b"\xe9lise");
    let admin = basic_for(b"admin");
    // `YWRtaW` `46` `aHVu…`: the CSI takes `46` as parameters and `a` as its final.
    assert_eq!(&admin[6..9], "46a");
    let bearer = ["42abcdefghij", "klmnopqrstuv", "wxyzABCDEF"].concat();
    let basic = format!("Authorization: {BASIC}");
    let bearer_out = format!("Authorization: {BEARER}");
    vec![
        (
            format!("Authorization: Basic \u{1b}[{cyrillic}"),
            basic.clone(),
        ),
        (
            format!("Authorization: Basic \u{9b}{cyrillic}"),
            basic.clone(),
        ),
        (
            format!("Authorization: Basic \u{1b}[{latin1}"),
            basic.clone(),
        ),
        (
            format!("Authorization: Basic {}\u{1b}[{}", &admin[..6], &admin[6..]),
            basic.clone(),
        ),
        (
            format!("Authorization: Basic {}\u{1b}[{}", &admin[..7], &admin[7..]),
            basic.clone(),
        ),
        // The escape's own parameters before the credential's characters it took.
        (
            format!("Authorization: Basic \u{1b}[1;3{cyrillic}"),
            basic.clone(),
        ),
        (
            format!(
                "Authorization: Basic {}\u{1b}[1;{}",
                &admin[..6],
                &admin[6..]
            ),
            basic.clone(),
        ),
        (
            format!("Authorization: Bearer \u{1b}[{bearer}"),
            bearer_out.clone(),
        ),
        (
            format!("Authorization: Bearer \u{9b}{bearer}"),
            bearer_out.clone(),
        ),
        (
            format!("Authorization: Bearer \u{1b}[1;31{bearer}"),
            bearer_out.clone(),
        ),
    ]
}

#[test]
fn an_escape_that_eats_a_credentials_characters_still_redacts_it() {
    for (input, want) in table() {
        let out = scrub(&input);
        assert_eq!(out, want, "{input:?}");
        assert_eq!(scrub(&out), out, "idempotent on {input:?}");
    }
}

#[test]
fn coloured_prose_after_the_schemes_stays() {
    for prose in [
        "Basic \u{1b}[1mconfiguration\u{1b}[0m is covered",
        "Basic \u{1b}[01;34mauthentication\u{1b}[0m",
        "Basic usage of the API",
        "Bearer authentication-required",
        // A colour code's own digit before a lowercase word is not a credential's.
        "Bearer \u{1b}[1mauthentication-required-here\u{1b}[0m",
    ] {
        assert_eq!(scrub(prose), scrub(&scrub(prose)), "{prose:?}");
        let stripped = prose.replace("\u{1b}[1m", "").replace("\u{1b}[01;34m", "");
        assert_eq!(scrub(prose), stripped.replace("\u{1b}[0m", ""), "{prose:?}");
    }
}
