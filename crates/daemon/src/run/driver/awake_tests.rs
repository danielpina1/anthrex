//! `AwakeClock`: what a read reports, and the property every report keeps (review
//! round 2 of the clock-step fix): no wait ends before its true awake time, by more
//! than `READ_TOLERANCE`.

use super::*;

/// A year, the forward step's length below.
const X: u64 = 365 * 24 * 3_600;

/// The wall time `ms` milliseconds after the epoch.
fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

/// One read of `clock` at wall time `wall`, `awake` the awake instant.
fn read_at(clock: &AwakeClock, wall: Duration, awake: Instant) -> ClockStep {
    clock.read(wall, awake)
}

/// A simulated host: its true wall and awake clocks, and what the clock reported.
/// `stamps` are waits armed at earlier reads: the wall time and awake time they were
/// armed at, and the seconds reported by then.
struct Host {
    clock: AwakeClock,
    at: Instant,
    /// Nanoseconds since the epoch, and awake nanoseconds since `at`.
    wall: u128,
    awake: u128,
    reported: i128,
    stamps: Vec<(u128, u128, i128)>,
}

/// A wait's bound: a stall's `stall_after_secs`.
const BOUND: u64 = 300;

impl Host {
    fn new(wall_ms: u64) -> Self {
        let mut host = Host {
            clock: AwakeClock::default(),
            at: Instant::now(),
            wall: u128::from(wall_ms) * 1_000_000,
            awake: 0,
            reported: 0,
            stamps: Vec::new(),
        };
        host.read();
        host
    }

    /// `ns` nanoseconds pass awake; the wall clock runs `ppm` parts per million slower
    /// than the awake clock (the awake clock fast).
    fn pass(&mut self, ns: u128, ppm: u128) {
        self.awake += ns;
        self.wall += ns - ns * ppm / 1_000_000;
    }

    /// The host sleeps `ns`: the wall clock moves, the awake clock does not.
    fn sleep(&mut self, ns: u128) {
        self.wall += ns;
    }

    fn set(&mut self, ahead: bool, secs: u64) {
        let ns = u128::from(secs) * 1_000_000_000;
        self.wall = if ahead {
            self.wall + ns
        } else {
            self.wall - ns
        };
    }

    /// A read, as the driver's `handle` makes one, then the check of every armed wait:
    /// once the reducer's integer clock says its bound has passed (`not_before` from its
    /// truncated arming second, moved by the steps reported since), its awake time is at
    /// least the bound, less the tolerance.
    fn read(&mut self) -> ClockStep {
        let wall = Duration::from_nanos(u64::try_from(self.wall).unwrap());
        let awake = self.at + Duration::from_nanos(u64::try_from(self.awake).unwrap());
        let step = read_at(&self.clock, wall, awake);
        match step {
            ClockStep::Kept => {}
            ClockStep::Slept(s) => self.reported += i128::from(s),
            ClockStep::Back(s) => self.reported -= i128::from(s),
        }
        let now = self.wall / 1_000_000_000;
        let tolerance = READ_TOLERANCE.as_nanos();
        for &(wall_at, awake_at, reported_at) in &self.stamps {
            let shift = self.reported - reported_at;
            let due =
                i128::try_from(wall_at / 1_000_000_000).unwrap() + i128::from(BOUND) + 1 + shift;
            if i128::try_from(now).unwrap() >= due {
                let spent = self.awake - awake_at;
                assert!(
                    spent + tolerance >= u128::from(BOUND) * 1_000_000_000,
                    "a wait armed at {wall_at} ns ended after {spent} ns awake ({step:?})"
                );
            }
        }
        step
    }

    /// A wait armed now, at the read just made.
    fn arm(&mut self) {
        self.stamps.push((self.wall, self.awake, self.reported));
    }
}

