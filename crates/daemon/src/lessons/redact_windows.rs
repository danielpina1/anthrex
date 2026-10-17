//! The round-2 re-review of fix round 2 for task 9's review, m1 and m2: an AWS run's
//! windows. A secret is some contiguous stretch of a run's parts ([`super::runs`]),
//! not always the whole run or one visible piece: another candidate can share an
//! escape-joined run (`<id> ESC[1;<secret>ESC[0m<other>`, m1), and an escape can split
//! the secret while another glues text after it (`<s[..20]>ESC[0m<s[20..]>ESC[38;5;`,
//! m2). So every window of a run is judged, not the run and its pieces alone. A window
//! starts and ends where a part does, in one of the run's three readings (visible,
//! kept, all); [`super::more`]'s `Judge::Tail` lets it start anywhere, and
//! `Judge::Ends` end anywhere (a glued head or tail). Then the windows of whole visible
//! pieces whose escapes could hold what the visible ones lack are restored. Each
//! window's text is replaced; overlapping ones are merged. Pure and linear: each
//! reading is walked once per end and once per start, and the restored windows end
//! after at most [`AWS_SECRET_LEN`] pieces.

use super::more::AWS_SECRET_LEN;
use super::runs::{Part, Run};

/// The fewest visible characters a window needs for its kept or all reading, or its
/// restoration, to be judged: fix round 2 for task 9's review, m3. A window with fewer
/// leaves no piece of a secret this long (milestone 9.18's leak bound, the property
/// tests' 30), so judging it can only redact text: a colour code's own parameters count
/// toward 40 (`ls --color` paths on an id's line). The whole run's exact kept and all
/// readings are judged without it, as before.
pub const RESTORE_VISIBLE_MIN: usize = 30;

/// One reading of a run: its bytes, each part's span in them, which offsets a part
/// starts or ends at, and running counts of digits, uppercase and lowercase letters,
/// and visible bytes.
struct Reading {
    spans: Vec<(usize, usize)>,
    boundary: Vec<bool>,
    counts: Vec<[u32; 4]>,
}

impl Reading {
    fn new(run: &Run<'_>, of: impl Fn(&Part) -> Vec<u8>) -> Self {
        let mut r = Reading {
            spans: Vec::with_capacity(run.parts.len()),
            boundary: vec![true],
            counts: vec![[0; 4]],
        };
        for part in &run.parts {
            let from = r.counts.len() - 1;
            let visible = matches!(part, Part::Visible(..));
            for b in of(part) {
                let mut c = *r.counts.last().expect("counts start with one");
                c[0] += u32::from(b.is_ascii_digit());
                c[1] += u32::from(b.is_ascii_uppercase());
                c[2] += u32::from(b.is_ascii_lowercase());
                c[3] += u32::from(visible);
                r.counts.push(c);
                r.boundary.push(false);
            }
            let to = r.counts.len() - 1;
            r.boundary[to] = true;
            r.spans.push((from, to));
        }
        r
    }

    fn len(&self) -> usize {
        self.counts.len() - 1
    }

    /// The counts in `s..e`.
    fn count(&self, s: usize, e: usize) -> [u32; 4] {
        std::array::from_fn(|i| self.counts[e][i] - self.counts[s][i])
    }

    /// Whether `s..e` holds a digit, an uppercase and a lowercase letter.
    fn mixed(&self, s: usize, e: usize) -> bool {
        self.count(s, e)[..3].iter().all(|&n| n > 0)
    }

    /// The text range of the reading's `s..e`: every part it touches, whole (a glued
    /// head or tail goes with the secret, as a whole piece did), and the escapes before
    /// it that add nothing to this reading (as the whole run took its leading ones).
    fn text_range(&self, run: &Run<'_>, s: usize, e: usize) -> (usize, usize) {
        let bounds = |p: usize| match run.parts[p] {
            Part::Visible(a, b) | Part::Invisible(a, b) => (a, b),
        };
        let mut p = self.spans.partition_point(|sp| sp.1 <= s);
        while p > 0 && self.spans[p - 1].0 == self.spans[p - 1].1 {
            p -= 1;
        }
        let q = self.spans.partition_point(|sp| sp.0 < e) - 1;
        (bounds(p).0, bounds(q).1)
    }
}

