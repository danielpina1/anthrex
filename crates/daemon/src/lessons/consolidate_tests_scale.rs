//! Task M9.15.6's review, I1: the known-failure derivation is linear in the history,
//! with its results unchanged. The old derivation is kept as a test oracle
//! (`consolidate_oracle.rs`); seeded random histories must give the same lessons, and
//! the clean-run scans, counted, must grow linearly where the old ones grew as H·T.

use super::super::{Lesson, SCANS, oracle};
use super::*;

/// A seeded xorshift: the same histories on every run.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}

const OWNS: [&str; 7] = [
    "crates/a/**",
    "crates/a/src/x.rs",
    "crates/a/src/**",
    "crates/b/**",
    "crates/c/lib.rs",
    "docs/**",
    "crates/d/**",
];
const FPS: [&str; 4] = [FP, FP2, "00000000000000aa", "00000000000000bb"];
const HEADLINES: [&str; 3] = [HEADLINE, "test flaky_one failed", "panic in the parser"];
const OUTCOMES: [TaskOutcome; 6] = [
    TaskOutcome::Merged,
    TaskOutcome::MergedWithoutApproval,
    TaskOutcome::Blocked,
    TaskOutcome::Cancelled,
    TaskOutcome::Unfinished,
    TaskOutcome::Reported,
];

/// One random history with its store, `now` and `[lessons]` table: runs that repeat,
/// gaps from none to past expiry, every outcome, overlapping globs, repeated
/// signatures, flaky tests and dismissals.
fn case(seed: u64) -> (Vec<HistoryLine>, Store, u64, Lessons) {
    let mut rng = Rng::new(seed);
    let cfg = Lessons {
        known_min: 1 + rng.below(3) as u32,
        known_window_days: rng.pick(&[1, 3, 7, 30]),
        known_clean_runs: rng.below(4) as u32,
        expiry_days: rng.pick(&[2, 10, 60]),
        ..cfg()
    };
    let mut history = Vec::new();
    let (mut at, mut runs) = (T0, 1);
    for i in 0..rng.below(120) {
        at += match rng.below(10) {
            0 => 0,
            1..=5 => rng.below(6) * 3600,
            6..=8 => rng.below(4) * DAY,
            _ => rng.below(70) * DAY,
        };
        if rng.below(3) == 0 {
            runs += 1;
        }
        let run = rid(if rng.below(8) == 0 {
            1 + rng.below(runs) as u32
        } else {
            runs as u32
        });
        let mut t = task(&run, &format!("t{i}"), at, rng.pick(&OUTCOMES));
        t.gates.checks = rng.below(3) as u32;
        let owns: Vec<&str> = (0..1 + rng.below(3)).map(|_| rng.pick(&OWNS)).collect();
        t = owning(t, &owns);
        for _ in 0..rng.below(3) {
            t = failed(t, rng.pick(&FPS), rng.pick(&HEADLINES));
        }
        history.push(line(t));
        if rng.below(15) == 0 {
            history.push(flaky(&run, at, "flaky_one"));
        }
    }
    let mut store = Store::default();
    if rng.below(4) == 0 {
        store.dismissed.insert(rng.pick(&FPS).to_string());
    }
    let now = at + rng.pick(&[0, DAY, 30 * DAY, 100 * DAY]);
    (history, store, now, cfg)
}

#[test]
fn the_linear_derivation_matches_the_old_one() {
    let mut formed = 0;
    for seed in 0..600 {
        let (history, store, now, cfg) = case(seed);
        let got = known(&history, &store, now, &cfg);
        assert_eq!(
            got,
            oracle::known(&history, &store, now, &cfg),
            "seed {seed}"
        );
        formed += usize::from(!got.is_empty());
    }
    // Not vacuous: many of the histories have a live known failure.
    assert!(formed >= 100, "only {formed} cases formed one");
}

/// `n` runs, each with one task signed `FP` over `crates/a/**` and one merged, checked
/// task over `crates/b/**` an hour apart: one long life (default `[lessons]`), never
/// superseded, whose files never grow.
fn chronic(n: u32) -> Vec<HistoryLine> {
    let mut history = Vec::new();
    for i in 1..=n {
        let at = T0 + u64::from(i) * 3600;
        let hit = failed(task(&rid(i), "t1", at, TaskOutcome::Blocked), FP, HEADLINE);
        history.push(line(hit));
        let other = task(&rid(i), "t2", at + 1, TaskOutcome::Merged);
        history.push(line(owning(other, &["crates/b/**"])));
    }
    history
}

/// The clean-run candidates `derive` visited for `history`.
fn scans(
    history: &[HistoryLine],
    derive: fn(&[HistoryLine], &Store, u64, &Lessons) -> Vec<Lesson>,
) -> u64 {
    SCANS.with(|c| c.set(0));
    let now = T0 + 400 * 3600 * 2;
    let got = derive(history, &Store::default(), now, &cfg());
    assert_eq!(ids(&got), vec![kf(FP)]);
    SCANS.with(|c| c.get())
}

/// Doubling the history (its hits and its task records) at most about doubles the
/// scans; the old derivation's quadrupled, which this checks too, so the bound is one
/// the old code fails.
#[test]
fn clean_run_scans_grow_linearly() {
    let (small, large) = (chronic(200), chronic(400));
    let (a, b) = (scans(&small, known), scans(&large, known));
    assert!(a > 0, "the life was checked for clean runs");
    assert!(b * 10 <= a * 22, "scans {a} → {b}: more than linear");
    let (old_a, old_b) = (scans(&small, oracle::known), scans(&large, oracle::known));
    assert!(
        old_b * 10 > old_a * 35,
        "the oracle's scans {old_a} → {old_b} should grow as H·T"
    );
}
