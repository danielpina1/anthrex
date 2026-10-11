//! The daemon's start constants the client's `ensure_daemon` also reads (milestone 9.12
//! decision 10's note). Pure.

use std::time::Duration;

/// How long the CLI's own daemon start waits for another daemon's lock to clear — long
/// enough to cover a `daemon stop` immediately followed by a start (decision 24).
pub const LOCK_WAIT: Duration = Duration::from_secs(5);

/// `<data_dir>/daemon.spawning`: written by a client's `ensure_daemon`
/// (`crates/tui/src/spawn.rs`) under its claim of the lifetime lock, just before it spawns
/// a daemon, and removed by that daemon once it holds the lock. While it is fresh, another
/// client that wins the claim in the gap between the two knows a daemon is already on its
/// way and spawns none (M9.11.28's fix round, S28-I1).
pub const SPAWN_MARKER: &str = "daemon.spawning";
