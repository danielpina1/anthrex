//! The session rows of decision 44's reconcile table, and decision 28's check for
//! leftover session processes. Blocking.
//!
//! **Leftover processes (decision 28; fix round 1, ruling T21-I1).** A session's process
//! belonged to the old daemon; one still alive after a restart would interleave the
//! session's transcript with its resume. Only the pid a live (not ended) round recorded
//! is examined; no other process on the machine is ever looked at. It is signalled only
//! when all of these hold: it is alive (not a zombie), it runs as the daemon's uid, its
//! parent is [`ORPHAN_PARENT`] (the old daemon, its parent, died), and its argv holds
//! the session id (the round's `session_id`, else its launching `CreateWindow`'s
//! `session_uuid`) as a whole element or as `--session-id=<id>` / `--resume=<id>`.
//! `SIGTERM` goes to the process group when the pid leads it (as every headless session
//! does) and it is not the daemon's own, else to the pid alone; after
//! [`SESSION_KILL_GRACE`] a pid that still passes every check gets `SIGKILL`. A session
//! whose pid was never recorded, or Codex's first turn (no id in its argv), is not
//! found; the followups file carries the Codex marker.

use std::process::Command;
use std::time::{Duration, Instant};

use proto::{RunRef, WindowInfo, WindowKind};

use super::Reconciled;
use crate::run::engine::{OpKind, OpResult};
use crate::run::model::{PendingOp, Run};

/// The parent of a process whose parent died: init or launchd.
pub const ORPHAN_PARENT: u32 = 1;

/// Decision 28: `SIGTERM`, then `SIGKILL` after this.
pub const SESSION_KILL_GRACE: Duration = Duration::from_secs(2);

/// A session id is a UUID or a thread id; anything shorter is not trusted as one.
const MIN_ID_LEN: usize = 8;

/// `ps` prints one short line for one pid.
const PS_OUTPUT_MAX: usize = 64 * 1024;

const POLL: Duration = Duration::from_millis(50);

/// `CreateWindow`: the restored headless window whose `RunRef` is the launch's
/// (the spec's, else the round's that the op launched) is the op's result; the highest
/// id wins, being the latest registered.
pub(super) fn restored_window(
    run: &Run,
    pending: &PendingOp,
    spec_ref: Option<&RunRef>,
    windows: &[WindowInfo],
) -> Reconciled {
    let wanted = spec_ref.cloned().or_else(|| round_ref(run, pending));
    let Some(wanted) = wanted else {
        return Reconciled::NotStarted;
    };
    windows
        .iter()
        .filter(|w| w.kind == WindowKind::Headless && w.run.as_ref() == Some(&wanted))
        .map(|w| w.id)
        .max()
        .map_or(Reconciled::NotStarted, |window_id| {
            Reconciled::Replay(OpResult::Window { window_id })
        })
}

/// The `RunRef` a round launched by `pending` was registered with.
fn round_ref(run: &Run, pending: &PendingOp) -> Option<RunRef> {
    let task_id = pending.task_id.as_deref()?;
    let task = run.task(task_id)?;
    let round = task.rounds.iter().find(|r| r.launch_op == pending.op)?;
    Some(RunRef {
        run_id: run.id.clone(),
        task_id: Some(task_id.to_string()),
        role: round.role,
        session: round.session,
    })
}

/// One recorded session process to check: a live round's pid and the session id its
/// argv must carry (the round's `session_id`, else the `session_uuid` of the pending
/// `CreateWindow` that launched it).
struct Candidate {
    pid: u32,
    id: String,
}

/// Decision 28's candidates: only rounds that have not ended and recorded a pid.
fn candidates(run: &Run) -> Vec<Candidate> {
    let mut found = Vec::new();
    for round in run.tasks.iter().flat_map(|t| t.rounds.iter()) {
        let Some(pid) = round.pid.filter(|pid| *pid > 1 && *pid <= i32::MAX as u32) else {
            continue;
        };
        if round.ended {
            continue;
        }
        let launched = run
            .pending_ops
            .get(&round.launch_op)
            .and_then(|p| match &p.kind {
                OpKind::CreateWindow { session_uuid, .. } => session_uuid.clone(),
                _ => None,
            });
        let Some(id) = round.session_id.clone().or(launched) else {
            continue;
        };
        if id.len() >= MIN_ID_LEN && !id.contains(char::is_whitespace) {
            found.push(Candidate { pid, id });
        }
    }
    found
}

