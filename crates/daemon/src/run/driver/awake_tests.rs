//! `AwakeClock`: what a read reports, and the property every report keeps (review
//! rounds 2 and 3 of the clock-step fix): no wait ends before its true awake time by
//! more than `READ_TOLERANCE + READ_SPREAD`, and none later than a few seconds past it.

use super::*;

/// A year, the forward step's length below.
const X: u64 = 365 * 24 * 3_600;

/// The wall time `ms` milliseconds after the epoch.
fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

/// One read of `clock` at wall time `wall`, `awake` the awake instant, both clocks
/// sampled at once.
fn read_at(clock: &AwakeClock, wall: Duration, awake: Instant) -> ClockStep {
    sampled(clock, &[(awake, wall, awake)]).0
}

/// A read whose attempts sample `(awake before, wall, awake after)` in turn (the last
/// repeated if the clock asks for more): what it reported, and the wall time the
/// driver's `now` comes from.
fn sampled(clock: &AwakeClock, samples: &[(Instant, Duration, Instant)]) -> (ClockStep, Duration) {
    let mut next = samples.iter().copied();
    let mut wall = Duration::ZERO;
    let step = clock.read(|| {
        let (before, w, after) = next.next().unwrap_or(samples[samples.len() - 1]);
        wall = w;
        Sample {
            before,
            wall: w,
            after,
        }
    });
    (step, wall)
}

/// A wait armed at a read: the wall and awake times it was armed at, the seconds
/// reported by then, and whether it has ended.
struct Stamp {
    wall: u128,
    awake: u128,
    reported: i128,
    ended: bool,
}

/// A simulated host: its true wall and awake clocks, what the clock reported, and the
/// waits armed at earlier reads.
struct Host {
    clock: AwakeClock,
    at: Instant,
    /// Nanoseconds since the epoch, and awake nanoseconds since `at`.
    wall: u128,
    awake: u128,
    reported: i128,
    stamps: Vec<Stamp>,
    /// Review round 3 (T1): every wait has ended by `BOUND` plus this, awake.
    late: u128,
}

/// A wait's bound: a stall's `stall_after_secs`.
const BOUND: u64 = 300;
/// A second, in nanoseconds.
const NS: u128 = 1_000_000_000;

