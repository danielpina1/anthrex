//! Final review B-10 (the parked T25-N1, in both orders): a process's exit can reach
//! the engine twice, as the engine's own synthetic exit (`kill_effect`, for a window
//! whose process was already reaped) and as the real one, which trails it by up to the
//! session driver's output grace. The round may have been resumed in between (a
//! hand-back reopening a killed round, or decision 32's resume after an exit), with no
//! process yet. The second exit of the same process must not end, or count a death on,
//! the resumed round.

use super::fixture::*;
use super::turns::working_on;
use crate::run::engine::AgentSignal;

fn exit(fx: &mut Fixture, window: u32, pid: u32, killed: bool) {
    fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: killed,
            pid,
        },
    );
}

/// A mid-turn exit is resumed (decision 32); the same exit again (the engine's
/// synthetic one after the real, or the reverse) is not a second death.
#[test]
fn a_repeated_exit_of_one_process_is_not_a_second_death() {
    let (mut fx, window) = working_on("");
    fx.signal(window, AgentSignal::ProcessStarted { pid: 41 });
    exit(&mut fx, window, 41, false);
    let round = &fx.task("t1").rounds[0];
    assert_eq!((round.deaths, round.ended, round.pid), (1, false, None));
    for killed in [false, true] {
        exit(&mut fx, window, 41, killed);
        let round = &fx.task("t1").rounds[0];
        assert_eq!((round.deaths, round.ended), (1, false), "killed = {killed}");
    }
    assert_eq!(fx.task("t1").stalls, 0);
    // The resumed process's own exit still counts.
    fx.signal(window, AgentSignal::ProcessStarted { pid: 42 });
    exit(&mut fx, window, 42, false);
    assert_eq!(fx.task("t1").rounds[0].deaths, 2);
}

/// Both orders of T25-N1: the killed round is reopened by a hand-back (`ended = false`,
/// no process), then the other copy of its exit arrives, killed or not.
#[test]
fn the_other_copy_of_a_killed_exit_leaves_a_reopened_round_alone() {
    for second_killed in [false, true] {
        let (mut fx, window) = working_on("");
        fx.signal(window, AgentSignal::ProcessStarted { pid: 41 });
        exit(&mut fx, window, 41, true);
        assert!(fx.task("t1").rounds[0].ended);
        // What `reopen_stopped` and the outbox's `ResumeSession` do to the round.
        let round = &mut fx.task_mut("t1").rounds[0];
        round.ended = false;
        round.ended_at = None;
        round.turn_open = true;
        round.pid = None;
        exit(&mut fx, window, 41, second_killed);
        let round = &fx.task("t1").rounds[0];
        assert!(!round.ended, "second_killed = {second_killed}");
        assert_eq!(round.deaths, 0, "second_killed = {second_killed}");
    }
}

/// F3 review N2: the engine's synthetic exit for a round with no process carries pid 0,
/// and one kill sends only one. A second kill of the reopened round (before its resumed
/// process starts) sends another pid-0 exit, which is new, not a repeat: it ends the
/// round.
#[test]
fn a_second_synthetic_exit_ends_a_reopened_round() {
    let (mut fx, window) = working_on("");
    exit(&mut fx, window, 0, true);
    assert!(fx.task("t1").rounds[0].ended);
    let round = &mut fx.task_mut("t1").rounds[0];
    round.ended = false;
    round.ended_at = None;
    round.turn_open = true;
    round.pid = None;
    exit(&mut fx, window, 0, true);
    assert!(fx.task("t1").rounds[0].ended, "the second kill ends it");
}