/// The net shift (`Slept` less `Back`) that a forward step of `X` read `a1`
/// milliseconds awake after the anchor, and its correction read `a2` later, step: the
/// wall clock's true second is `100 + phase` tenths at the anchor.
fn net_shift(phase: u64, a1: u64, a2: u64) -> i64 {
    let clock = AwakeClock::default();
    let at = Instant::now();
    let wall = |m: u64| ms(100_000 + phase * 100 + m);
    assert_eq!(read_at(&clock, wall(0), at), ClockStep::Kept);
    let ahead = read_at(&clock, wall(a1) + Duration::from_secs(X), at + ms(a1));
    let back = read_at(&clock, wall(a1 + a2), at + ms(a1 + a2));
    let (ClockStep::Slept(f), ClockStep::Back(b)) = (ahead, back) else {
        panic!("{ahead:?} then {back:?} (phase {phase}, {a1} ms, {a2} ms)");
    };
    i64::try_from(f).unwrap() - i64::try_from(b).unwrap()
}

/// Review round 1 (M1): each read rounded on its own, so a step and its correction
/// read 1.5 s and then 0.3 s awake after the anchor came out as `Slept(X - 1)` and
/// `Back(X)`: the waits moved a second earlier than before, and a stall fired a second
/// short. They cancel.
#[test]
fn a_forward_step_and_its_correction_cancel_across_fractional_spans() {
    assert_eq!(net_shift(0, 1_500, 300), 0);
}

/// Whatever the wall clock's phase and the awake spans, the net shift of a forward
/// step and its correction is never negative, so no wait ends early, and under a
/// second.
#[test]
fn a_forward_step_and_its_correction_never_net_a_step_back() {
    for phase in 0..10 {
        for a1 in [0, 100, 300, 500, 900, 1_000, 1_500, 1_999, 2_700] {
            for a2 in [0, 100, 300, 500, 900, 1_000, 1_500, 1_999, 2_700] {
                let net = net_shift(phase, a1, a2);
                assert!((0..=1).contains(&net), "{net} (phase {phase}, {a1}, {a2})");
            }
        }
    }
}

/// The sleeps (ms), the awake time before the read that sees each (ms), and the
/// awake time (ms) that moves the later reads to another phase of the second.
fn pairs() -> Vec<(u64, u64, u64)> {
    let mut all = Vec::new();
    for sleep in [10_100, 10_500, 10_900, 3_000, 2_400] {
        for after in [100, 900, 1_000, 1_900] {
            for realign in [0, 100, 500, 900] {
                all.push((sleep, after, realign));
            }
        }
    }
    all
}

/// Review round 2 (N2): a step reported before the pair (a 10.1 s sleep read 0.9 s
/// later) carried its rounding into the pair, which then netted a second back, and a
/// wait armed between them ended a second short. Every wait armed at any read here
/// ends no earlier than its awake bound.
#[test]
fn a_step_reported_before_a_step_and_its_correction_ends_no_wait_early() {
    for phase in [0, 3, 7] {
        for (sleep, after, realign) in pairs() {
            let mut host = Host::new(100_000 + phase * 100);
            host.sleep(u128::from(sleep) * 1_000_000);
            host.pass(u128::from(after) * 1_000_000, 0);
            host.read();
            // The reads after it fall at another phase of the awake second.
            host.pass(u128::from(realign) * 1_000_000, 0);
            for _ in 0..3 {
                host.pass(1_000_000_000, 0);
                host.read();
                host.arm();
            }
            host.set(true, X);
            host.pass(1_000_000_000, 0);
            host.read();
            host.set(false, X);
            host.pass(1_000_000_000, 0);
            host.read();
            // Reads at 0.3 s, so one falls just past each wait's due second.
            for _ in 0..(BOUND * 10 / 3 + 20) {
                host.pass(300_000_000, 0);
                host.read();
            }
        }
    }
}

/// Review round 2: an awake clock fast by `ppm` against the wall (`CLOCK_UPTIME_RAW`
/// is not disciplined; the wall is) makes the offset fall slowly. Reported read to read
/// as steps back, that drift pulled the waits it spanned early; no wait armed at any
/// read here ends before its awake bound, over a long span and with reads at 1 s.
#[test]
fn a_fast_awake_clock_ends_no_wait_early() {
    for ppm in [50, 1_000, 10_000] {
        let mut host = Host::new(100_000);
        for i in 0..20_000_u64 {
            host.pass(1_000_000_000 + u128::from(i % 7) * 13_000_000, ppm);
            host.read();
            if i % 97 == 0 {
                host.arm();
            }
            // Keep the check bounded: drop waits long past their bound.
            let awake = host.awake;
            host.stamps
                .retain(|&(_, at, _)| awake - at < 2 * u128::from(BOUND) * 1_000_000_000);
        }
    }
}

