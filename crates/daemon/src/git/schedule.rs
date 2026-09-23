//! When to probe a root, and whether to publish the answer — as pure state machines.
//!
//! Everything about *timing* in the git subsystem lives in [`Scheduler`], and
//! everything about *publication* lives in [`Publisher`]. Neither reads a clock,
//! spawns anything, or touches the filesystem: the instant is always an argument and
//! the probe result is always an argument. [`crate::git::GitRegistry`]'s per-root task
//! is the only thing that knows about tokio, and it is a thin loop around these two.
//!
//! That split is what design decision 30 asks for. A test that had to sleep for thirty
//! seconds to watch the safety poll fire, or sixty to watch the circuit breaker
//! reopen, is a test nobody runs; `crates/daemon/tests/git_registry.rs` asserts both by
//! doing arithmetic on one base `Instant`.
//!
//! The instants are [`tokio::time::Instant`] rather than [`std::time::Instant`]
//! because the task feeds them straight to [`tokio::time::sleep_until`], and because
//! tokio's clock is the one a paused-time test can move.

use std::collections::VecDeque;
use std::time::Duration;

use proto::GitState;
use tokio::time::Instant;

/// Design decision 13: an accepted event probes this long after the *last* accepted
/// event, so a burst of writes costs one probe.
pub const DEBOUNCE: Duration = Duration::from_millis(300);
/// Design decision 13: every registered root is probed at least this often, whatever
/// the watcher does or fails to do.
pub const POLL_INTERVAL: Duration = Duration::from_secs(30);
/// Design decision 14: the window the breaker counts accepted events over.
pub const BREAKER_WINDOW: Duration = Duration::from_secs(10);
/// Design decision 14: *more than* this many accepted events inside
/// [`BREAKER_WINDOW`] trips the breaker, so the 201st is the one that trips it.
pub const BREAKER_EVENTS: usize = 200;
/// Design decision 14: how long a tripped root stays on poll-only.
pub const BREAKER_COOLDOWN: Duration = Duration::from_secs(60);

/// What [`Scheduler::plan`] decided for this instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// Start a probe now. Never true while one is already outstanding.
    pub probe: bool,
    /// The next instant at which calling [`Scheduler::plan`] again could change
    /// anything. Always in the future, so a caller that sleeps until it cannot spin.
    pub wake_at: Instant,
}

/// The timing state machine for one root.
///
/// The caller's contract is three calls: [`Scheduler::record_event`] for every
/// *accepted* watcher event (the path filter has already run), [`Scheduler::plan`] to
/// ask what should happen at an instant, and [`Scheduler::probe_finished`] when the
/// probe `plan` asked for has returned. A probe that was never finished blocks every
/// later one, which is design decision 10's "one probe in flight per root" expressed
/// as a type rather than as a comment.
#[derive(Debug)]
pub struct Scheduler {
    /// The next safety-poll deadline. Always set: a registered root always has a poll
    /// pending, which is why [`Plan::wake_at`] is not an `Option`.
    next_poll: Instant,
    /// The debounce deadline, if an accepted event is waiting for one. Never set while
    /// the breaker is open.
    debounce: Option<Instant>,
    /// Accepted events inside [`BREAKER_WINDOW`], oldest first. Cleared when the
    /// breaker trips, and not appended to while it is open, so a storm cannot grow it.
    events: VecDeque<Instant>,
    /// When the breaker reopens, while it is open.
    breaker_until: Option<Instant>,
    /// A probe is outstanding.
    in_flight: bool,
    /// A refresh was asked for while a probe was outstanding. Exactly one further
    /// probe runs when that one returns — a bit, never a queue (design decision 10).
    pending: bool,
    /// `config.toml`'s `git.poll_secs`, defaulting to [`POLL_INTERVAL`]. Held per
    /// scheduler rather than read from the constant so the configured value is the one
    /// thing that decides, with no second path back to the default (milestone 6's
    /// `[git]` table; git-surface spec 3.7).
    poll_interval: Duration,
    /// `config.toml`'s `git.debounce_ms`, defaulting to [`DEBOUNCE`].
    debounce_interval: Duration,
}

impl Scheduler {
    /// A freshly registered root: due for a probe at `now` (design decision 13,
    /// "immediately on registration").
    ///
    /// `poll_interval` and `debounce_interval` come from `config.toml`'s `[git]` table;
    /// [`POLL_INTERVAL`] and [`DEBOUNCE`] are their defaults, and `config::Git`'s own
    /// defaults are asserted equal to them in `crates/daemon/tests/git_schedule.rs`.
    pub fn new(now: Instant, poll_interval: Duration, debounce_interval: Duration) -> Self {
        Self {
            next_poll: now,
            debounce: None,
            events: VecDeque::new(),
            breaker_until: None,
            in_flight: false,
            pending: false,
            poll_interval,
            debounce_interval,
        }
    }

