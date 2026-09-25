//! The run harness's daemon, owned as a child process (M8a.24 fix round 1).
//!
//! `anthrex daemon start` detaches the daemon and gives up after the CLI's own 3 s
//! (`ENSURE_DAEMON_SOCKET_WAIT`). Under load the daemon can bind its socket after that,
//! and a harness that trusted the CLI's failure left it running. So the harness starts
//! `anthrex daemon start --foreground` itself (what the detached spawn runs), keeps the
//! `Child`, and waits for the socket with its own bound. Whatever happens, the harness
//! still owns that exact process and stops it: through `anthrex daemon stop` once its
//! socket is up, else by killing the child. The pid comes from the unreaped `Child`
//! handle, so it cannot name any other process; nothing scans the process table.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long the daemon may take to bind its socket. Before binding it loads its state
/// and config and restores its runs; a restored run's reconcile makes git calls at the
/// harness's `git_timeout_secs = 5`, the same class and count as `run start`'s
/// preflight (at most nine calls, 45 s). The watcher-arming stall of a freshly built
/// executable (1.5–7.8 s, `docs/timing-budgets.md`) happens after binding. A daemon
/// that exits early is seen at once, so only a hung start waits this out.
pub const DAEMON_START_WAIT: Duration = Duration::from_secs(60);

/// How long a stopped daemon may take to exit: `anthrex daemon stop`'s own worst case
/// (19 s, `docs/timing-budgets.md`), with margin.
pub const DAEMON_EXIT_WAIT: Duration = Duration::from_secs(30);

pub struct DaemonProcess {
    child: Child,
    exited: bool,
}

impl DaemonProcess {
    /// Spawns `command` (`anthrex daemon start --foreground` with the harness's
    /// environment) in its own process group, its output appended to `log`.
    pub fn spawn(command: &mut Command, log: &Path) -> DaemonProcess {
        let out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        let child = command
            .stdin(Stdio::null())
            .stdout(out.try_clone().unwrap())
            .stderr(out)
            .process_group(0)
            .spawn()
            .expect("spawn the daemon");
        DaemonProcess {
            child,
            exited: false,
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Whether the process has exited (and is reaped).
    pub fn has_exited(&mut self) -> bool {
        if !self.exited && matches!(self.child.try_wait(), Ok(Some(_))) {
            self.exited = true;
        }
        self.exited
    }

    /// Waits at most `wait` for `socket` to accept a connection. `Err` when the daemon
    /// exited first or the wait ran out; the process is still owned either way.
    pub fn wait_up(&mut self, socket: &Path, wait: Duration) -> Result<(), String> {
        let deadline = Instant::now() + wait;
        loop {
            if std::os::unix::net::UnixStream::connect(socket).is_ok() {
                return Ok(());
            }
            if self.has_exited() {
                return Err("the daemon exited before its socket was up".into());
            }
            if Instant::now() >= deadline {
                return Err(format!("no socket within {wait:?}"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits at most `wait` for the process to exit.
    pub fn wait_exit(&mut self, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        while !self.has_exited() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    /// Stops the daemon: once its socket is up (waiting out a slow start), `stop` runs
    /// `anthrex daemon stop` against it; a daemon that does not exit, or never came up,
    /// is killed. Always leaves the process reaped.
    pub fn stop(&mut self, socket: &Path, start_wait: Duration, stop: impl FnOnce()) {
        if self.has_exited() {
            return;
        }
        if self.wait_up(socket, start_wait).is_ok() {
            stop();
            if self.wait_exit(DAEMON_EXIT_WAIT) {
                return;
            }
        }
        self.kill();
    }

    /// Kills the process if it has not exited, and reaps it. Our own unreaped child:
    /// its pid cannot have been reused, and nothing else is signalled.
    pub fn kill(&mut self) {
        if !self.has_exited() {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.exited = true;
        }
    }
}

/// M8a.24 fix round 2 (N2): a daemon dropped without a completed stop (a panic in the
/// stop, a harness unwinding) is killed, never leaked.
impl Drop for DaemonProcess {
    fn drop(&mut self) {
        self.kill();
    }
}
