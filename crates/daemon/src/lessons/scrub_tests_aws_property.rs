//! Milestone 9.18 task 9's review, A-I1: an escape whose parameter or intermediate
//! bytes are a secret's characters. The strip eats the whole escape, so neither the
//! visible reading nor the one that keeps only the final byte is the secret. First the
//! review's table, with secrets that start with `7`, `12`, `/` and `+`; then a seeded
//! property over random secrets, every real escape form and every placement
//! ([`super::scrub_tests_gen`], fix round 2's m4), at four seeds, so the case-by-case
//! loop ends. Every key-shaped input is built at run time (ruling C9).

use super::scrub;
use super::scrub_tests_gen::{
    FORMS, PLACEMENTS, Rng, SEEDS, leaks, leaves_its_parameters, opens_a_string, per_seed, placed,
    without_hidden,
};
use super::scrub_tests_residuals::{aws_id, aws_secret, sts_id};

/// A piece of the secret this long that survives is a leak (the brief's bound).
const LEAK_RUN: usize = 30;

/// The longest piece of `secret` that `out` still holds.
pub(super) fn longest_leak(out: &str, secret: &str) -> usize {
    (1..=secret.len())
        .rev()
        .find(|&len| (0..=secret.len() - len).any(|at| out.contains(&secret[at..at + len])))
        .unwrap_or(0)
}

/// `secret` rotated left by `by` characters.
fn rotated(secret: &str, by: usize) -> String {
    [&secret[by..], &secret[..by]].concat()
}

/// `secret` with its first characters replaced by `head`.
pub(super) fn headed(secret: &str, head: &str) -> String {
    [head, &secret[head.len()..]].concat()
}