    /// Records one accepted watcher event at `now` and returns whether *this* event
    /// tripped the circuit breaker — the caller logs that, once per trip (decision 14).
    ///
    /// While the breaker is open the event is dropped on the floor rather than
    /// counted: the root is on poll-only anyway, and not counting is what keeps
    /// [`Self::events`] bounded during a `git checkout` of a large tree.
    pub fn record_event(&mut self, now: Instant) -> bool {
        self.refresh(now);
        if self.breaker_until.is_some() {
            return false;
        }
        self.events.push_back(now);
        if self.events.len() > BREAKER_EVENTS {
            self.breaker_until = Some(now + BREAKER_COOLDOWN);
            self.events.clear();
            self.debounce = None;
            return true;
        }
        self.debounce = Some(now + self.debounce_interval);
        false
    }

    /// Decides what happens at `now`, consuming whatever deadlines have come due.
    ///
    /// Starts at most one probe, and only when none is outstanding; a deadline that
    /// comes due while one is sets the pending bit instead.
    pub fn plan(&mut self, now: Instant) -> Plan {
        self.refresh(now);

        let poll_due = now >= self.next_poll;
        let debounce_due = self.debounce.is_some_and(|deadline| now >= deadline);
        if debounce_due {
            self.debounce = None;
        }

        let mut probe = false;
        if poll_due || debounce_due || self.pending {
            if self.in_flight {
                self.pending = true;
            } else {
                self.pending = false;
                self.in_flight = true;
                probe = true;
            }
        }

        // The poll deadline moves on whenever it comes due — including when it only
        // set the pending bit, because a deadline left in the past would make
        // `wake_at` be `now` for ever and spin the task's loop. It also moves on when
        // a probe starts for any other reason: the poll exists to bound how stale the
        // state can get, and a probe that has just started is that bound being met.
        if poll_due || probe {
            self.next_poll = now + self.poll_interval;
        }

        Plan {
            probe,
            wake_at: match self.debounce {
                Some(debounce) if debounce < self.next_poll => debounce,
                _ => self.next_poll,
            },
        }
    }

    /// Asks for one probe as soon as possible: at the next [`Self::plan`], or when the
    /// outstanding probe returns if one is in flight. No debounce, and not counted
    /// towards the breaker; it is not a watcher event.
    pub fn request_probe(&mut self) {
        self.pending = true;
    }

    /// The outstanding probe has returned. The next [`Self::plan`] may start another.
    pub fn probe_finished(&mut self) {
        self.in_flight = false;
    }

    /// Expires the breaker and drops events that have fallen out of the window.
    /// Called from both entry points so neither depends on the other being called.
    fn refresh(&mut self, now: Instant) {
        if self.breaker_until.is_some_and(|until| now >= until) {
            self.breaker_until = None;
        }
        while self
            .events
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= BREAKER_WINDOW)
        {
            self.events.pop_front();
        }
    }
}

/// Decides what a probe result is worth publishing, for one root.
///
/// Two rules, both of them about not lying to a client.
///
/// * An identical state is not published twice. `DaemonMsg::Git` is a broadcast; an
///   unchanged 30-second poll must not wake every attached TUI.
/// * A failed probe never publishes `None` over a state that once succeeded.
///   [`crate::git::probe::probe`] returns `None` for three different things — not a
///   working tree, `git` could not be spawned, `git` failed — but on the wire `None`
///   means only the first. So a root that has a last known state re-publishes *that*,
///   with [`GitState::stale`] set, and keeps doing so until a probe succeeds again; a
///   root that never succeeded publishes `None` once and then stays quiet.
#[derive(Debug, Default)]
pub struct Publisher {
    /// The last state a probe actually produced, if any.
    last_good: Option<GitState>,
    /// The last thing published. The outer `Option` is "has anything been published
    /// yet", which is what makes the `None`-once rule expressible.
    last_published: Option<Option<GitState>>,
}

impl Publisher {
    /// Returns what to publish for this probe result, or `None` to publish nothing.
    pub fn decide(&mut self, result: Option<GitState>) -> Option<Option<GitState>> {
        let candidate = match result {
            Some(state) => {
                self.last_good = Some(state.clone());
                Some(state)
            }
            None => self.last_good.as_ref().map(|good| GitState {
                stale: true,
                ..good.clone()
            }),
        };
        if self.last_published.as_ref() == Some(&candidate) {
            return None;
        }
        self.last_published = Some(candidate.clone());
        Some(candidate)
    }
}
