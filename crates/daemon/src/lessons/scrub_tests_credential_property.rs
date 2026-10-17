//! Fix round 2 for task 9's review, I2: the AWS property's escape forms and placements
//! ([`super::scrub_tests_gen`]) around and inside `Basic` and `Bearer` credentials. A
//! `Basic` credential is a random `user:password` pair, its user name often not ASCII
//! (Cyrillic, CJK, Latin-1 bytes, UTF-8 accents), base64-encoded; a `Bearer` credential
//! is a random token. Each case asserts that no 8-character piece of the credential
//! reaches a lesson (`redact` after the line breakers drop the hidden characters, as
//! `dossier` and `distill` do) and that `scrub` is idempotent on its output. Every
//! key-shaped input is built at run time (ruling C9).

use super::scrub_tests_gen::{
    FORMS, PLACEMENTS, Rng, SEEDS, leaks, leaves_its_parameters, opens_a_string, per_seed, placed,
    without_hidden,
};
use super::scrub_tests_residuals::base64;
use super::{redact, scrub};

/// A piece of the credential this long that reaches a lesson is a leak.
const LEAK_RUN: usize = 8;

/// A user name: ASCII, Cyrillic, CJK, a Latin-1 byte (RFC 7617's legacy charset) or a
/// UTF-8 accent, one to six of them.
fn user(rng: &mut Rng) -> Vec<u8> {
    let units: [&[u8]; 10] = [
        b"a",
        b"Z",
        b"7",
        "Ю".as_bytes(),
        "р".as_bytes(),
        "山".as_bytes(),
        "田".as_bytes(),
        b"\xe9",
        "é".as_bytes(),
        "ß".as_bytes(),
    ];
    let mut name = Vec::new();
    for _ in 0..1 + rng.below(6) {
        let unit = *rng.choose(&units);
        if unit.len() == 1 && unit[0].is_ascii() {
            // Any ASCII letter or digit, not only the sample's.
            let ascii: Vec<u8> = (b'a'..=b'z')
                .chain(b'A'..=b'Z')
                .chain(b'0'..=b'9')
                .collect();
            name.push(rng.pick(&ascii) as u8);
        } else {
            name.extend_from_slice(unit);
        }
    }
    name
}

/// A password: 6 to 20 printable ASCII characters (a `:` among them), sometimes with a
/// non-ASCII one.
fn password(rng: &mut Rng) -> Vec<u8> {
    let printable: Vec<u8> = (b' '..=b'~').collect();
    let mut pass: Vec<u8> = (0..6 + rng.below(15))
        .map(|_| rng.pick(&printable) as u8)
        .collect();
    if rng.below(4) == 0 {
        pass.extend_from_slice("ü".as_bytes());
    }
    pass
}

pub(super) fn basic_credential(rng: &mut Rng) -> String {
    base64(&[user(rng), b":".to_vec(), password(rng)].concat())
}

/// A token of 16 to 40 bearer characters holding a digit, an uppercase and a
/// lowercase letter; a quarter start with one to three of the characters an escape
/// takes as parameters or intermediates.
pub(super) fn bearer_credential(rng: &mut Rng) -> String {
    let all: Vec<u8> = (b'A'..=b'Z')
        .chain(b'a'..=b'z')
        .chain(b'0'..=b'9')
        .chain(*b"._~+/-")
        .collect();
    let eaten: Vec<u8> = (b'0'..=b'9').chain(*b"+/-.").collect();
    loop {
        let lead = if rng.below(4) == 0 {
            1 + rng.below(3)
        } else {
            0
        };
        let len = 16 + rng.below(25);
        let s: String = (0..len)
            .map(|i| rng.pick(if i < lead { &eaten } else { &all }))
            .collect();
        let has = |f: fn(&u8) -> bool| s.as_bytes().iter().any(f);
        if has(u8::is_ascii_digit) && has(u8::is_ascii_uppercase) && has(u8::is_ascii_lowercase) {
            return s;
        }
    }
}

/// Where a credential goes, for `scheme` (`Basic` or `Bearer`).
pub(super) fn place(scheme: &str, context: usize, body: &str) -> String {
    match context {
        0 => format!("Authorization: {scheme} {body}"),
        1 => format!("{} {body}", scheme.to_lowercase()),
        2 => format!("curl -H 'Authorization: {scheme} {body}' https://example.test/"),
        _ => format!(r#"{{"Authorization": "{scheme} {body}"}}"#),
    }
}

pub(super) const CONTEXTS: usize = 4;

/// The property at `seed` for `scheme`: (cases run, cases excluded). The exclusions are
/// the AWS property's, a lone ESC that opens a string and a glued CSI with parameters
/// of its own that does not parse, in front of the credential too: the parameters the
/// strip leaves (`ESC[:` before `6.3f…` leaves `:6.3f…`) stand between the scheme and
/// the credential, and a `Basic` or `Bearer` credential has no line to be judged on
/// (an AWS secret has its id's line or its key name).
fn property(seed: u64, scheme: &str, credentials: usize) -> (usize, usize) {
    let mut rng = Rng(seed);
    let (mut cases, mut excluded) = (0, 0);
    for _ in 0..credentials {
        let cred = if scheme == "Basic" {
            basic_credential(&mut rng)
        } else {
            bearer_credential(&mut rng)
        };
        for context in 0..CONTEXTS {
            for form in FORMS {
                for placement in PLACEMENTS {
                    let (body, escapes) = placed(&mut rng, &cred, form, placement);
                    if escapes.iter().any(|p| {
                        opens_a_string(&p.escape, &p.rest)
                            || leaves_its_parameters(&p.escape, &p.rest)
                    }) {
                        excluded += 1;
                        continue;
                    }
                    let input = place(scheme, context, &body);
                    let out = scrub(&input);
                    let lesson = redact(&without_hidden(&out));
                    assert!(
                        !leaks(&lesson, &cred, LEAK_RUN),
                        "seed {seed:#x}: {input:?} -> {lesson:?} leaks {cred:?}"
                    );
                    assert_eq!(scrub(&out), out, "seed {seed:#x}: idempotent on {input:?}");
                    cases += 1;
                }
            }
        }
    }
    (cases, excluded)
}

/// Credentials each seed draws per scheme; each runs in every context, form and
/// placement.
const CREDENTIALS: usize = 80;

#[test]
fn no_escape_around_or_inside_a_basic_or_bearer_credential_leaves_it() {
    for scheme in ["Basic", "Bearer"] {
        let runs = per_seed(|seed| property(seed, scheme, CREDENTIALS));
        for (seed, (cases, excluded)) in SEEDS.iter().zip(runs) {
            assert!(cases >= 5_000, "{scheme} seed {seed:#x}: {cases} cases");
            assert!(
                excluded < cases / 25,
                "{scheme} seed {seed:#x}: {excluded} of {cases} excluded"
            );
        }
    }
}
