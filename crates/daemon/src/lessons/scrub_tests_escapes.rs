//! Milestone 9.18 task 6's review (part A): an escape whose final byte is one of a
//! secret's characters (I1, M1), a written-out escape glued before an AWS secret (I2),
//! a `Basic` credential that is not UTF-8 (M2), and an STS key id in its full shape
//! (concern 2). Every key-shaped input is built at run time (ruling C9).

use super::scrub_tests_residuals::{aws_id, aws_secret, base64, sts_id};
use super::{BASIC, BEARER, TOKEN, redact, scrub};

/// The longest piece of `secret` that `out` still holds, if it is 8 characters or more.
fn leak<'a>(out: &str, secret: &'a str) -> Option<&'a str> {
    (8..=secret.len())
        .rev()
        .flat_map(|len| (0..=secret.len() - len).map(move |at| &secret[at..at + len]))
        .find(|piece| out.contains(piece))
}

fn assert_no_leak(input: &str, secret: &str) -> String {
    let out = scrub(input);
    assert_eq!(leak(&out, secret), None, "{input:?} -> {out:?}");
    assert_eq!(scrub(&out), out, "idempotent on {input:?}");
    out
}

/// The review's I1 inputs: an escape's final byte is the secret's first character, or
/// one inside it, or one of the id's.
fn final_byte_inputs() -> Vec<String> {
    let (id, secret) = (aws_id(), aws_secret());
    let (head, tail) = secret.split_at(20);
    vec![
        format!("{id} \u{1b}{secret}"),
        format!("{id} \u{1b}[{secret}"),
        format!(r"{id} \u001b{secret}"),
        format!("{id} {head}\u{1b}{tail}"),
        format!("AKIA\u{1b}{} {secret}", &id[4..]),
        format!("AKIA\u{1b}[{} {secret}", &id[4..]),
        format!("secret_access_key=\u{1b}{secret}"),
        format!("{id}\u{1b}{secret}"),
    ]
}

#[test]
fn an_escape_whose_final_is_a_secret_character_still_redacts_it() {
    let secret = aws_secret();
    for input in final_byte_inputs() {
        assert_no_leak(&input, &secret);
    }
    let got: Vec<String> = final_byte_inputs()[..4].iter().map(|s| scrub(s)).collect();
    assert_eq!(got, vec![format!("{TOKEN} {TOKEN}"); 4]);
    assert_eq!(
        scrub(&final_byte_inputs()[6]),
        format!("secret_access_key={TOKEN}")
    );
}

/// The review's I2 spellings, which `decode` leaves as text.
fn written_out_escapes() -> [&'static str; 5] {
    ["^[[1m", r"\e[1m", r"\33[1m", "&#27;[1m", "%1b%5B1m"]
}

