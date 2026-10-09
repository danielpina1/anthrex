//! A probe's child process, killed (its whole process group) and reaped on drop unless
//! the probe marked it completed. Shared by the startup Codex version probe and model
//! discovery.

use std::process::Child;
use std::time::{Duration, Instant};

pub(crate) struct ProbeChild {
    pub(crate) child: Child,
    pub(crate) deadline: Instant,
    pub(crate) completed: bool,
    pub(crate) reaped: bool,
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
