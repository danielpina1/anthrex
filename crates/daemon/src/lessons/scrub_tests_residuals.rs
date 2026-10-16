//! Milestone 9.18 decision 6, task M9.18.4: the redaction residuals the 9.15 review
//! found (Stripe keys, AWS secret access keys, `Basic` credentials, a URL's user
//! information spelled JSON-escaped or percent-encoded). Every key-shaped input is
//! built at run time (ruling C9): GitHub push protection refuses such literals.

use super::{BASIC, TOKEN, redact, scrub};

/// `redact` and `scrub` of `input`, each with its name.
fn both(input: &str) -> [(&'static str, String); 2] {
    [("redact", redact(input)), ("scrub", scrub(input))]
}

fn assert_becomes(input: &str, want: &str) {
    for (how, out) in both(input) {
        assert_eq!(out, want, "{how} {input:?}");
    }
}

fn assert_left(input: &str) {
    assert_becomes(input, input);
}

/// A 24-character mixed-case Stripe body.
fn stripe_body() -> String {
    ["Ab3dEf", "Gh5jKl", "Mn7pQr", "St9vWx"].concat()
}

fn stripe(kind: &str) -> String {
    format!("sk_{}_{}", kind, stripe_body())
}

/// AWS's published example key id, built in pieces.
fn aws_id() -> String {
    ["AKIA", "IOSFODNN7", "EXAMPLE"].concat()
}

/// AWS's published example secret access key (40 characters), built in pieces.
fn aws_secret() -> String {
    let secret = ["wJalrXUtnFEMI", "/K7MDENG/", "bPxRfiCYEXAMPLEKEY"].concat();
    assert_eq!(secret.len(), 40);
    secret
}

/// `user:pass` in base64, built in pieces.
fn basic_credential() -> String {
    ["dXNl", "cjpw", "YXNz"].concat()
}

/// Every input the tests below redact or leave.
fn inputs() -> Vec<String> {
    let (id, secret, cred) = (aws_id(), aws_secret(), basic_credential());
    vec![
        format!("key {} end", stripe("live")),
        format!("key {} end", stripe("test")),
        format!("{}x", "sk_live_"),
        "task_sk_live_notes".to_string(),
        format!("aws_secret_access_key = {secret}"),
        format!(r#"{{"SecretAccessKey": "{secret}"}}"#),
        format!(r#"AWS_SECRET_ACCESS_KEY="{secret}""#),
        format!("id {id} secret {secret}"),
        format!("{secret},{id}"),
        format!("hash {secret}"),
        format!("id {id}\nhash {secret}"),
        format!("Authorization: Basic {cred}"),
        format!("basic {cred}"),
        format!("\u{1b}Basic {cred}"),
        "Basic usage of the API".to_string(),
        "Basic aGVsbG8gd29ybGQ=".to_string(),
        r"https:\/\/user:pass@host\/x".to_string(),
        "https%3A%2F%2Fuser%3Atok%40host".to_string(),
        "https%3a%2f%2fuser%3atok%40host".to_string(),
        "https%3A%2F%2Fhost%2Fpath%40x".to_string(),
    ]
}

#[test]
fn a_stripe_secret_key_is_redacted() {
    for kind in ["live", "test"] {
        assert_becomes(
            &format!("key {} end", stripe(kind)),
            &format!("key {TOKEN} end"),
        );
    }
    // A body under four characters, and an all-lowercase one, are words.
    assert_left(&format!("{}x", "sk_live_"));
    assert_left("task_sk_live_notes");
}

#[test]
fn an_aws_secret_after_its_key_name_is_redacted() {
    let secret = aws_secret();
    for (name_and_separator, end) in [
        ("aws_secret_access_key = ", ""),
        (r#"{"SecretAccessKey": ""#, r#""}"#),
        (r#"AWS_SECRET_ACCESS_KEY=""#, r#"""#),
    ] {
        assert_becomes(
            &format!("{name_and_separator}{secret}{end}"),
            &format!("{name_and_separator}{TOKEN}{end}"),
        );
    }
    // A 40-character hash after the name holds no uppercase letter: left.
    assert_left(&format!(
        "aws_secret_access_key = {}",
        "0a1b2c3d4e".repeat(4)
    ));
    // One character more is not a secret access key.
    assert_left(&format!("aws_secret_access_key = {secret}Z"));
}

#[test]
fn an_aws_secret_beside_its_key_id_is_redacted() {
    let (id, secret) = (aws_id(), aws_secret());
    assert_becomes(
        &format!("id {id} secret {secret}"),
        &format!("id {TOKEN} secret {TOKEN}"),
    );
    assert_becomes(&format!("{secret},{id}"), &format!("{TOKEN},{TOKEN}"));
    // A line with neither the id nor the key name keeps its 40 characters.
    assert_left(&format!("hash {secret}"));
    // The id on another line does not reach it.
    assert_becomes(
        &format!("id {id}\nhash {secret}"),
        &format!("id {TOKEN}\nhash {secret}"),
    );
    // A 40-character sha beside the id stays.
    let sha = "0a1b2c3d4e".repeat(4);
    assert_becomes(&format!("{id} {sha}"), &format!("{TOKEN} {sha}"));
}

#[test]
fn basic_credentials_are_redacted() {
    let cred = basic_credential();
    assert_becomes(
        &format!("Authorization: Basic {cred}"),
        &format!("Authorization: {BASIC}"),
    );
    assert_becomes(&format!("basic {cred}"), BASIC);
    assert_left("Basic usage of the API");
    // `hello world`: no colon once decoded.
    assert_left("Basic aGVsbG8gd29ybGQ=");
}

#[test]
fn an_escape_before_the_basic_marker_leaves_it_whole() {
    // Ruling C8: ECMA-48's general form would take the marker's `B` as its final.
    let input = format!("\u{1b}Basic {}", basic_credential());
    assert_eq!(scrub(&input), BASIC, "{input:?}");
}

#[test]
fn an_escaped_url_credential_is_redacted() {
    assert_becomes(r"https:\/\/user:pass@host\/x", r"https:\/\/***@host\/x");
    assert_becomes(
        "https%3A%2F%2Fuser%3Atok%40host",
        "https%3A%2F%2F***%40host",
    );
    assert_becomes(
        "https%3a%2f%2fuser%3atok%40host",
        "https%3a%2f%2f***%40host",
    );
    // A `%40` in the path, after the authority's encoded `/`, is not user information.
    assert_left("https%3A%2F%2Fhost%2Fpath%40x");
}

#[test]
fn scrub_stays_idempotent_and_linear_on_the_new_rules() {
    for input in inputs() {
        let once = scrub(&input);
        assert_eq!(scrub(&once), once, "{input:?}");
    }
    // Ruling C6: the bytes the new checks read, not a clock.
    let mixed = ["aB3", "cD4"].concat();
    // The brief's line: `Basic ` and `sk_live_` with no body, so nothing is read.
    let bare = format!("{} {} ", "Basic", "sk_live_");
    for (unit, reads) in [
        (bare, false),
        (format!("{} {} ", "Basic", "A".repeat(12)), true),
        (format!("{} {} ", aws_id(), "0a1b2c3d4e".repeat(4)), true),
        (
            format!("{}={} ", "secret_access_key", mixed.repeat(7)),
            true,
        ),
        (
            format!("{}{} ", "https%3A%2F%2F", "user%3A".repeat(3)),
            true,
        ),
    ] {
        for size in [16 * 1024, 64 * 1024] {
            let text = unit.repeat(size / unit.len());
            super::BODY_READ.with(|c| c.set(0));
            let out = scrub(&text);
            let read = super::BODY_READ.with(|c| c.get());
            assert!(
                read <= 2 * text.len() as u64,
                "{read} for {} of {unit:?}",
                text.len()
            );
            assert_eq!(read > 0, reads, "{read} for {unit:?}");
            assert_eq!(scrub(&out), out, "{unit:?}");
        }
    }
}
