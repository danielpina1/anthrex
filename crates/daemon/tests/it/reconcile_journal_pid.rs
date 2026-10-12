//! M9.11.20 (decision 20, B6's unrecorded-pid case): restore stops a leftover session
//! whose pid only the run's journal holds. A session spawned just before the daemon
//! stopped has its pid in a journal `Done { Window { pid: Some(_) } }` line (M9.11.13)
//! but not in `run.json` (the engine's early-signal hold); reconcile makes that pid a
//! candidate of decision 28's leftover check, whose every other check is unchanged.
//!
//! R7: every process signalled here is one this test started (an orphaned `sleep` with
//! a test-unique `argv[0]`), and every one is SIGKILLed in a drop guard only while its
//! command line is still this test's. The journal is written by hand, as `run.json` is
//! saved, then both are loaded through `journal::load_all`, as restore loads them.

use crate::support;

#[allow(dead_code)]
// Each binary mounted its own copy before the merge (milestone 9.12, decision 13).
#[allow(clippy::duplicate_mod)]
#[path = "run_journal/fixture.rs"]
mod fixture;

use daemon::run::engine::OpResult;
use daemon::run::journal::{JOURNAL_FILE, JournalLine, append, load_all, save_run};
use daemon::run::model::{OpId, Run};
use daemon::run::reconcile::{Reconciliation, reconcile_with_orphan_parent};
use fixture::{create_window, pend, plain_run, round, run_ref};
use proto::AgentRole;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use support::run_git::{T, real_git};

/// The op that launched the round, and the window the journal says it made.
const LAUNCH_OP: OpId = 4;
const WINDOW_ID: u32 = 9;

/// A session id no other process on the machine carries.
fn unique_id(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{tag}-{}-{nanos:x}", std::process::id())
}