/// A candidate that passed every check, and how to signal it.
struct Leftover {
    pid: u32,
    group: bool,
    id: String,
}

/// Decision 28 (fix round 1, T21-I1): each live round's recorded pid is signalled only
/// when it is alive, runs as this daemon's uid, was orphaned (its parent is
/// `orphan_parent`: the old daemon, its parent, is dead), and its argv holds the
/// session id as a whole element (or `--session-id=<id>`, `--resume=<id>`). No other
/// process is ever examined. Returns what it did.
pub(super) fn kill_leftovers(run: &Run, orphan_parent: u32, timeout: Duration) -> Vec<String> {
    let mut notes = Vec::new();
    let mut leftovers = Vec::new();
    for candidate in candidates(run) {
        if let Some(leftover) = qualifies(&candidate, orphan_parent, timeout) {
            signal(&leftover, libc::SIGTERM);
            notes.push(format!(
                "killed a leftover process {} of session {} (SIGTERM{})",
                leftover.pid,
                leftover.id,
                if leftover.group { " to its group" } else { "" }
            ));
            leftovers.push(leftover);
        }
    }
    let deadline = Instant::now() + SESSION_KILL_GRACE;
    while !leftovers.is_empty() {
        std::thread::sleep(POLL);
        // A pid that no longer passes every check (it exited, or was reused) is dropped
        // and never signalled again.
        leftovers.retain(|l| {
            let candidate = Candidate {
                pid: l.pid,
                id: l.id.clone(),
            };
            qualifies(&candidate, orphan_parent, timeout).is_some()
        });
        if Instant::now() >= deadline {
            for leftover in &leftovers {
                signal(leftover, libc::SIGKILL);
                notes.push(format!(
                    "process {} of session {} outlived SIGTERM; sent SIGKILL",
                    leftover.pid, leftover.id
                ));
            }
            break;
        }
    }
    notes
}

/// Every check of decision 28 as ruled in T21-I1, on one recorded pid.
fn qualifies(candidate: &Candidate, orphan_parent: u32, timeout: Duration) -> Option<Leftover> {
    let status = status(candidate.pid, timeout)?;
    // SAFETY: getuid and getpgrp take no arguments and cannot fail.
    let (own_uid, own_group) = unsafe { (libc::getuid(), libc::getpgrp()) };
    if status.zombie || status.uid != own_uid || status.ppid != orphan_parent {
        return None;
    }
    let argv = process_argv(candidate.pid)?;
    if !argv_carries(&argv, &candidate.id) {
        return None;
    }
    Some(Leftover {
        pid: candidate.pid,
        group: status.pgid == candidate.pid && status.pgid as libc::pid_t != own_group,
        id: candidate.id.clone(),
    })
}

/// Whether `argv` holds `id` as a whole element, or as `--session-id=<id>` or
/// `--resume=<id>`. (`--resume <id>` and `exec resume <id>` have it as an element.)
fn argv_carries(argv: &[String], id: &str) -> bool {
    argv.iter().any(|arg| {
        arg == id
            || arg.strip_prefix("--session-id=") == Some(id)
            || arg.strip_prefix("--resume=") == Some(id)
    })
}

struct Status {
    uid: u32,
    ppid: u32,
    pgid: u32,
    zombie: bool,
}

