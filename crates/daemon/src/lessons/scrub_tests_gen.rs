//! The seeded generator the redactor's property tests share (task 9's review, A-I1;
//! fix round 2 for that review, m4): splitmix64 from fixed seeds, no clock and no
//! crate; every real escape form; and every placement of escapes around and inside a
//! secret. Key-shaped strings are built at run time (ruling C9).

use proto::safe_text::is_hidden_format;

/// splitmix64: deterministic, no clock, no crate.
pub(super) struct Rng(pub(super) u64);

impl Rng {
    pub(super) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub(super) fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub(super) fn pick(&mut self, from: &[u8]) -> char {
        char::from(from[self.below(from.len())])
    }

    pub(super) fn choose<'a, T>(&mut self, from: &'a [T]) -> &'a T {
        &from[self.below(from.len())]
    }
}

/// The properties' seeds: the first round's, and three more, so a property's coverage
/// does not rest on one stream (fix round 2).
pub(super) const SEEDS: [u64; 4] = [
    0x5eed_0918_a11c_0de5,
    0x0918_f1c5_2b0b_7e3d,
    0xa11c_0de5_5eed_0918,
    0x2d3c_4b5a_6978_8796,
];

/// Runs `property` once per seed, each on its own thread, and returns its results in
/// the seeds' order. A panic names its seed in its message.
pub(super) fn per_seed<T: Send>(property: impl Fn(u64) -> T + Sync) -> Vec<T> {
    let property = &property;
    std::thread::scope(|scope| {
        let runs: Vec<_> = SEEDS
            .iter()
            .map(|&seed| scope.spawn(move || property(seed)))
            .collect();
        runs.into_iter()
            .map(|run| run.join().expect("a seed's property passes"))
            .collect()
    })
}

/// The real escape forms a property puts before, after or inside a secret.
#[derive(Clone, Copy, Debug)]
pub(super) enum Form {
    /// ESC with nothing after it: the secret's next character is its final (ECMA-48's
    /// general form), after any `/` or `+` it takes as intermediates.
    Lone,
    /// ESC Fe, complete: ESC and one of 0x40-0x5F that opens no string (`ESC \` is ST).
    Fe,
    /// ESC Fp (0x30-0x3F: `ESC 7`, `ESC =`) or Fs (0x60-0x7E: `ESC c`), complete.
    FpFs,
    /// nF: ESC, intermediates (a quarter ending in `%`), and a final or (glued) none.
    Nf,
    /// A CSI (7-bit or 8-bit) with parameters, and intermediates and a final, or
    /// (glued) neither, so the secret supplies them.
    Csi,
    /// An OSC ended by BEL.
    OscBel,
    /// An OSC ended by ST.
    OscSt,
    /// A DCS, SOS, PM or APC string ended by ST.
    StringSt,
    /// A hidden format character (which the line breakers drop).
    Hidden,
}

pub(super) const FORMS: [Form; 9] = [
    Form::Lone,
    Form::Fe,
    Form::FpFs,
    Form::Nf,
    Form::Csi,
    Form::OscBel,
    Form::OscSt,
    Form::StringSt,
    Form::Hidden,
];

const INTERMEDIATES: &[u8] = b" !\"#$%&'()*+,-./";

pub(super) fn escape(rng: &mut Rng, form: Form) -> String {
    let printable: Vec<u8> = (b' '..=b'~').collect();
    let mut e = String::new();
    match form {
        Form::Lone => e.push('\u{1b}'),
        Form::Fe => {
            e.push('\u{1b}');
            e.push(rng.pick(b"@ABCDEFGHIJKLMNOQRSTUVWYZ\\"));
        }
        Form::FpFs => {
            e.push('\u{1b}');
            let finals: Vec<u8> = (b'0'..=b'?').chain(b'`'..=b'~').collect();
            e.push(rng.pick(&finals));
        }
        Form::Nf => {
            e.push('\u{1b}');
            for _ in 0..rng.below(2) {
                e.push(rng.pick(INTERMEDIATES));
            }
            // Fix round 2's I1: `ESC %` then `1B…` once read as a CSI.
            e.push(if rng.below(4) == 0 {
                '%'
            } else {
                rng.pick(INTERMEDIATES)
            });
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
                    e.push(rng.pick(INTERMEDIATES));
                }
                e.push(rng.pick(&(b'@'..=b'~').collect::<Vec<u8>>()));
            }
        }
        Form::OscBel | Form::OscSt | Form::StringSt => {
            e.push('\u{1b}');
            if matches!(form, Form::StringSt) {
                e.push(rng.pick(b"PX^_"));
            } else {
                e.push(']');
                e.push_str(rng.choose(&["0;", "2;", "8;;"]));
            }
            for _ in 0..rng.below(12) {
                e.push(rng.pick(&printable));
            }
            e.push_str(if matches!(form, Form::OscBel) {
                "\u{7}"
            } else {
                "\u{1b}\\"
            });
        }
        Form::Hidden => e.push(*rng.choose(&[
            '\u{200b}', '\u{200d}', '\u{2060}', '\u{feff}', '\u{ad}', '\u{180e}',
        ])),
    }
    e
}

/// Where a property's escapes go around and inside a secret.
#[derive(Clone, Copy, Debug)]
pub(super) enum Placement {
    Front,
    Inside,
    After,
    BothSides,
    TwoInside,
}

pub(super) const PLACEMENTS: [Placement; 5] = [
    Placement::Front,
    Placement::Inside,
    Placement::After,
    Placement::BothSides,
    Placement::TwoInside,
];

/// One escape a property placed: the escape, the body's text after it (a second
/// escape included), and whether it is inside the secret (for the named exclusions).
pub(super) struct Placed {
    pub(super) escape: String,
    pub(super) rest: String,
    pub(super) inside: bool,
}