impl Host {
    fn new(wall_ms: u64) -> Self {
        let mut host = Host {
            clock: AwakeClock::default(),
            at: Instant::now(),
            wall: u128::from(wall_ms) * 1_000_000,
            awake: 0,
            reported: 0,
            stamps: Vec::new(),
            late: 3 * NS,
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
        let ns = u128::from(secs) * NS;
        self.wall = if ahead {
            self.wall + ns
        } else {
            self.wall - ns
        };
    }

    fn now(&self) -> (Duration, Instant) {
        let wall = Duration::from_nanos(u64::try_from(self.wall).unwrap());
        let awake = self.at + Duration::from_nanos(u64::try_from(self.awake).unwrap());
        (wall, awake)
    }

    /// A read, as the driver's `handle` makes one.
    fn read(&mut self) -> ClockStep {
        let (wall, awake) = self.now();
        let (step, used) = sampled(&self.clock, &[(awake, wall, awake)]);
        self.check(step, used)
    }

    /// Review round 3 (N4): a read whose thread is descheduled for `ns` after reading
    /// the wall clock, before the awake clock: its next attempt samples both at once.
    fn read_stalled(&mut self, ns: u128) -> ClockStep {
        let (wall, before) = self.now();
        self.pass(ns, 0);
        let (wall2, after) = self.now();
        let (step, used) = sampled(&self.clock, &[(before, wall, after), (after, wall2, after)]);
        self.check(step, used)
    }

    /// The check of every armed wait after a read whose `now` came from wall time
    /// `used`: once the reducer's integer clock says its bound has passed (`not_before`
    /// from its truncated arming second, moved by the steps reported since), its awake
    /// time is at least the bound, less the tolerance; and by `late` past the bound, it
    /// has passed.
    fn check(&mut self, step: ClockStep, used: Duration) -> ClockStep {
        match step {
            ClockStep::Kept => {}
            ClockStep::Slept(s) => self.reported += i128::from(s),
            ClockStep::Back(s) => self.reported -= i128::from(s),
        }
        let now = i128::from(used.as_secs());
        let tolerance = (READ_TOLERANCE + READ_SPREAD).as_nanos();
        let bound = u128::from(BOUND) * NS;
        for stamp in &mut self.stamps {
            let shift = self.reported - stamp.reported;
            let due = i128::try_from(stamp.wall / NS).unwrap() + i128::from(BOUND) + 1 + shift;
            let spent = self.awake - stamp.awake;
            if now >= due && !stamp.ended {
                stamp.ended = true;
                assert!(
                    spent + tolerance >= bound,
                    "a wait armed at {} ns ended after {spent} ns awake ({step:?})",
                    stamp.wall
                );
            }
            assert!(
                stamp.ended || spent <= bound + self.late,
                "a wait armed at {} ns had not ended after {spent} ns awake",
                stamp.wall
            );
        }
        step
    }

    /// A wait armed now, at the read just made.
    fn arm(&mut self) {
        let (wall, awake, reported) = (self.wall, self.awake, self.reported);
        self.stamps.push(Stamp {
            wall,
            awake,
            reported,
            ended: false,
        });
    }

    /// Drops the waits armed more than twice the bound ago (each has ended).
    fn forget(&mut self) {
        let awake = self.awake;
        self.stamps
            .retain(|s| awake - s.awake < 2 * u128::from(BOUND) * NS);
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
        // The drift back is never reported: a wait runs long by the drift over it.
        host.late += u128::from(BOUND) * NS * ppm / 1_000_000;
        for i in 0..20_000_u64 {
            host.pass(1_000_000_000 + u128::from(i % 7) * 13_000_000, ppm);
            host.read();
            if i % 97 == 0 {
                host.arm();
            }
            // Keep the check bounded.
            host.forget();
        }
    }
}

/// An awake clock slow against the wall reports a rise of the offset as `Slept` at
/// once, so no wait ends before its awake bound either.
#[test]
fn a_slow_awake_clock_ends_no_wait_early() {
    let mut host = Host::new(100_000);
    // A `Slept(1)` for each 0.1 s the wall gains: 0.3 s over a wait, so 3 s late.
    host.late += 4 * NS;
    for i in 0..5_000_u64 {
        host.awake += 1_000_000_000;
        host.wall += 1_000_000_000 + 1_000_000; // the wall gains 1,000 ppm
        host.read();
        if i % 31 == 0 {
            host.arm();
        }
        host.forget();
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

/// Review round 3 (N4): the thread descheduled for 1.1 to 2 s between reading the wall
/// clock and the awake clock understated the offset by the stall, so the read reported
/// a false `Back` and a wait due at that read ended up to the stall early. A read whose
/// awake clock moved more than `READ_SPREAD` across it is sampled again, so no wait
/// here ends early, and none ends late.
#[test]
fn a_thread_stalled_between_the_clock_reads_ends_no_wait_early() {
    for stall in [1_100_u64, 1_500, 1_900, 2_000, 5_000] {
        for phase in [0, 300, 700] {
            let mut host = Host::new(100_000 + phase);
            for i in 0..2_000_u64 {
                if i % 37 == 11 {
                    host.read_stalled(u128::from(stall) * 1_000_000);
                } else {
                    host.pass(300_000_000, 0);
                    host.read();
                }
                host.arm();
                host.forget();
            }
        }
    }
}

/// A read whose every attempt is stalled reports nothing and leaves the clock as it
/// was: the next read measures from the same anchor.
#[test]
fn a_read_stalled_on_every_attempt_reports_nothing() {
    let clock = AwakeClock::default();
    let at = Instant::now();
    assert_eq!(read_at(&clock, ms(100_000), at), ClockStep::Kept);
    let stalled = (at + ms(1_000), ms(101_000), at + ms(3_000));
    assert_eq!(sampled(&clock, &[stalled]).0, ClockStep::Kept);
    // A real step back at the next read is still seen in full.
    assert_eq!(
        read_at(&clock, ms(91_000), at + ms(4_000)),
        ClockStep::Back(13)
    );
}

/// The first awake instant is the one the offset is measured from: the wall is read
/// after it, so the offset reads high (late), never low. A step back of exactly 1 s
/// read 5 ms into the sample is not reported in full.
#[test]
fn a_sample_is_measured_from_its_first_awake_instant() {
    let clock = AwakeClock::default();
    let at = Instant::now();
    assert_eq!(read_at(&clock, ms(100_000), at), ClockStep::Kept);
    let sample = (at + ms(2_000), ms(101_005), at + ms(2_005));
    assert_eq!(sampled(&clock, &[sample]).0, ClockStep::Kept);
}
