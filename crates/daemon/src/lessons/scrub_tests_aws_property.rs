//! Milestone 9.18 task 9's review, A-I1: an escape whose parameter or intermediate
//! bytes are a secret's characters. The strip eats the whole escape, so neither the
//! visible reading nor the one that keeps only the final byte is the secret. First the
//! review's table, with secrets that start with `7`, `12`, `/` and `+`; then a seeded
//! property over random secrets, every real escape form and the three placements, so
//! the case-by-case loop ends. Every key-shaped input is built at run time (ruling C9).

use super::scrub;
use super::scrub_tests_residuals::{aws_id, aws_secret, sts_id};

/// A piece of the secret this long that survives is a leak (the brief's bound).
const LEAK_RUN: usize = 30;

/// The longest piece of `secret` that `out` still holds.
fn longest_leak(out: &str, secret: &str) -> usize {
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
fn headed(secret: &str, head: &str) -> String {
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

/// splitmix64: deterministic, no clock, no crate.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick(&mut self, from: &[u8]) -> char {
        char::from(from[self.below(from.len())])
    }
}

/// The property's seed.
pub(super) const SEED: u64 = 0x5eed_0918_a11c_0de5;

/// The secret's alphabet, built at run time.
fn alphabet() -> Vec<u8> {
    (b'A'..=b'Z')
        .chain(b'a'..=b'z')
        .chain(b'0'..=b'9')
        .chain(*b"/+")
        .collect()
}

/// A random 40-character secret holding a digit, an uppercase and a lowercase letter.
/// A quarter start with one to three of `0-9/+`, the characters an escape takes as
/// parameters or intermediates.
fn secret(rng: &mut Rng) -> String {
    let all = alphabet();
    let eaten: Vec<u8> = (b'0'..=b'9').chain(*b"/+").collect();
    loop {
        let lead = if rng.below(4) == 0 {
            1 + rng.below(3)
        } else {
            0
        };
        let s: String = (0..40)
            .map(|i| rng.pick(if i < lead { &eaten } else { &all }))
            .collect();
        let has = |f: fn(&u8) -> bool| s.as_bytes().iter().any(f);
        if has(u8::is_ascii_digit) && has(u8::is_ascii_uppercase) && has(u8::is_ascii_lowercase) {
            return s;
        }
    }
}

/// The real escape forms the property puts before (or inside) a secret.
#[derive(Clone, Copy, Debug)]
enum Form {
    /// ESC with nothing after it: the secret's next character is its final (ECMA-48's
    /// general form), after any `/` or `+` it takes as intermediates.
    Lone,
    /// ESC Fe, complete: ESC and one of 0x40-0x5F that opens no string.
    Fe,
    /// nF: ESC, intermediates, and a final or (glued) none.
    Nf,
    /// A CSI (7-bit or 8-bit) with parameters, and intermediates and a final, or
    /// (glued) neither, so the secret supplies them.
    Csi,
    /// An OSC ended by BEL.
    OscBel,
    /// An OSC ended by ST.
    OscSt,
}

const FORMS: [Form; 6] = [
    Form::Lone,
    Form::Fe,
    Form::Nf,
    Form::Csi,
    Form::OscBel,
    Form::OscSt,
];

fn escape(rng: &mut Rng, form: Form) -> String {
    let printable: Vec<u8> = (b' '..=b'~').collect();
    let mut e = String::new();
    match form {
        Form::Lone => e.push('\u{1b}'),
        Form::Fe => {
            e.push('\u{1b}');
            e.push(rng.pick(b"@ABCDEFGHIJKLMNOQRSTUVWYZ\\"));
        }
        Form::Nf => {
            e.push('\u{1b}');
            for _ in 0..1 + rng.below(2) {
                e.push(rng.pick(b" !\"#$%&'()*+,-./"));
            }
            if rng.below(2) == 0 {
                e.push(rng.pick(&(b'0'..=b'~').collect::<Vec<u8>>()));
            }
        }
        Form::Csi => {
            e.push_str(if rng.below(4) == 0 {
                "\u{9b}"
            } else {
                "\u{1b}["
            });
            for _ in 0..rng.below(5) {
                e.push(rng.pick(b"0123456789;:<=>?"));
            }
            // Half the CSIs are glued: no intermediates and no final of their own, so
            // the secret's characters complete them.
            if rng.below(2) == 0 {
                for _ in 0..rng.below(3) {
                    e.push(rng.pick(b" !\"#$%&'()*+,-./"));
                }
                e.push(rng.pick(&(b'@'..=b'~').collect::<Vec<u8>>()));
            }
        }
        Form::OscBel | Form::OscSt => {
            e.push_str("\u{1b}]");
            e.push_str(["0;", "2;", "8;;"][rng.below(3)]);
            for _ in 0..rng.below(12) {
                e.push(rng.pick(&printable));
            }
            e.push_str(if matches!(form, Form::OscBel) {
                "\u{7}"
            } else {
                "\u{1b}\\"
            });
        }
    }
    e
}