/// `ps -o <field>= -p <pid>` for one process the test started, trimmed; `None` once it
/// is gone.
fn ps_field(pid: u32, field: &str) -> Option<String> {
    let out = Command::new("ps")
        .args(["-ww", "-o", &format!("{field}="), "-p", &pid.to_string()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// Whether `pid` still runs with `argv0` (a test-unique string) as its command line's
/// start: the test's own process, not one that reused its pid.
fn still_ours(pid: u32, argv0: &str) -> bool {
    ps_field(pid, "args").is_some_and(|args| args.starts_with(argv0))
}

fn wait_until(what: &str, within: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            eprintln!("gave up waiting for {what}");
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A `sleep 120` the test starts as a daemon's session is left after a crash, made as
/// `run_journal/sessions.rs`'s `Orphan` makes one: orphaned (its parent, a `bash`,
/// exits at once), with `argv0` as its `argv[0]`, leading its own process group
/// (`set -m`) as every headless session does. Dropping it SIGKILLs it only while its
/// command line is still this test's.
struct Orphan {
    pid: u32,
    argv0: String,
}

impl Orphan {
    fn start(argv0: &str) -> Self {
        let out = Command::new("bash")
            .args([
                "-c",
                "set -m; (exec -a \"$0\" sleep 120) </dev/null >/dev/null 2>&1 & echo $!",
                argv0,
            ])
            .process_group(0)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let pid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
        let orphan = Orphan {
            pid,
            argv0: argv0.to_string(),
        };
        assert!(
            wait_until("the orphan's argv", Duration::from_secs(10), || still_ours(
                pid, argv0
            )),
            "the orphan never showed its argv"
        );
        orphan
    }

    /// Its parent once the spawning `bash` has exited: 1 where orphans go to init or
    /// launchd, a subreaper's pid elsewhere.
    fn parent(&self) -> u32 {
        let mut ppid = 0;
        let reparented = wait_until("the orphan's reparenting", Duration::from_secs(10), || {
            ppid = ps_field(self.pid, "ppid")
                .and_then(|p| p.parse().ok())
                .unwrap_or(0);
            ppid != 0 && ppid != std::process::id()
        });
        assert!(reparented, "the orphan was never reparented (ppid {ppid})");
        ppid
    }

    fn alive(&self) -> bool {
        still_ours(self.pid, &self.argv0)
    }
}

impl Drop for Orphan {
    fn drop(&mut self) {
        if self.alive() {
            // SAFETY: the test's own orphan, its command line just checked.
            unsafe {
                libc::kill(self.pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
}

/// A run whose task t1 has one worker round launched by op [`LAUNCH_OP`], a pending
/// `CreateWindow` with `session_uuid` `id`, and the round's own `pid` as given.
fn run_with_round(data: &Path, id: &str, pid: Option<u32>, ended: bool) -> Run {
    let mut run = plain_run(data);
    let kind = create_window(
        Path::new("/tmp/t1"),
        run_ref("t1", AgentRole::Worker, 1),
        Some(id),
    );
    pend(&mut run, LAUNCH_OP, Some("t1"), kind);
    let mut worker = round(AgentRole::Worker, 1, LAUNCH_OP, None, pid);
    worker.ended = ended;
    run.tasks[0].rounds.push(worker);
    run
}

/// Saves `run`, journals its launch's intent, then writes the launch's `Done` line by
/// hand in decision 43's shape (`{"op":<id>,"done":{"Window":{…}}}`), the line
/// M9.11.13 appends once a session's spawn returns; loads both back as restore does.
fn saved_with_done(run: &Run, journal_pid: u32) -> (Run, Vec<JournalLine>) {
    save_run(run).unwrap();
    let dir = &run.data_dir;
    let kind = run.pending_ops[&LAUNCH_OP].kind.clone();
    append(
        dir,
        &JournalLine::Intent {
            op: LAUNCH_OP,
            kind,
        },
    )
    .unwrap();
    let line = format!(
        r#"{{"op":{LAUNCH_OP},"done":{{"Window":{{"window_id":{WINDOW_ID},"pid":{journal_pid}}}}}}}"#
    );
    // The writer's bytes, not only their meaning: `journal::append` serialises a line
    // with `serde_json`, exactly as here, so M9.11.13's `Done` is this text.
    let written = serde_json::to_string(&JournalLine::Done {
        op: LAUNCH_OP,
        result: OpResult::Window {
            window_id: WINDOW_ID,
            pid: Some(journal_pid),
        },
    })
    .unwrap();
    assert_eq!(written, line, "the journal writes the shape restore reads");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(JOURNAL_FILE))
        .unwrap();
    writeln!(file, "{line}").unwrap();
    drop(file);
    let data = run.data_dir.parent().unwrap().parent().unwrap();
    let (mut runs, problems) = load_all(data);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs.len(), 1);
    let (run, journal) = runs.pop().unwrap();
    assert!(
        journal.iter().any(|l| matches!(
            l,
            JournalLine::Done {
                op: LAUNCH_OP,
                result: OpResult::Window {
                    window_id: WINDOW_ID,
                    pid: Some(p),
                },
            } if *p == journal_pid
        )),
        "the hand-written line loads as the launch's Done: {journal:?}"
    );
    (run, journal)
}

fn restore(run: &Run, journal: &[JournalLine], parent: u32) -> Reconciliation {
    reconcile_with_orphan_parent(real_git(), run, journal, &[], T, parent)
}

#[test]
fn a_leftover_known_only_from_the_journal_is_stopped() {
    let data = tempfile::tempdir().unwrap();
    let id = unique_id("journal-only-session");
    let leftover = Orphan::start(&id);
    let parent = leftover.parent();
    if cfg!(target_os = "macos") {
        assert_eq!(parent, 1, "launchd adopts orphans");
    }
    let run = run_with_round(data.path(), &id, None, false);
    let (run, journal) = saved_with_done(&run, leftover.pid);

    let result = restore(&run, &journal, parent);
    assert!(
        wait_until("the kill", Duration::from_secs(10), || !leftover.alive()),
        "the leftover whose pid only the journal held was stopped: {:?}",
        result.notes
    );
    assert!(
        result
            .notes
            .iter()
            .any(|n| n.contains("killed a leftover process")
                && n.contains(&leftover.pid.to_string())
                && n.contains(&format!("of session {id}"))),
        "{:?}",
        result.notes
    );
}

#[test]
fn a_journal_pid_of_an_ended_round_is_left_alone() {
    let data = tempfile::tempdir().unwrap();
    let id = unique_id("ended-session");
    let orphan = Orphan::start(&id);
    let parent = orphan.parent();
    let run = run_with_round(data.path(), &id, None, true);
    let (run, journal) = saved_with_done(&run, orphan.pid);

    let result = restore(&run, &journal, parent);
    assert!(orphan.alive(), "never signalled: {:?}", result.notes);
    assert!(
        !result.notes.iter().any(|n| n.contains("killed")),
        "{:?}",
        result.notes
    );
}

/// Pinning, and the reused-pid case: the journal's pid now belongs to a process whose
/// argv does not hold the session id (here only inside a longer path, as another
/// program that took the pid over would have), so it is never signalled.
#[test]
fn a_journal_pid_whose_argv_lacks_the_session_is_left_alone() {
    let data = tempfile::tempdir().unwrap();
    let id = unique_id("reused-pid-session");
    let stranger = Orphan::start(&format!("/x/{id}.jsonl"));
    let parent = stranger.parent();
    let run = run_with_round(data.path(), &id, None, false);
    let (run, journal) = saved_with_done(&run, stranger.pid);

    let result = restore(&run, &journal, parent);
    assert!(stranger.alive(), "never signalled: {:?}", result.notes);
    assert!(
        !result.notes.iter().any(|n| n.contains("killed")),
        "{:?}",
        result.notes
    );
}

/// Pinning: a round that recorded its own pid is checked by that pid, whatever the
/// journal says; the journal's pid is not examined.
#[test]
fn a_recorded_pid_still_wins() {
    let data = tempfile::tempdir().unwrap();
    let id = unique_id("recorded-session");
    let recorded = Orphan::start(&id);
    let journaled = Orphan::start(&id);
    let parent = recorded.parent();
    assert_eq!(journaled.parent(), parent);
    let run = run_with_round(data.path(), &id, Some(recorded.pid), false);
    let (run, journal) = saved_with_done(&run, journaled.pid);

    let result = restore(&run, &journal, parent);
    assert!(
        wait_until("the kill", Duration::from_secs(10), || !recorded.alive()),
        "the recorded pid was stopped: {:?}",
        result.notes
    );
    assert!(
        journaled.alive(),
        "the journal's pid was never signalled: {:?}",
        result.notes
    );
    assert!(
        !result
            .notes
            .iter()
            .any(|n| n.contains(&journaled.pid.to_string())),
        "{:?}",
        result.notes
    );
}
