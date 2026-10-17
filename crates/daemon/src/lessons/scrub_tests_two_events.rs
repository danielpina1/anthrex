//! The round-2 re-review of fix round 2 for task 9's review, m1 and m2: a secret that
//! two separate escape events reach. The single-event properties
//! ([`super::scrub_tests_aws_property`], [`super::scrub_tests_credential_property`])
//! draw one escape form per case, so they never build an escape that eats or splits a
//! secret and then a second escape, of another form, that glues text or another
//! candidate to it. These draw two independent events per case: two forms at two
//! spots ([`SPOT_PAIRS`]: before and inside, inside and after, inside twice, before and
//! after), with text glued in front of a `Before` escape or after an `After` one. The
//! same seeds, exclusions and bounds as the single-event properties. Every key-shaped
//! input is built at run time (ruling C9).

use super::scrub_tests_aws_property::{CONTEXTS, alphabet, longest_leak, place, secret};
use super::scrub_tests_credential_property as cred;
use super::scrub_tests_gen::{
    FORMS, Form, Rng, SEEDS, SPOT_PAIRS, Spot, leaks, leaves_its_parameters, opens_a_string,
    per_seed, placed_two, without_hidden,
};
use super::{redact, scrub};

/// A piece of an AWS secret this long that survives is a leak (the brief's bound).
const AWS_LEAK_RUN: usize = 30;
/// A piece of a `Basic` or `Bearer` credential this long that reaches a lesson is one.
const CREDENTIAL_LEAK_RUN: usize = 8;

/// `n` characters drawn from `from`.
fn drawn(rng: &mut Rng, from: &[u8], n: usize) -> String {
    (0..n).map(|_| rng.pick(from)).collect()
}

/// Text glued in front of a `Before` escape: none, or a few of the secret's alphabet
/// (a written-out escape's `1m`, a word).
fn aws_head(rng: &mut Rng) -> String {
    match rng.below(3) {
        0 => String::new(),
        _ => {
            let n = 1 + rng.below(4);
            drawn(rng, &alphabet(), n)
        }
    }
}

/// Text glued after an `After` escape, and the second secret it holds, if any: none; a
/// few of the alphabet; a truncated CSI's digits (`38`); another secret; or a longer
/// mixed run with a digit (a path).
fn aws_tail(rng: &mut Rng) -> (String, Option<String>) {
    match rng.below(5) {
        0 => (String::new(), None),
        1 => {
            let n = 1 + rng.below(6);
            (drawn(rng, &alphabet(), n), None)
        }
        2 => {
            let n = 1 + rng.below(3);
            (drawn(rng, b"0123456789", n), None)
        }
        3 => {
            let other = secret(rng);
            (other.clone(), Some(other))
        }
        _ => {
            let n = 41 + rng.below(20);
            let run = drawn(rng, &alphabet(), n - 1);
            (format!("{run}7"), None)
        }
    }
}

/// Every pair of forms, at every pair of spots.
fn event_pairs() -> Vec<[(Form, Spot); 2]> {
    let mut pairs = Vec::new();
    for spots in SPOT_PAIRS {
        for a in FORMS {
            for b in FORMS {
                pairs.push([(a, spots[0]), (b, spots[1])]);
            }
        }
    }
    pairs
}

/// The AWS property at `seed`: (cases run, cases excluded). The exclusions are the
/// single-event property's, for an escape inside the secret.
fn aws_property(seed: u64, secrets: usize) -> (usize, usize) {
    let mut rng = Rng(seed);
    let (mut cases, mut excluded) = (0, 0);
    let pairs = event_pairs();
    for _ in 0..secrets {
        let s = secret(&mut rng);
        for context in 0..CONTEXTS {
            for &events in &pairs {
                let head = aws_head(&mut rng);
                let (tail, other) = aws_tail(&mut rng);
                let (body, escapes) = placed_two(&mut rng, &s, events, &head, &tail);
                if escapes.iter().any(|p| {
                    opens_a_string(&p.escape, &p.rest)
                        || (p.inside && leaves_its_parameters(&p.escape, &p.rest))
                }) {
                    excluded += 1;
                    continue;
                }
                let input = place(context, &body);
                let out = scrub(&input);
                let shown = without_hidden(&out);
                let tail_glued = matches!(events[1].1, Spot::After);
                for secret in std::iter::once(&s).chain(other.iter().filter(|_| tail_glued)) {
                    assert!(
                        !leaks(&shown, secret, AWS_LEAK_RUN),
                        "seed {seed:#x}: {input:?} -> {out:?} leaks {} of {secret:?}",
                        longest_leak(&shown, secret)
                    );
                }
                assert_eq!(scrub(&out), out, "seed {seed:#x}: idempotent on {input:?}");
                cases += 1;
            }
        }
    }
    (cases, excluded)
}

/// Secrets each seed draws; each runs in every context and every pair of events.
const SECRETS: usize = 20;