#[test]
fn a_written_out_escape_glued_before_an_aws_secret_still_redacts_it() {
    let (id, secret) = (aws_id(), aws_secret());
    for glue in written_out_escapes() {
        assert_no_leak(&format!("{id} {glue}{secret}"), &secret);
        assert_no_leak(&format!("{id} {glue}{secret}^[[0m"), &secret);
        assert_no_leak(&format!("secret_access_key={glue}{secret}"), &secret);
        assert_no_leak(
            &format!(r#"{{"SecretAccessKey": "{glue}{secret}"}}"#),
            &secret,
        );
    }
    assert_eq!(
        scrub(&format!("{id} ^[[1m{secret}^[[0m")),
        format!("{TOKEN} ^[[{TOKEN}^[[0m")
    );
    // Without the id or the key name, a 40-character run stays (best-effort).
    let plain = format!("hash ^[[1m{secret}");
    assert_eq!(scrub(&plain), plain);
    // The whole value right after the key name is still judged exactly: one character
    // more is not a secret access key.
    let longer = format!("aws_secret_access_key = {secret}Z");
    assert_eq!(scrub(&longer), longer);
}

/// `admin:hunter2hunter2` in base64.
fn basic_admin() -> String {
    base64(["admin:", "hunter2", "hunter2"].concat().as_bytes())
}

#[test]
fn an_escape_before_a_basic_or_bearer_credential_still_redacts_it() {
    // The review's M1: the escape takes the credential's first character as its final.
    let cred = basic_admin();
    assert_eq!(
        scrub(&format!("Authorization: Basic \u{1b}{cred}")),
        format!("Authorization: {BASIC}")
    );
    // An escape inside the credential, after its first 8 characters.
    let out = scrub(&format!("Basic {}\u{1b}{}", &cred[..8], &cred[8..]));
    assert_eq!(out, BASIC);
    // A 16-character bearer credential that would be 15 once the escape took its first.
    let bearer = ["abcd", "efgh", "1234", "wxyz"].concat();
    assert_eq!(
        scrub(&format!("Authorization: Bearer \u{1b}{bearer}")),
        format!("Authorization: {BEARER}")
    );
    assert_eq!(
        scrub(&format!("Bearer {}\u{1b}[0m{}", &bearer[..6], &bearer[6..])),
        BEARER
    );
    // Prose after the schemes is still prose.
    for prose in ["Basic usage of the API", "Bearer authentication-required"] {
        assert_eq!(scrub(prose), prose);
    }
}

#[test]
fn a_basic_credential_that_is_not_utf8_is_redacted() {
    // The review's M2: Latin-1 `admin:pässwörd`, RFC 7617's legacy charset.
    let latin1 = base64(b"admin:p\xe4ssw\xf6rd");
    assert_eq!(latin1, ["YWRt", "aW46", "cORz", "c3f2", "cmQ="].concat());
    for input in [
        format!("Basic {latin1}"),
        format!("Authorization: basic {latin1}"),
    ] {
        let want = input.replace(&input[input.len() - 6 - latin1.len()..], BASIC);
        assert_eq!(redact(&input), want, "{input:?}");
        assert_eq!(scrub(&input), want, "{input:?}");
    }
    // Bytes with no colon are not a credential, valid UTF-8 or not.
    let no_colon = base64(b"p\xe4ssw\xf6rd-only");
    let kept = format!("Basic {no_colon}");
    assert_eq!(scrub(&kept), kept);
    // Common prose after `Basic` stays.
    for word in [
        "configuration",
        "authentication",
        "functionality",
        "operations",
    ] {
        let prose = format!("Basic {word} is covered");
        assert_eq!(scrub(&prose), prose);
    }
}

#[test]
fn an_sts_key_id_in_its_full_shape_is_redacted() {
    // The review's concern 2: `ASIA` joins `AKIA`'s full shape only.
    let sts = sts_id();
    assert_eq!(scrub(&format!("id {sts} end")), format!("id {TOKEN} end"));
    // A short uppercase word with the prefix is not an id.
    for word in ["ASIAPAC2 region", "ASIAN markets", "ASIAPACIFIC1"] {
        assert_eq!(scrub(word), word);
    }
}

/// Every input above.
pub(super) fn escape_inputs() -> Vec<String> {
    let (id, secret) = (aws_id(), aws_secret());
    let cred = basic_admin();
    let mut inputs = final_byte_inputs();
    for glue in written_out_escapes() {
        inputs.push(format!("{id} {glue}{secret}"));
        inputs.push(format!("secret_access_key={glue}{secret}"));
    }
    inputs.push(format!("Authorization: Basic \u{1b}{cred}"));
    inputs.push(format!("Basic {}\u{1b}{}", &cred[..8], &cred[8..]));
    inputs.push(format!("Basic {}", base64(b"admin:p\xe4ssw\xf6rd")));
    inputs.push(format!("id {} end", sts_id()));
    inputs
}

#[test]
fn scrub_stays_idempotent_on_the_escape_inputs_and_their_fragments() {
    for input in escape_inputs() {
        let once = scrub(&input);
        assert_eq!(scrub(&once), once, "{input:?}");
    }
    // A fragment fuzz: random concatenations of the pieces the inputs are built from.
    let (id, secret) = (aws_id(), aws_secret());
    let fragments: Vec<String> = vec![
        id.clone(),
        sts_id(),
        "AKIA".into(),
        secret.clone(),
        secret[..20].into(),
        secret[20..].into(),
        "\u{1b}".into(),
        "\u{1b}[".into(),
        "\u{1b}[1m".into(),
        "\u{1b}]8;;x".into(),
        "\u{7}".into(),
        r"\u001b".into(),
        "^[[1m".into(),
        r"\e[".into(),
        "%1b%5B".into(),
        "1m".into(),
        " ".into(),
        "\n".into(),
        "=".into(),
        "secret_access_key=".into(),
        "Basic ".into(),
        "Bearer ".into(),
        basic_admin(),
        "[redacted token]".into(),
        "\u{200b}".into(),
        "w".into(),
        "Z".into(),
    ];
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize
    };
    for _ in 0..20_000 {
        let len = 1 + next() % 8;
        let input: String = (0..len)
            .map(|_| fragments[next() % fragments.len()].as_str())
            .collect();
        let once = scrub(&input);
        assert_eq!(scrub(&once), once, "{input:?}");
    }
}