/// `secret` with escapes of `form` placed as `placement` says, and each escape.
pub(super) fn placed(
    rng: &mut Rng,
    secret: &str,
    form: Form,
    placement: Placement,
) -> (String, Vec<Placed>) {
    let cut = |rng: &mut Rng| 1 + rng.below(secret.len() - 1);
    let cuts: Vec<usize> = match placement {
        Placement::Front => vec![0],
        Placement::Inside => vec![cut(rng)],
        Placement::After => vec![secret.len()],
        Placement::BothSides => vec![0, secret.len()],
        Placement::TwoInside => {
            let (a, b) = (cut(rng), cut(rng));
            vec![a.min(b), a.max(b)]
        }
    };
    let events: Vec<(usize, Form)> = cuts.into_iter().map(|at| (at, form)).collect();
    at_cuts(rng, secret, "", &events, "")
}

/// Where one escape event goes: in front of the secret, inside it, or after it.
#[derive(Clone, Copy, Debug)]
pub(super) enum Spot {
    Before,
    Inside,
    After,
}

/// The pairs of escape events the two-event properties draw: an escape that eats or
/// splits the secret, then a second one somewhere else (the round-2 re-review's m1
/// and m2, each two separate escape events).
pub(super) const SPOT_PAIRS: [[Spot; 2]; 4] = [
    [Spot::Before, Spot::Inside],
    [Spot::Inside, Spot::After],
    [Spot::Inside, Spot::Inside],
    [Spot::Before, Spot::After],
];

/// `secret` with two escape events, each of its own form at its own spot, `head` glued
/// in front of a `Before` escape and `tail` glued after an `After` one; and each
/// escape. A glue is left out when its spot is not drawn.
pub(super) fn placed_two(
    rng: &mut Rng,
    secret: &str,
    events: [(Form, Spot); 2],
    head: &str,
    tail: &str,
) -> (String, Vec<Placed>) {
    let mut cuts: Vec<(usize, Form)> = events
        .iter()
        .map(|&(form, spot)| {
            let at = match spot {
                Spot::Before => 0,
                Spot::Inside => 1 + rng.below(secret.len() - 1),
                Spot::After => secret.len(),
            };
            (at, form)
        })
        .collect();
    cuts.sort_by_key(|&(at, _)| at);
    let spot = |s: fn(&Spot) -> bool| events.iter().any(|(_, e)| s(e));
    let head = if spot(|e| matches!(e, Spot::Before)) {
        head
    } else {
        ""
    };
    let tail = if spot(|e| matches!(e, Spot::After)) {
        tail
    } else {
        ""
    };
    at_cuts(rng, secret, head, &cuts, tail)
}

/// `head`, then `secret` with an escape of its form drawn at each cut (in order), then
/// `tail`; and each escape, its `rest` the whole body after it.
fn at_cuts(
    rng: &mut Rng,
    secret: &str,
    head: &str,
    cuts: &[(usize, Form)],
    tail: &str,
) -> (String, Vec<Placed>) {
    let mut body = head.to_string();
    let mut inside = Vec::new();
    let mut last = 0;
    for &(at, form) in cuts {
        body.push_str(&secret[last..at]);
        let e = escape(rng, form);
        body.push_str(&e);
        let end = body.len();
        inside.push((e, end, 0 < at && at < secret.len()));
        last = at;
    }
    body.push_str(&secret[last..]);
    body.push_str(tail);
    let escapes = inside
        .into_iter()
        .map(|(escape, end, inside)| Placed {
            escape,
            rest: body[end..].to_string(),
            inside,
        })
        .collect();
    (body, escapes)
}

/// `out` without its hidden format characters, as the line breakers leave it.
pub(super) fn without_hidden(out: &str) -> String {
    out.chars().filter(|c| !is_hidden_format(*c)).collect()
}

/// Whether `out` holds a piece of `secret` at least `run` characters long.
pub(super) fn leaks(out: &str, secret: &str, run: usize) -> bool {
    secret.len() >= run && (0..=secret.len() - run).any(|at| out.contains(&secret[at..at + run]))
}

/// Whether the escape `escape` then `rest` opens a string (a lone ESC before `P`, `X`,
/// `]`, `^` or `_`: a DCS, SOS, OSC, PM or APC), which the strip removes up to the next
/// BEL or ESC (task 9's review, A-M2).
pub(super) fn opens_a_string(escape: &str, rest: &str) -> bool {
    escape == "\u{1b}" && rest.starts_with(['P', 'X', ']', '^', '_'])
}

/// Whether the glued CSI `escape`, with parameters of its own, fails to parse before
/// `rest`: after its parameters (and the digits it takes from `rest`) come
/// intermediates and then no final. The strip then removes the CSI's lead alone and
/// leaves its own parameters as text (task 9's review, m1, narrowed by fix round 2: a
/// CSI with no parameters of its own leaves nothing).
pub(super) fn leaves_its_parameters(escape: &str, rest: &str) -> bool {
    let glued = escape.starts_with("\u{1b}[") || escape.starts_with('\u{9b}');
    // `ESC [` and U+009B are two bytes each.
    let own = &escape.as_bytes()[2.min(escape.len())..];
    if !glued || own.is_empty() || own.last().is_some_and(|b| (0x40..=0x7e).contains(b)) {
        return false;
    }
    let mut bytes = rest
        .bytes()
        .skip_while(|b| (0x30..=0x3f).contains(b))
        .peekable();
    while bytes.next_if(|b| (0x20..=0x2f).contains(b)).is_some() {}
    !bytes.next().is_some_and(|b| (0x40..=0x7e).contains(&b))
}
