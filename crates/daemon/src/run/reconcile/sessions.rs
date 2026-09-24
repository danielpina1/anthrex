//! The session rows of decision 44's reconcile table, and decision 28's check for
//! leftover session processes. Blocking.
//!
//! **Leftover processes.** A session's process belonged to the old daemon; one still
//! alive after a restart would interleave the session's transcript with its resume. A
//! process is a leftover when its command line (`ps -o args=`) contains the id of a
//! session the run still has live: a round's `session_id` that has not ended, a pending
//! `CreateWindow`'s `session_uuid`, a pending `ResumeSession`'s `session_id`. That
//! finds a process whose pid was never recorded (it died before `ProcessStarted` was
//! persisted) as well as one whose pid was. **A process whose command line lacks the id
//! is never signalled**, whatever pid a round recorded: a pid is reused, a session id
//! is not. `SIGTERM` goes to the process group when the process leads its own (every
//! headless session does, and never the daemon's own group), else to the process; after
//! [`SESSION_KILL_GRACE`] a process whose command line still carries the id gets
//! `SIGKILL`. Codex's first turn (`codex exec`, before it has a thread id) carries no
//! id and cannot be found this way; its round has no `session_id` yet either.

use std::collections::BTreeSet;
use std::process::Command;
use std::time::{Duration, Instant};

use proto::{RunRef, WindowInfo, WindowKind};

use super::Reconciled;
use crate::run::engine::{OpKind, OpResult};
use crate::run::model::{PendingOp, Run};

/// Decision 28: `SIGTERM`, then `SIGKILL` after this.
pub const SESSION_KILL_GRACE: Duration = Duration::from_secs(2);

/// Shorter strings are not searched for: a session id is a UUID or a thread id, and a
/// short one would match unrelated command lines.
const MIN_ID_LEN: usize = 8;

/// `ps` output is a line per process; the cap is generous for a busy machine.
const PS_OUTPUT_MAX: usize = 16 * 1024 * 1024;

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

/// Every session id whose process could outlive the old daemon (module docs).
pub(super) fn session_ids(run: &Run) -> Vec<String> {
    let mut ids = BTreeSet::new();
    for round in run.tasks.iter().flat_map(|t| t.rounds.iter()) {
        if !round.ended
            && let Some(id) = &round.session_id
        {
            ids.insert(id.clone());
        }
    }
    for pending in run.pending_ops.values() {
        match &pending.kind {
            OpKind::CreateWindow {
                session_uuid: Some(id),
                ..
            }
            | OpKind::ResumeSession { session_id: id, .. } => {
                ids.insert(id.clone());
            }
            _ => {}
        }
    }
    ids.into_iter()
        .filter(|id| id.trim().len() >= MIN_ID_LEN && !id.contains(char::is_whitespace))
        .collect()
}

/// One leftover: its pid, whether it leads its own group, and the id it carries.
struct Leftover {
    pid: i32,
    group: bool,
    id: String,
}

/// Decision 28: kills every process whose command line carries one of `ids`, and says
/// what it did.
pub(super) fn kill_leftovers(ids: &[String], timeout: Duration) -> Vec<String> {
    let mut notes = Vec::new();
    if ids.is_empty() {
        return notes;
    }
    let table = match processes(timeout) {
        Ok(table) => table,
        Err(err) => {
            notes.push(format!(
                "could not list processes to find leftover sessions: {err}"
            ));
            return notes;
        }
    };
    let own_pid = std::process::id() as i32;
    // SAFETY: getpgrp takes no arguments and cannot fail.
    let own_group = unsafe { libc::getpgrp() };
    let mut leftovers = Vec::new();
    for (pid, pgid, args) in table {
        if pid == own_pid || pid <= 1 {
            continue;
        }
        if let Some(id) = ids.iter().find(|id| args.contains(id.as_str())) {
            leftovers.push(Leftover {
                pid,
                group: pgid == pid && pgid != own_group,
                id: id.clone(),
            });
        }
    }
    for leftover in &leftovers {
        signal(leftover, libc::SIGTERM);
        notes.push(format!(
            "killed a leftover process {} of session {} (SIGTERM{})",
            leftover.pid,
            leftover.id,
            if leftover.group { " to its group" } else { "" }
        ));
    }
    let deadline = Instant::now() + SESSION_KILL_GRACE;
    loop {
        let alive: Vec<&Leftover> = leftovers
            .iter()
            .filter(|l| still_carries(l, timeout))
            .collect();
        if alive.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            for leftover in alive {
                signal(leftover, libc::SIGKILL);
                notes.push(format!(
                    "process {} of session {} outlived SIGTERM; sent SIGKILL",
                    leftover.pid, leftover.id
                ));
            }
            break;
        }
        std::thread::sleep(POLL);
    }
    notes
}

/// Whether `leftover.pid` still runs with its session id on its command line; a pid
/// that exited, became a zombie, or was reused by another program does not.
fn still_carries(leftover: &Leftover, timeout: Duration) -> bool {
    let mut command = Command::new("ps");
    command.args(["-ww", "-o", "args=", "-p", &leftover.pid.to_string()]);
    match crate::subprocess::run(&mut command, PS_OUTPUT_MAX, timeout) {
        crate::subprocess::Outcome::Complete(out) => {
            String::from_utf8_lossy(&out).contains(leftover.id.as_str())
        }
        _ => false,
    }
}

fn signal(leftover: &Leftover, signal: i32) {
    // SAFETY: plain signal syscalls on a pid (or the group it leads) whose command line
    // was just seen to carry one of this run's session ids.
    let result = unsafe {
        if leftover.group {
            libc::killpg(leftover.pid, signal)
        } else {
            libc::kill(leftover.pid, signal)
        }
    };
    if result == -1 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            tracing::warn!(
                ?error,
                pid = leftover.pid,
                signal,
                "could not signal a leftover session"
            );
        }
    }
}

/// `(pid, pgid, args)` for every process, from `ps -ww -A -o pid= -o pgid= -o args=`.
fn processes(timeout: Duration) -> Result<Vec<(i32, i32, String)>, String> {
    let mut command = Command::new("ps");
    command.args(["-ww", "-A", "-o", "pid=", "-o", "pgid=", "-o", "args="]);
    let out = match crate::subprocess::run(&mut command, PS_OUTPUT_MAX, timeout) {
        crate::subprocess::Outcome::Complete(out) => out,
        other => return Err(format!("ps did not complete: {other:?}")),
    };
    let text = String::from_utf8_lossy(&out);
    let mut table = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(pid), Some(pgid)) = (fields.next(), fields.next()) else {
            continue;
        };
        let (Ok(pid), Ok(pgid)) = (pid.parse::<i32>(), pgid.parse::<i32>()) else {
            continue;
        };
        let args = fields.collect::<Vec<_>>().join(" ");
        table.push((pid, pgid, args));
    }
    Ok(table)
}
