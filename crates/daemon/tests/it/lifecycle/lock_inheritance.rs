//! The lifetime lock and the daemon's children (9.19 follow-up 6). `assert_lock_released`
//! reads only this process's table, so a copy of `daemon.lock` a window child inherited,
//! which lives in another process, would hold the `flock` past the daemon's stop without
//! failing it. A submodule of `lifecycle.rs`, sharing its fixtures via `use super::*`.

use super::*;
use crate::support::descriptors::{child_descriptors_on, close_on_exec};

/// The child reader's positive control: a descriptor this test makes inheritable on
/// purpose, then spawns a child of its own across, is found in that child. Without it a
/// reader that never sees anything (a wrong `/proc` path, an `lsof` that lists nothing)
/// would pass the test below for nothing. The child is this test's own `sleep`, killed
/// through its own `Child` handle.
#[test]
fn the_child_reader_sees_an_inherited_descriptor() {
    use std::os::fd::AsRawFd;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inherited");
    let file = std::fs::File::create(&path).unwrap();
    // SAFETY: `F_SETFD` changes only `file`'s own descriptor flags; it stays open.
    assert_eq!(
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, 0) },
        0
    );
    assert_eq!(close_on_exec(file.as_raw_fd()), Some(false));
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawning sleep");
    drop(file);
    let held = child_descriptors_on(child.id(), &path);
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        !held.is_empty(),
        "the child (pid {}) inherited a descriptor on {}, and the reader did not see it",
        child.id(),
        path.display()
    );
}

/// While the daemon runs: every descriptor this process holds on `daemon.lock` is
/// close-on-exec, so no child any spawn makes can inherit it; and a real window child, the
/// shell, holds no descriptor on it. The first half is the one a lock opened inheritable
/// fails (checked by mutant); the second cannot be, as `portable-pty` closes every
/// descriptor above 2 in a PTY child before exec, so it pins that guarantee instead. The
/// shell names its own pid in a file (through `sh`'s `$PPID`, so any login shell works,
/// fish included), so the check reads that process and no other.
#[tokio::test]
async fn a_window_child_does_not_hold_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let data_dir = dir.path().join("data");
    let pid_file = dir.path().join("shell.pid");
    let handle = tokio::spawn(run(opts(socket.clone(), data_dir.clone())));
    wait_accepting(&socket, Duration::from_secs(5)).await;

    let (mut c, _welcome) = Client::connect(&socket).await;
    c.send(ClientMsg::CreateWindow {
        spec: shell_spec("w"),
        cols: 80,
        rows: 24,
    })
    .await;
    let window_id = match c
        .recv_until(|m| matches!(m, DaemonMsg::Created { .. }))
        .await
    {
        DaemonMsg::Created { window_id } => window_id,
        other => panic!("expected Created, got {other:?}"),
    };
    let tmp = dir.path().join("shell.pid.tmp");
    let line = format!(
        "sh -c 'echo $PPID > \"$1\" && mv \"$1\" \"$2\"' sh '{}' '{}'\n",
        tmp.display(),
        pid_file.display()
    );
    c.send(ClientMsg::Input {
        window_id,
        bytes: line.into_bytes(),
    })
    .await;
    let pid = wait_pid(&pid_file, Duration::from_secs(20)).await;

    let lock = data_dir.join("daemon.lock");
    let ours = descriptors_on(&lock);
    assert!(
        !ours.is_empty(),
        "the running daemon holds no descriptor on daemon.lock"
    );
    for fd in ours {
        assert_ne!(
            close_on_exec(fd),
            Some(false),
            "descriptor {fd} on daemon.lock is inheritable: a child spawned now keeps the lock"
        );
    }
    let child = child_descriptors_on(pid, &lock);
    assert!(
        child.is_empty(),
        "the window's shell (pid {pid}) holds descriptors {child:?} on {}",
        lock.display()
    );

    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;
    tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("daemon::run did not return within 10s")
        .expect("the daemon task panicked")
        .expect("run returned an error");
    assert_lock_released(&data_dir);
}

/// Waits, with a deadline, for the shell to have written its pid (a login shell reads the
/// user's profile first, so this is the slow part).
async fn wait_pid(path: &Path, timeout: Duration) -> u32 {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let read = std::fs::read_to_string(path).ok();
        if let Some(pid) = read.and_then(|s| s.trim().parse().ok()) {
            return pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the shell did not write its pid to {} within {timeout:?}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