/// The review's A-I1 table: (input, secret).
fn table() -> Vec<(String, String)> {
    let (id, sts, s) = (aws_id(), sts_id(), aws_secret());
    // `7MDENG/…`, `/K7MDENG…`, `12alr…` and `+Jalr…`.
    let seven = rotated(&s, 15);
    let slash = rotated(&s, 13);
    let twelve = headed(&s, "12");
    let plus = headed(&s, "+");
    let (head, tail) = s.split_at(15);
    vec![
        (format!("{id} \u{1b}[{seven}"), seven.clone()),
        (format!("{id} \u{1b}{slash}"), slash.clone()),
        (format!("{id} \u{1b}[{twelve}"), twelve.clone()),
        (format!("{id} \u{1b}[{plus}"), plus.clone()),
        (format!("{id} \u{1b}({plus}"), plus.clone()),
        (format!("{id} \u{9b}{seven}"), seven.clone()),
        (format!("{id} \u{1b}[1;31{seven}"), seven.clone()),
        (format!("{id}\u{1b}[{seven}"), seven.clone()),
        (format!("{sts} \u{1b}[{twelve}"), twelve.clone()),
        (format!("secret_access_key=\u{1b}[{twelve}"), twelve.clone()),
        (format!("secret_access_key=\u{1b}{slash}"), slash.clone()),
        (
            format!(r#"{{"SecretAccessKey": "{}{seven}"}}"#, "\u{1b}["),
            seven.clone(),
        ),
        (
            format!(r#"{{"SecretAccessKey": "{}{plus}"}}"#, "\u{1b}("),
            plus.clone(),
        ),
        // Inside the secret: `wJalrXUtnFEMI` ESC `/K7MDENG…`.
        (format!("{id} {}\u{1b}{}", &s[..13], &s[13..]), s.clone()),
        (format!("{id} {head}\u{1b}[{tail}"), s.clone()),
    ]
}

#[test]
fn an_escape_whose_parameters_are_secret_characters_still_redacts_it() {
    for (input, secret) in table() {
        let out = scrub(&input);
        assert!(
            longest_leak(&out, &secret) < 8,
            "{input:?} -> {out:?} leaks {}",
            longest_leak(&out, &secret)
        );
        assert_eq!(scrub(&out), out, "idempotent on {input:?}");
    }
}

/// The secret's alphabet, built at run time.
fn alphabet() -> Vec<u8> {
    (b'A'..=b'Z')
        .chain(b'a'..=b'z')
        .chain(b'0'..=b'9')
        .chain(*b"/+")
        .collect()
}

/// Heads that make a text escape with the `\` of an ST or the `%` of an nF in front
/// (fix round 2's I1: an ESC or a CSI once ate the secret's next characters).
const HELD_HEADS: [&str; 12] = [
    "033", "0033", "233", "x1b", "x1B", "x9b", "u001b", "u009b", "1B", "1b", "9B", "9b",
];

/// A random 40-character secret holding a digit, an uppercase and a lowercase letter.
/// A quarter start with one to three of `0-9/+`, the characters an escape takes as
/// parameters or intermediates, and an eighth with one of [`HELD_HEADS`].
fn secret(rng: &mut Rng) -> String {
    let all = alphabet();
    let eaten: Vec<u8> = (b'0'..=b'9').chain(*b"/+").collect();
    loop {
        let head: String = match rng.below(8) {
            0 | 1 => (0..1 + rng.below(3)).map(|_| rng.pick(&eaten)).collect(),
            2 => (*rng.choose(&HELD_HEADS)).to_string(),
            _ => String::new(),
        };
        let tail: String = (head.len()..40).map(|_| rng.pick(&all)).collect();
        let s = head + &tail;
        let has = |f: fn(&u8) -> bool| s.as_bytes().iter().any(f);
        if has(u8::is_ascii_digit) && has(u8::is_ascii_uppercase) && has(u8::is_ascii_lowercase) {
            return s;
        }
    }
}

/// Where a secret goes: after an id, after the key name, or in JSON.
pub(super) fn place(context: usize, body: &str) -> String {
    match context {
        0 => format!("{} {body}", aws_id()),
        1 => format!("{} {body} end", sts_id()),
        2 => format!("{}{body}", aws_id()),
        3 => format!("{}={body}", "secret_access_key"),
        4 => format!("{} = {body}", "aws_secret_access_key"),
        _ => format!(r#"{{"{}": "{body}"}}"#, "SecretAccessKey"),
    }
}

pub(super) const CONTEXTS: usize = 6;

/// The property at `seed`: (cases run, cases excluded). Every secret runs in every
/// context, with every escape form in every placement ([`PLACEMENTS`]).
///
/// Excluded by name, for an escape inside the secret only (the same escape in front of
/// it or after it is run):
/// - A lone ESC before a `P` or `X` opens a DCS or SOS string, which the strip removes
///   to the line's end; the secret's head before it stays (task 9's review, A-M2, a
///   follow-up).
/// - A glued CSI with parameters of its own that does not parse (`ESC[5;` before
///   `/7…`). The strip removes its lead alone and leaves those parameters as text,
///   which splits the secret into two runs of under 40 that decision 6's one-run rule
///   cannot see through, so both halves stay: a leak of 30 or more characters when one
///   half is that long (task 9's review, m1, a follow-up).
fn property(seed: u64, secrets: usize) -> (usize, usize) {
    let mut rng = Rng(seed);
    let (mut cases, mut excluded) = (0, 0);
    for _ in 0..secrets {
        let s = secret(&mut rng);
        for context in 0..CONTEXTS {
            for form in FORMS {
                for placement in PLACEMENTS {
                    let (body, escapes) = placed(&mut rng, &s, form, placement);
                    if escapes.iter().any(|p| {
                        p.inside
                            && (opens_a_string(&p.escape, &p.rest)
                                || leaves_its_parameters(&p.escape, &p.rest))
                    }) {
                        excluded += 1;
                        continue;
                    }
                    check(seed, &place(context, &body), &s);
                    cases += 1;
                }
            }
        }
    }
    (cases, excluded)
}

/// No 30-character piece of `secret` is left once the line breakers drop the hidden
/// characters, and `scrub` is idempotent on its output.
fn check(seed: u64, input: &str, secret: &str) {
    let out = scrub(input);
    let shown = without_hidden(&out);
    assert!(
        !leaks(&shown, secret, LEAK_RUN),
        "seed {seed:#x}: {input:?} -> {out:?} leaks {}",
        longest_leak(&shown, secret)
    );
    assert_eq!(scrub(&out), out, "seed {seed:#x}: idempotent on {input:?}");
}

/// Secrets each seed draws; each runs in every context, form and placement.
pub(super) const SECRETS: usize = 150;

#[test]
fn no_escape_around_or_inside_an_aws_secret_leaves_thirty_of_its_characters() {
    for (seed, (cases, excluded)) in SEEDS.iter().zip(per_seed(|seed| property(seed, SECRETS))) {
        assert!(cases >= 10_000, "seed {seed:#x}: {cases} cases");
        // A lone ESC before `P` or `X`, or a glued CSI before `/` or `+` then a digit,
        // inside the secret is rare.
        assert!(
            excluded < cases / 50,
            "seed {seed:#x}: {excluded} of {cases} excluded"
        );
    }
}
