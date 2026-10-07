//! A stand-in `git` the test holds (M9.1: the deadline tests made deterministic). Its
//! first call prints one line and exits 0; every later call is held until the test
//! releases it. A git read through it ends only by its caller's deadline, never by
//! how long a call happened to take.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// How often the waits below look.
const POLL: Duration = Duration::from_millis(10);

/// The stand-in's own cap on a held call, in its 50 ms steps (60 s): a test that
/// panics before it releases leaves no process behind for long. It also exits at once
/// when its directory is gone.
const HOLD_STEPS: u32 = 1200;

pub(crate) struct GatedGit {
    dir: PathBuf,
    program: PathBuf,
}

impl GatedGit {
    /// A stand-in in `dir` (created) whose first call answers `first`.
    pub(crate) fn new(dir: &Path, first: &str) -> GatedGit {
        std::fs::create_dir_all(dir).unwrap();
        let d = dir.display();
        let program = dir.join("git.sh");
        let body = format!(
            "#!/bin/sh\n\
             d='{d}'\n\
             echo \"$*\" >> \"$d/calls\"\n\
             if mkdir \"$d/answered\" 2>/dev/null; then printf '%s\\n' '{first}'; exit 0; fi\n\
             : > \"$d/held\"\n\
             n=0\n\
             while [ ! -e \"$d/release\" ] && [ \"$n\" -lt {HOLD_STEPS} ]; do\n\
             \x20 [ -d \"$d\" ] || exit 1\n\
             \x20 sleep 0.05\n\
             \x20 n=$((n+1))\n\
             done\n\
             : > \"$d/exited\"\n\
             exit 1\n"
        );
        testexec::write_executable(&program, body);
        GatedGit {
            dir: dir.to_path_buf(),
            program,
        }
    }

    pub(crate) fn program(&self) -> std::ffi::OsString {
        self.program.clone().into_os_string()
    }

    /// A later call is running and held: it started and has not exited.
    pub(crate) fn held(&self) -> bool {
        self.dir.join("held").exists() && !self.dir.join("exited").exists()
    }

    /// Each call's arguments, in order.
    pub(crate) fn calls(&self) -> Vec<String> {
        let calls = std::fs::read_to_string(self.dir.join("calls")).unwrap_or_default();
        calls.lines().map(str::to_string).collect()
    }

    /// Lets the held call exit (non-zero).
    pub(crate) fn release(&self) {
        std::fs::write(self.dir.join("release"), "").unwrap();
    }

    /// Waits, yielding to the runtime, until a call is held; panics after `within`.
    pub(crate) async fn wait_held(&self, within: Duration) {
        self.wait(within, "call held", || self.held()).await;
    }

    /// Waits, yielding to the runtime, until the held call has exited; panics after
    /// `within`.
    pub(crate) async fn wait_exited(&self, within: Duration) {
        let exited = || self.dir.join("exited").exists();
        self.wait(within, "exit of the held call", exited).await;
    }

    async fn wait(&self, within: Duration, what: &str, done: impl Fn() -> bool) {
        let deadline = tokio::time::Instant::now() + within;
        while !done() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "no {what} within {within:?}; calls {:?}",
                self.calls()
            );
            tokio::time::sleep(POLL).await;
        }
    }
}

impl Drop for GatedGit {
    /// A test that failed before its release still lets the held call go.
    fn drop(&mut self) {
        let _ = std::fs::write(self.dir.join("release"), "");
    }
}
