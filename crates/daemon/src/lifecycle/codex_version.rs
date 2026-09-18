//! One bounded startup probe. Pipes are nonblocking, so descendants cannot keep
//! a reader thread or the daemon waiting after the process deadline.

use crate::launch::codex::{MIN_CODEX_VERSION, parse_version};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub(super) async fn check(program: String) {
    let timeout = Duration::from_secs(5);
    let deadline = Instant::now() + timeout;
    let result = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || probe(&program, deadline)),
    )
    .await;
    match result {
        Ok(Ok(Ok(version))) if version < MIN_CODEX_VERSION => {
            tracing::warn!(
                ?version,
                ?MIN_CODEX_VERSION,
                "Codex version below supported minimum"
            );
        }
        Ok(Ok(Ok(_))) => {}
        other => tracing::debug!(result = ?other, "could not read Codex version"),
    }
}

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

fn probe(program: &str, deadline: Instant) -> anyhow::Result<(u64, u64, u64)> {
    anyhow::ensure!(
        Instant::now() < deadline,
        "probe deadline elapsed before spawn"
    );
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
    while Instant::now() < read_deadline {
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
    anyhow::bail!("version probe timed out")
}
