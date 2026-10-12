//! Bounds the daemon waits under that the client's alert texts also count (milestone 9.12
//! decision 10's note): the host's read and push timeouts and the preflight bound
//! computed from them, the tuning start bound and the installed-agent probe's timeout.
//! Pure.

use std::time::Duration;

/// Decision 9: `gh` reads, `git config`/`ls-remote`/`rev-parse`/`merge-base`.
pub const HOST_READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Decision 9: `git push` (the dry run too), `git fetch`, a branch delete.
pub const PUSH_TIMEOUT: Duration = Duration::from_secs(120);

/// Preflight's whole bound (task M9.2.12): its six checks' own (decision 9), the
/// ahead count's read (the final fix wave, A5), the seal's two reads, and a margin, so
/// a host that ignores its per-command timeouts still answers. The CLI's `run start`
/// reply bound is derived from it.
pub const PREFLIGHT_BOUND: Duration =
    Duration::from_secs(8 * HOST_READ_TIMEOUT.as_secs() + PUSH_TIMEOUT.as_secs() + 5);

/// Ruling T9-3: how long a start waits for its tuning, the repository's tuning lock and
/// the file work together (one history read and one small write); past it the run
/// starts untuned and says so (the daemon's `run::driver::tuning::TUNING_BUSY`). Every
/// start bound that counts the build counts it (`docs/timing-budgets.md`).
pub const TUNING_START_BOUND: Duration = Duration::from_secs(10);

/// How long the probe may take. It stats two paths; only a hung file system is slower.
/// A goal start's triage waits for it before the decider call (`docs/timing-budgets.md`).
pub const INSTALLED_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