#[test]
fn no_two_escape_events_around_or_inside_an_aws_secret_leave_thirty_of_its_characters() {
    let runs = per_seed(|seed| aws_property(seed, SECRETS));
    for (seed, (cases, excluded)) in SEEDS.iter().zip(runs) {
        assert!(cases >= 30_000, "seed {seed:#x}: {cases} cases");
        assert!(
            excluded < cases / 25,
            "seed {seed:#x}: {excluded} of {cases} excluded"
        );
    }
}

/// The credential property at `seed` for `scheme`: (cases run, cases excluded). The
/// exclusions are the single-event property's, for every escape. Text glued after an
/// `After` escape is none or a few word characters.
fn credential_property(seed: u64, scheme: &str, credentials: usize) -> (usize, usize) {
    let mut rng = Rng(seed);
    let (mut cases, mut excluded) = (0, 0);
    let pairs = event_pairs();
    let word: Vec<u8> = (b'a'..=b'z').chain(b'0'..=b'9').collect();
    for _ in 0..credentials {
        let credential = if scheme == "Basic" {
            cred::basic_credential(&mut rng)
        } else {
            cred::bearer_credential(&mut rng)
        };
        for context in 0..cred::CONTEXTS {
            for &events in &pairs {
                let tail = if rng.below(2) == 0 {
                    String::new()
                } else {
                    let n = 1 + rng.below(6);
                    drawn(&mut rng, &word, n)
                };
                let (body, escapes) = placed_two(&mut rng, &credential, events, "", &tail);
                if escapes.iter().any(|p| {
                    opens_a_string(&p.escape, &p.rest) || leaves_its_parameters(&p.escape, &p.rest)
                }) {
                    excluded += 1;
                    continue;
                }
                let input = cred::place(scheme, context, &body);
                let out = scrub(&input);
                let lesson = redact(&without_hidden(&out));
                assert!(
                    !leaks(&lesson, &credential, CREDENTIAL_LEAK_RUN),
                    "seed {seed:#x}: {input:?} -> {lesson:?} leaks {credential:?}"
                );
                assert_eq!(scrub(&out), out, "seed {seed:#x}: idempotent on {input:?}");
                cases += 1;
            }
        }
    }
    (cases, excluded)
}

/// Credentials each seed draws per scheme.
const CREDENTIALS: usize = 12;

#[test]
fn no_two_escape_events_around_or_inside_a_basic_or_bearer_credential_leave_it() {
    for scheme in ["Basic", "Bearer"] {
        let runs = per_seed(|seed| credential_property(seed, scheme, CREDENTIALS));
        for (seed, (cases, excluded)) in SEEDS.iter().zip(runs) {
            assert!(cases >= 10_000, "{scheme} seed {seed:#x}: {cases} cases");
            assert!(
                excluded < cases / 12,
                "{scheme} seed {seed:#x}: {excluded} of {cases} excluded"
            );
        }
    }
}

/// `secret` rotated left by `by` characters.
fn rotated(secret: &str, by: usize) -> String {
    [&secret[by..], &secret[..by]].concat()
}

#[test]
fn the_re_reviews_two_event_inputs_leave_no_piece_of_either_secret() {
    use super::scrub_tests_residuals::{aws_id, aws_secret};
    let (id, s) = (aws_id(), aws_secret());
    let other = rotated(&s, 20);
    let (head, tail) = s.split_at(20);
    let path = ["src/Components/UserProfile/", "ProfileSettings2Form"].concat();
    let key = "secret_access_key";
    let inputs = [
        // m1: a second candidate in the same escape-joined run.
        format!("{id} \u{1b}[1;{s}\u{1b}[0m{other}"),
        format!("{id} \u{1b}{s}\u{1b}[0m{other}"),
        format!("{id} \u{1b}[1;{s}\u{1b}[0m/\u{1b}[1m{path}"),
        format!("{key}=\u{1b}[1;{s}\u{1b}[0m{other}"),
        format!(
            r#"{{"SecretAccessKey": "{}{s}{}{other}"}}"#,
            "\u{1b}[1;", "\u{1b}[0m"
        ),
        // m2: split by one escape, text glued on by another, under `Judge::Exact`.
        format!("{key}={head}\u{1b}\\{tail}\u{1b}[38;5;"),
        format!("{key}={head}\u{1b}[0m{tail}\u{1b}[38;5;"),
        format!("aws_{key} = {head}\u{1b}7{tail}\u{1b}[1"),
    ];
    for input in inputs {
        let out = scrub(&input);
        for secret in [&s, &other] {
            assert!(
                longest_leak(&out, secret) < 8,
                "{input:?} -> {out:?} leaks {} of {secret:?}",
                longest_leak(&out, secret)
            );
        }
        assert_eq!(scrub(&out), out, "idempotent on {input:?}");
    }
}