/// Where a secret goes: after an id, after the key name, or in JSON.
fn place(context: usize, body: &str) -> String {
    match context {
        0 => format!("{} {body}", aws_id()),
        1 => format!("{} {body}", sts_id()),
        2 => format!("{}{body}", aws_id()),
        3 => format!("{}={body}", "secret_access_key"),
        _ => format!(r#"{{"{}": "{body}"}}"#, "SecretAccessKey"),
    }
}

const CONTEXTS: usize = 5;

/// Whether `escape` then `rest` opens a DCS or SOS string (ESC P, ESC X), which the
/// strip removes up to the next BEL or ESC.
fn opens_a_string(escape: &str, rest: &str) -> bool {
    escape == "\u{1b}" && (rest.starts_with('P') || rest.starts_with('X'))
}

/// Whether the glued CSI `escape` fails to parse before `rest`: after its parameters
/// (and the digits it takes from `rest`) come intermediates (`/`, `+`) and then no final
/// (a digit). The strip then removes the CSI's lead alone.
fn leaves_its_parameters(escape: &str, rest: &str) -> bool {
    let glued = escape.starts_with("\u{1b}[") || escape.starts_with('\u{9b}');
    // `ESC [` and U+009B are two bytes each.
    let own = &escape.as_bytes()[2.min(escape.len())..];
    if !glued || own.last().is_some_and(|b| (0x40..=0x7e).contains(b)) {
        return false;
    }
    let mut bytes = rest
        .bytes()
        .skip_while(|b| (0x30..=0x3f).contains(b))
        .peekable();
    while bytes.next_if(|b| (0x20..=0x2f).contains(b)).is_some() {}
    !bytes.next().is_some_and(|b| (0x40..=0x7e).contains(&b))
}

/// The property: (secrets, cases run, cases excluded).
fn property(secrets: usize) -> (usize, usize) {
    let mut rng = Rng(SEED);
    let (mut cases, mut excluded) = (0, 0);
    for _ in 0..secrets {
        let s = secret(&mut rng);
        for context in 0..CONTEXTS {
            for form in FORMS {
                // In front of the secret.
                let e = escape(&mut rng, form);
                check(&place(context, &format!("{e}{s}")), &s);
                cases += 1;
                // Inside it. Excluded by name (task 9's review, A-M2, a follow-up): a
                // lone ESC before a `P` or `X` inside the secret opens a DCS or SOS
                // string, which the strip removes to the line's end; the secret's head
                // before it stays, and nothing before the strip can tell it from text.
                //
                // Excluded by name, too: a glued CSI inside the secret that does not parse
                // (`ESC[5;` before `/7…`). The strip removes its lead alone and leaves its
                // own parameters (`5;`) as text, which splits the secret into two runs of
                // under 40; nothing before or after the strip can tell those parameters
                // from text, and decision 6's secret is one run of its alphabet. No
                // program writes half a CSI into a secret. In front of the secret the same
                // CSI leaves its parameters before it, and the secret is still caught.
                let at = 1 + rng.below(s.len() - 1);
                let e = escape(&mut rng, form);
                let input = place(context, &format!("{}{e}{}", &s[..at], &s[at..]));
                let after = &input[input.find(&e).expect("the escape") + e.len()..];
                if opens_a_string(&e, after) || leaves_its_parameters(&e, after) {
                    excluded += 1;
                    continue;
                }
                check(&input, &s);
                cases += 1;
            }
        }
    }
    (cases, excluded)
}

fn check(input: &str, secret: &str) {
    let out = scrub(input);
    let leak = longest_leak(&out, secret);
    assert!(
        leak < LEAK_RUN,
        "seed {SEED:#x}: {input:?} -> {out:?} leaks {leak}"
    );
    assert_eq!(scrub(&out), out, "seed {SEED:#x}: idempotent on {input:?}");
}

/// Secrets the property draws; each runs in every context and form, in front and inside.
pub(super) const SECRETS: usize = 400;

#[test]
fn no_escape_before_or_inside_an_aws_secret_leaves_thirty_of_its_characters() {
    let (cases, excluded) = property(SECRETS);
    assert!(cases >= 10_000, "{cases} cases");
    // A lone ESC before `P` or `X`, or a glued CSI before `/` or `+` then a digit, is
    // rare.
    assert!(excluded < cases / 50, "{excluded} of {cases} excluded");
}