/// An awake clock slow against the wall reports a rise of the offset as `Slept` at
/// once, so no wait ends before its awake bound either.
#[test]
fn a_slow_awake_clock_ends_no_wait_early() {
    let mut host = Host::new(100_000);
    for i in 0..5_000_u64 {
        host.awake += 1_000_000_000;
        host.wall += 1_000_000_000 + 1_000_000; // the wall gains 1,000 ppm
        host.read();
        if i % 31 == 0 {
            host.arm();
        }
        let awake = host.awake;
        host.stamps
            .retain(|&(_, at, _)| awake - at < 2 * u128::from(BOUND) * 1_000_000_000);
    }
}

/// One step: the anchor at wall second `was`, then a read at `wall`, `m` awake
/// milliseconds later.
fn once(was: u64, wall: u64, m: u64) -> ClockStep {
    let clock = AwakeClock::default();
    let at = Instant::now();
    assert_eq!(
        read_at(&clock, Duration::from_secs(was), at),
        ClockStep::Kept
    );
    read_at(&clock, Duration::from_secs(wall), at + ms(m))
}

#[test]
fn a_wall_jump_with_no_awake_time_is_a_sleep() {
    // Rounded up: 999.8 s.
    assert_eq!(once(0, 1_001, 1_200), ClockStep::Slept(1_000));
    assert_eq!(once(0, 1_000, 0), ClockStep::Slept(1_000));
}

#[test]
fn awake_time_and_its_tolerance_are_not_a_sleep() {
    assert_eq!(once(0, 1, 1_000), ClockStep::Kept);
    assert_eq!(once(0, 600, 600_000), ClockStep::Kept);
    assert_eq!(once(0, 0, 0), ClockStep::Kept);
    // A wall gaining more than the tolerance is reported, rounded up.
    assert_eq!(once(0, 1, 800), ClockStep::Slept(1));
}

#[test]
fn a_wall_falling_behind_the_awake_clock_is_a_step_back() {
    assert_eq!(once(100, 100, 5_000), ClockStep::Back(5));
    // Rounded down: 951.5 s.
    assert_eq!(once(1_000, 50, 1_500), ClockStep::Back(951));
    // Under a second behind: not reported (it runs late).
    assert_eq!(once(100, 100, 900), ClockStep::Kept);
    assert_eq!(once(100, 99, 0), ClockStep::Back(1));
    // Keeping pace, or ahead, is no step back.
    assert_eq!(once(100, 700, 600_000), ClockStep::Kept);
    assert_eq!(once(100, 1_100, 1_000), ClockStep::Slept(999));
}

#[test]
fn the_clock_reads_from_its_anchor() {
    let clock = AwakeClock::default();
    let at = Instant::now();
    let read = |wall, secs| {
        read_at(
            &clock,
            Duration::from_secs(wall),
            at + Duration::from_secs(secs),
        )
    };
    assert_eq!(
        read(100, 0),
        ClockStep::Kept,
        "the first read is the anchor"
    );
    assert_eq!(read(101, 1), ClockStep::Kept);
    assert_eq!(read(1_102, 2), ClockStep::Slept(1_000));
    assert_eq!(read(1_103, 3), ClockStep::Kept);
    // Set back to true time: the lag is the forward step's.
    assert_eq!(read(104, 4), ClockStep::Back(1_000));
    assert_eq!(read(105, 5), ClockStep::Kept);
}

/// A slow drift back is never reported: the waits it spans run long by it.
#[test]
fn a_drift_back_is_not_reported() {
    let clock = AwakeClock::default();
    let at = Instant::now();
    assert_eq!(read_at(&clock, ms(100_000), at), ClockStep::Kept);
    for i in 1..=10_u64 {
        // The wall loses 0.5 s a read.
        let wall = ms(100_000 + i * 500);
        let step = read_at(&clock, wall, at + Duration::from_secs(i));
        assert_eq!(step, ClockStep::Kept, "read {i}");
    }
}