/// The text ranges of `run` that are secrets, merged and in order. `tail` lets a
/// window start inside a piece, `head` end inside one.
pub(super) fn secret_ranges(run: &Run<'_>, tail: bool, head: bool) -> Vec<(usize, usize)> {
    let readings = [
        Reading::new(run, |p| match *p {
            Part::Visible(a, b) => run.piece(a, b).as_bytes().to_vec(),
            Part::Invisible(..) => Vec::new(),
        }),
        Reading::new(run, |p| match *p {
            Part::Visible(a, b) => run.piece(a, b).as_bytes().to_vec(),
            Part::Invisible(a, b) => run.kept_of(a, b).into_iter().collect(),
        }),
        Reading::new(run, |p| match *p {
            Part::Visible(a, b) => run.piece(a, b).as_bytes().to_vec(),
            Part::Invisible(a, b) => run.eaten_of(a, b).collect(),
        }),
    ];
    let mut ranges = Vec::new();
    for (i, r) in readings.iter().enumerate() {
        let n = r.len();
        // The visible reading is judged as it is; the kept and all readings only with
        // enough visible characters, or as the whole run.
        let judged = |s: usize, e: usize| {
            r.mixed(s, e)
                && (i == 0
                    || r.count(s, e)[3] as usize >= RESTORE_VISIBLE_MIN
                    || (s == 0 && e == n))
        };
        for e in AWS_SECRET_LEN..=n {
            let s = e - AWS_SECRET_LEN;
            let ends = r.boundary[e] && (tail || r.boundary[s]);
            let starts = head && r.boundary[s];
            if (ends || starts) && judged(s, e) {
                ranges.push(r.text_range(run, s, e));
            }
        }
    }
    restored(run, &readings, &mut ranges);
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (a, b) in ranges {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    merged
}

/// Task 9's review, A-I1, per window: the whole visible pieces `a..=b`, with
/// [`RESTORE_VISIBLE_MIN`] to [`AWS_SECRET_LEN`] visible characters, and every escape
/// beside them up to the next piece on each side, are a secret when the bytes of the
/// alphabet those escapes ate reach [`AWS_SECRET_LEN`] and the whole holds a digit, an
/// uppercase and a lowercase letter. An escape's own parameters can sit before the
/// secret's characters it took (`ESC[5;7` then `M…`), so which of its bytes were the
/// secret's is not known, only how many could be.
fn restored(run: &Run<'_>, readings: &[Reading; 3], ranges: &mut Vec<(usize, usize)>) {
    let (visible, all) = (&readings[0], &readings[2]);
    let pieces: Vec<usize> = (0..run.parts.len())
        .filter(|&p| matches!(run.parts[p], Part::Visible(..)))
        .collect();
    for (bi, &b) in pieces.iter().enumerate() {
        let last = pieces
            .get(bi + 1)
            .map_or(run.parts.len() - 1, |&next| next - 1);
        for ai in (0..=bi).rev() {
            let a = pieces[ai];
            let shown = visible.spans[b].1 - visible.spans[a].0;
            if shown >= AWS_SECRET_LEN {
                break;
            }
            if shown < RESTORE_VISIBLE_MIN {
                continue;
            }
            let first = if ai == 0 { 0 } else { pieces[ai - 1] + 1 };
            let (s, e) = (all.spans[first].0, all.spans[last].1);
            if e - s >= AWS_SECRET_LEN && all.mixed(s, e) {
                ranges.push(all.text_range(run, s, e));
            }
        }
    }
}
