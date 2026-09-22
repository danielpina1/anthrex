//! One bounded startup probe. Pipes are nonblocking, so descendants cannot keep
//! a reader thread or the daemon waiting after the process deadline.

use crate::launch::LaunchGate;
use crate::launch::codex::{MIN_CODEX_VERSION, parse_version};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// The whole startup probe's wall-clock budget: a window launch never waits on this probe
/// past this, because [`start`]'s watchdog below is an async timer that opens the launch
/// gate on schedule regardless of how the probe's own `spawn_blocking` closure is doing.
///
/// It is not, and must never again become, a budget the *handshake* spends: `serve` now
/// runs beside this probe rather than after it. That is the whole point of the
/// [`LaunchGate`] — see its module doc for the two-5-second-constants race that ordering
/// used to create.
///
/// `pub` (not `pub(super)`) so a bound outside this crate that must legitimately exceed
/// a daemon startup delayed by this probe — `crates/cli/tests/codex_version.rs`'s own
/// `probe()` helper being the motivating case — can derive its bound from this constant
/// instead of retyping `5` and hoping it never drifts (`docs/timing-budgets.md` standing
/// rule 1). Named `CODEX_PROBE_TIMEOUT` rather than `PROBE_TIMEOUT` because
/// `daemon::git::probe` already has an unrelated constant of that bare name; keeping them
/// textually distinct avoids a same-name-different-thing mixup at any call site that ends
/// up needing both.
pub const CODEX_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Starts the one startup probe, beside `server::serve` rather than ahead of it.
///
/// `gate` is opened as soon as the probe is done with the `codex` binary, or at
/// [`CODEX_PROBE_TIMEOUT`] at the latest — whichever comes first. Two independent opens,
/// because they answer two different failure modes and `LaunchGate::open` is idempotent:
///
/// - the [`LaunchGate::open_on_drop`] guard inside the blocking closure covers the normal
///   case and every abnormal one that still runs the closure (an error, a panic), and it
///   drops *after* [`ProbeChild`]'s own kill-and-reap, so a launch released by it can
///   never overlap the probe's child;
/// - the watchdog task covers the case the closure never runs at all. `spawn_blocking`
///   queues onto a pool this daemon shares with every worktree operation and every window
///   spawn; a saturated pool must not hold a window launch — and, through it, a client's
///   `CREATE_WINDOW_REPLY_TIMEOUT` — past the budget this constant promises.
///
/// The returned handle completes only once the blocking closure has, child included.
/// `lifecycle::run` cancels `shutdown` and awaits it before the daemon exits: the probe
/// now outlives `serve`'s start, so a `daemon stop` inside the probe's budget would
/// otherwise leave a `codex --version` child orphaned — the process group `ProbeChild`
/// kills on drop is only killed if that drop actually happens before the process exits.
///
/// That cancellation has one accepted cost, new with this ordering: a daemon stopped
/// inside the probe's budget never establishes the version, so its `warn` for an
/// unsupported Codex is not logged. It is advisory — nothing reads the probe's result but
/// the log — and the alternative is letting an exiting daemon sit on a `codex` that is
/// not answering for up to `CODEX_PROBE_TIMEOUT`, inside a `daemon stop` whose own client
/// gives the lock 10 s to be released. Losing an advisory line on a daemon that lived for
/// under five seconds is the cheaper of the two.
pub(super) fn start(program: String, gate: LaunchGate, shutdown: CancellationToken) -> JoinHandle<()> {
    let deadline = Instant::now() + CODEX_PROBE_TIMEOUT;
    let probe_gate = gate.clone();
    let handle = tokio::task::spawn_blocking(move || {
        let _open_when_done = probe_gate.open_on_drop();
        report(probe(&program, deadline, &shutdown));
    });
    tokio::spawn(async move {
        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
        gate.open();
    });
    handle
}