/// `ps -o uid= -o ppid= -o pgid= -o stat= -p <pid>`: one process, by pid.
fn status(pid: u32, timeout: Duration) -> Option<Status> {
    let mut command = Command::new("ps");
    command.args([
        "-o",
        "uid=",
        "-o",
        "ppid=",
        "-o",
        "pgid=",
        "-o",
        "stat=",
        "-p",
        &pid.to_string(),
    ]);
    let crate::subprocess::Outcome::Complete(out) =
        crate::subprocess::run(&mut command, PS_OUTPUT_MAX, timeout)
    else {
        return None;
    };
    let text = String::from_utf8_lossy(&out);
    let mut fields = text.split_whitespace();
    Some(Status {
        uid: fields.next()?.parse().ok()?,
        ppid: fields.next()?.parse().ok()?,
        pgid: fields.next()?.parse().ok()?,
        zombie: fields.next()?.starts_with('Z'),
    })
}

/// The process's argv, element by element, as the kernel holds it.
#[cfg(target_os = "linux")]
fn process_argv(pid: u32) -> Option<Vec<String>> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(
        bytes
            .split(|b| *b == 0)
            .filter(|arg| !arg.is_empty())
            .map(|arg| String::from_utf8_lossy(arg).into_owned())
            .collect(),
    )
}

/// The process's argv, element by element, from `KERN_PROCARGS2`.
#[cfg(target_os = "macos")]
fn process_argv(pid: u32) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    let mut argmax: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: a read-only sysctl into a correctly sized integer.
    let ok = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut argmax as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if ok != 0 || argmax <= 0 {
        return None;
    }
    let mut buf = vec![0u8; argmax as usize];
    let mut size = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    // SAFETY: a read-only sysctl into a buffer of `size` bytes.
    let ok = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if ok != 0 {
        return None;
    }
    parse_procargs2(&buf[..size])
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_argv(_pid: u32) -> Option<Vec<String>> {
    None
}

/// `KERN_PROCARGS2`'s layout: `argc` (a native `i32`), the executable path, NUL
/// padding, then `argc` NUL-terminated arguments (then the environment, ignored).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_procargs2(buf: &[u8]) -> Option<Vec<String>> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let mut rest = &buf[4..];
    let path_end = rest.iter().position(|b| *b == 0)?;
    rest = &rest[path_end..];
    let start = rest.iter().position(|b| *b != 0)?;
    rest = &rest[start..];
    let mut argv = Vec::new();
    for _ in 0..argc.max(0) {
        let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
        argv.push(String::from_utf8_lossy(&rest[..end]).into_owned());
        rest = rest.get(end + 1..).unwrap_or(&[]);
    }
    Some(argv)
}

fn signal(leftover: &Leftover, signal: i32) {
    let pid = leftover.pid as libc::pid_t;
    // SAFETY: plain signal syscalls on a recorded pid (or the group it leads) that just
    // passed every check of decision 28.
    let result = unsafe {
        if leftover.group {
            libc::killpg(pid, signal)
        } else {
            libc::kill(pid, signal)
        }
    };
    if result == -1 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            tracing::warn!(?error, pid, signal, "could not signal a leftover session");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{argv_carries, parse_procargs2};

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn the_session_id_must_be_a_whole_argv_element() {
        let id = "0000-aaaa-1111";
        assert!(argv_carries(&argv(&["claude", "--resume", id]), id));
        assert!(argv_carries(&argv(&["claude", "--session-id", id]), id));
        assert!(argv_carries(
            &argv(&["codex", "exec", "resume", id, "go"]),
            id
        ));
        assert!(argv_carries(
            &argv(&["claude", &format!("--resume={id}")]),
            id
        ));
        assert!(argv_carries(
            &argv(&["claude", &format!("--session-id={id}")]),
            id
        ));
        assert!(!argv_carries(
            &argv(&["tail", "-f", &format!("/p/{id}.jsonl")]),
            id
        ));
        assert!(!argv_carries(&argv(&["grep", &format!("{id}x")]), id));
        assert!(!argv_carries(
            &argv(&["claude", &format!("--other={id}")]),
            id
        ));
    }

    #[test]
    fn procargs2_is_read_element_by_element() {
        let mut buf = 3i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/bin/sleep\0\0\0\0sleep\0--resume\0a b\0HOME=/x\0");
        assert_eq!(
            parse_procargs2(&buf),
            Some(vec!["sleep".into(), "--resume".into(), "a b".into()])
        );
    }
}
