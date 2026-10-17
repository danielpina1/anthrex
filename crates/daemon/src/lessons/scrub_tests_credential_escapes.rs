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
        // Found by the property at ten times its size: the escape took the token's only
        // capital, so its visible characters are all lowercase.
        (
            format!(
                "Authorization: Bearer \u{1b}-{}",
                ["Er1-v4", "mor.flrhx"].concat()
            ),
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

#[test]
fn a_hidden_character_inside_a_credential_leaves_none_of_it() {
    // Found by the credential property (fix round 2): `scrub` redacted the head before
    // a ZWSP, and once the line breakers dropped it the password's base64 was left.
    let admin = basic_for(b"admin");
    let bearer = ["abcd1234", "EFGH5678", "ijkl"].concat();
    for hidden in ['\u{200b}', '\u{feff}', '\u{180e}', '\u{ad}'] {
        let input = format!(
            "Authorization: Basic {}{hidden}{}",
            &admin[..19],
            &admin[19..]
        );
        assert_eq!(
            scrub(&input),
            format!("Authorization: {BASIC}"),
            "{input:?}"
        );
        let input = format!(
            "Authorization: Bearer {}{hidden}{}",
            &bearer[..9],
            &bearer[9..]
        );
        assert_eq!(
            scrub(&input),
            format!("Authorization: {BEARER}"),
            "{input:?}"
        );
    }
}

#[test]
fn the_credentials_read_through_every_hidden_character() {
    let hidden = regex::Regex::new(&format!("^{}$", super::HIDDEN_FORMS)).expect("a pattern");
    let missed: Vec<char> = (0..=0x10_ffff)
        .filter_map(char::from_u32)
        .filter(|c| proto::safe_text::is_hidden_format(*c))
        .filter(|c| !hidden.is_match(c.encode_utf8(&mut [0; 4])))
        .collect();
    assert_eq!(missed, Vec::<char>::new());
}

#[test]
fn a_lone_esc_before_a_lowercase_p_or_x_is_no_string_in_the_credential_patterns() {
    // The two-event property at 10 × 16 seeds (scratch): `(?i)` on the whole `Bearer`
    // and `Basic` patterns folded into their escape forms, so `ESC p` and `ESC x` read
    // as a DCS or SOS to the next escape, and the capture ended inside a later OSC
    // (`bearer ESC pK4kx… OSC …BEL Oi` left 15 characters).
    let token = ["pK4kxXZb", "BbYZE2Oi"].concat();
    let osc = "\u{1b}]2;Ot%hG[K0j2*\u{7}";
    let input = format!("bearer \u{1b}{}{osc}{}", &token[..14], &token[14..]);
    assert_eq!(scrub(&input), BEARER, "{input:?}");
    // A `Basic` credential whose base64 starts with `x` (`Ąna:…`).
    let cred = basic_for("Ąna".as_bytes());
    assert!(cred.starts_with('x'), "{cred}");
    let input = format!("basic \u{1b}{}{osc}{}", &cred[..10], &cred[10..]);
    assert_eq!(scrub(&input), BASIC, "{input:?}");
}

#[test]
fn text_glued_after_a_credentials_colour_reset_goes_with_it() {
    // PR #80's review, item 4, a pin: the `Bearer` and `Basic` captures read through
    // escapes, so a word glued on after the reset that ends a coloured credential is
    // replaced with it. Stopping at the reset would reopen a leak: a reset inside the
    // credential (`<tok[..12]> ESC[0m <tok[12..]>`) is the same shape, and its rest
    // would stay. A padded `Basic` credential ends at its `=`, so only an unpadded one
    // takes the word.
    let token = ["abcd1234", "EFGH5678", "ijkl"].concat();
    let reset = "\u{1b}[0m";
    for (input, want) in [
        (
            format!("Bearer \u{1b}[1m{token}{reset}done"),
            BEARER.to_string(),
        ),
        (
            format!("Bearer {}{reset}{}", &token[..12], &token[12..]),
            BEARER.to_string(),
        ),
        (
            format!("Basic \u{1b}[1m{}{reset}done", basic_for(b"admin1")),
            BASIC.to_string(),
        ),
    ] {
        assert_eq!(scrub(&input), want, "{input:?}");
    }
}