/// Decision from milestone 3: a Codex too old for the hook flags is a `warn`, anything
/// else the probe could not establish is `debug` — an unreadable version is not the
/// user's problem to act on.
///
/// Logged from inside the blocking closure rather than by whoever awaits it, so the
/// warning still lands at startup now that nothing awaits the probe until shutdown.
///
/// [`PROBE_FINISHED`] is emitted unconditionally afterwards, which the three cases above
/// are not: a supported version is silent, by design. "The probe is over" is now an event
/// with observers — the launch gate is open, the probe's child is gone — and a daemon that
/// is asked to stop before this line can legitimately never log any of the three. Without
/// one stable line for it, a test that needs to let the probe finish before stopping the
/// daemon would have to guess which of the three outcomes its own stub will produce.
fn report(result: anyhow::Result<(u64, u64, u64)>) {
    match result {
        Ok(version) if version < MIN_CODEX_VERSION => {
            tracing::warn!(
                ?version,
                ?MIN_CODEX_VERSION,
                "Codex version below supported minimum"
            );
        }
        Ok(_) => {}
        other => tracing::debug!(result = ?other, "could not read Codex version"),
    }
    tracing::debug!("{PROBE_FINISHED}");
}

/// The one line the probe always logs when it is over, whatever it found. `pub` for the
/// same reason [`CODEX_PROBE_TIMEOUT`] is: `crates/cli/tests/codex_version.rs` waits for
/// it rather than retyping the string (`docs/timing-budgets.md` standing rule 1's
/// principle, applied to a marker rather than a duration).
pub const PROBE_FINISHED: &str = "codex version probe finished";

struct ProbeChild {
    child: Child,
    deadline: Instant,
    completed: bool,
    reaped: bool,
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        // process_group(0) gives this probe its own group, including descendants.
        let _ = crate::process::signal_group(self.child.id(), libc::SIGKILL);
        while !self.reaped && Instant::now() < self.deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => self.reaped = true,
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => break,
            }
        }
    }
}

fn probe(
    program: &str,
    deadline: Instant,
    shutdown: &CancellationToken,
) -> anyhow::Result<(u64, u64, u64)> {
    anyhow::ensure!(
        Instant::now() < deadline,
        "probe deadline elapsed before spawn"
    );
    anyhow::ensure!(!shutdown.is_cancelled(), "probe cancelled before spawn");
    let child = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let mut owned = ProbeChild {
        child,
        deadline,
        completed: false,
        reaped: false,
    };
    let mut stdout = owned.child.stdout.take().expect("stdout was piped");
    let fd = stdout.as_raw_fd();
    // SAFETY: stdout owns a live pipe fd; fcntl neither transfers nor closes it.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error().into());
    }
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let mut eof = false;
    let mut status = None;
    // Reserve cleanup time inside the same five-second budget.
    let read_deadline = deadline - Duration::from_millis(100);
    // `shutdown` is checked on every turn of this loop, not only at entry: the daemon can
    // be asked to stop at any point inside the budget now that this probe runs beside
    // `serve`, and the whole reason `lifecycle::run` waits for this closure before exiting
    // is that `ProbeChild`'s kill-and-reap must happen while the daemon is still alive.
    // Leaving the loop promptly is what makes that wait short enough to be free.
    while Instant::now() < read_deadline && !shutdown.is_cancelled() {
        if !eof {
            match stdout.read(&mut buffer) {
                Ok(0) => eof = true,
                Ok(count) => {
                    anyhow::ensure!(bytes.len() + count <= 64 * 1024, "version output too large");
                    bytes.extend_from_slice(&buffer[..count]);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        if status.is_none() {
            status = owned.child.try_wait()?;
            owned.reaped = status.is_some();
        }
        if eof && let Some(status) = status {
            owned.completed = true;
            anyhow::ensure!(status.success(), "version command failed: {status}");
            let output = std::str::from_utf8(&bytes)?;
            let version = parse_version(output)
                .ok_or_else(|| anyhow::anyhow!("unrecognised version output"))?;
            return Ok(version);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    anyhow::ensure!(!shutdown.is_cancelled(), "version probe cancelled");
    anyhow::bail!("version probe timed out")
}
