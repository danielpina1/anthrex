//! Decision 44's reconcile of sessions: a `CreateWindow` whose window the manager
//! restored, and decision 28's leftover session processes, found by the session id on
//! their command line. Every process signalled here is one this test started, and only
//! the recorded pid of a round is ever examined (fix round 1, T21-I1).

use crate::fixture::{create_window, intents, pend, plain_run, round, run_ref, window};
use crate::support::run_git::{T, real_git};
use daemon::run::engine::{OpKind, OpResult};
use daemon::run::reconcile::{
    ORPHAN_PARENT, Reconciled, Reconciliation, reconcile, reconcile_with_orphan_parent,
};
use proto::{AgentRole, WindowKind};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A session id no other process on the machine carries.
fn unique_id(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{tag}-{}-{nanos:x}", std::process::id())
}

#[test]
fn reconcile_create_window_finds_the_restored_window() {
    let data = tempfile::tempdir().unwrap();
    let mut run = plain_run(data.path());
    let cwd = Path::new("/tmp/t1");
    // Session 2 of t1's worker was being launched and registered twice (4, then 9, the
    // latest); session 1's window is the one before it; a PTY window and another run's
    // window carry the same reference; t2's reviewer launch never registered a window.
    let wanted = run_ref("t1", AgentRole::Worker, 2);
    let kind = create_window(cwd, wanted.clone(), Some(&unique_id("uuid-a")));
    pend(&mut run, 5, Some("t1"), kind);
    let kind = create_window(
        cwd,
        run_ref("t2", AgentRole::Reviewer, 1),
        Some(&unique_id("uuid-b")),
    );
    pend(&mut run, 6, Some("t2"), kind);
    let resume = OpKind::ResumeSession {
        window_id: 3,
        session_id: unique_id("resumed"),
        message: "carry on".into(),
        jitter_ms: 0,
    };
    pend(&mut run, 8, Some("t1"), resume);
    let mut elsewhere = run_ref("t1", AgentRole::Worker, 2);
    elsewhere.run_id = "another-run-0000".into();
    let windows = vec![
        window(
            3,
            Some(run_ref("t1", AgentRole::Worker, 1)),
            WindowKind::Headless,
        ),
        window(12, Some(wanted.clone()), WindowKind::Pty),
        window(11, Some(elsewhere), WindowKind::Headless),
        window(4, Some(wanted.clone()), WindowKind::Headless),
        window(9, Some(wanted), WindowKind::Headless),
        window(10, None, WindowKind::Headless),
    ];

    let journal = intents(&run);
    let got = reconcile(real_git(), &run, &journal, &windows, T).ops;
    assert_eq!(
        got,
        vec![
            (5, Reconciled::Replay(OpResult::Window { window_id: 9 })),
            (6, Reconciled::NotStarted),
            (8, Reconciled::NotStarted),
        ]
    );
}

/// A `sleep` the test owns, in its own process group with `argv[0]` set, so a group
/// signal can never reach the test runner. The test holds the `Child` unreaped until
/// it has seen it end, so its pid cannot be reused by anything else meanwhile, and
/// dropping it kills only a sleep that is still this test's.
struct Sleeper {
    child: Child,
}

impl Sleeper {
    fn start(argv0: &str) -> Self {
        let child = Command::new("sleep")
            .arg0(argv0)
            .arg("120")
            .process_group(0)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        Sleeper { child }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    /// How it ended, if it does within `within` (a deadline loop; reaps it).
    fn wait_for_end(&mut self, within: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + within;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        // Still unreaped, so still ours: the test's own clean-up.
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
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

/// A `sleep 120` the test starts as a daemon's session would be left after a crash:
/// orphaned (its parent, a `bash`, exits at once, so it is reparented), with `argv0` as
/// its `argv[0]`, and, with `own_group`, leading its own process group (`set -m`), as
/// every headless session does. Dropping it SIGKILLs it only while its command line is
/// still this test's.
struct Orphan {
    pid: u32,
    argv0: String,
}

impl Orphan {
    fn start(argv0: &str, own_group: bool) -> Self {
        let job = if own_group { "set -m; " } else { "" };
        let out = Command::new("bash")
            .args([
                "-c",
                &format!("{job}(exec -a \"$0\" sleep 120) </dev/null >/dev/null 2>&1 & echo $!"),
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
        wait_until("the orphan's reparenting", Duration::from_secs(10), || {
            ppid = ps_field(self.pid, "ppid")
                .and_then(|p| p.parse().ok())
                .unwrap_or(0);
            ppid != 0 && ppid != std::process::id()
        });
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

/// Reconcile as the daemon runs it when the orphan's parent is the usual 1; through the
/// injectable orphan parent where the platform reparents to a subreaper instead.
fn reconcile_orphans(run: &daemon::run::model::Run, parent: u32) -> Reconciliation {
    if parent == ORPHAN_PARENT {
        reconcile(real_git(), run, &[], &[], T)
    } else {
        reconcile_with_orphan_parent(real_git(), run, &[], &[], T, parent)
    }
}

/// Fix round 1, T21-I1: decision 28's leftover check, on the recorded pid only.
#[test]
fn reconcile_kills_an_orphaned_session_process_by_its_recorded_pid() {
    let data = tempfile::tempdir().unwrap();
    let mut run = plain_run(data.path());
    let id = unique_id("leftover-session");
    let leftover = Orphan::start(&id, true);
    let parent = leftover.parent();
    if cfg!(target_os = "macos") {
        assert_eq!(parent, 1, "launchd adopts orphans");
    }
    run.tasks[0].rounds.push(round(
        AgentRole::Worker,
        1,
        1,
        Some(&id),
        Some(leftover.pid),
    ));

    let result = reconcile_orphans(&run, parent);
    assert!(result.ops.is_empty());
    assert!(
        wait_until("the kill", Duration::from_secs(10), || !leftover.alive()),
        "the orphaned session process was killed"
    );
    assert!(
        result.notes.iter().any(|n| n.contains(&id)
            && n.contains(&leftover.pid.to_string())
            && n.contains("group")),
        "{:?}",
        result.notes
    );
}

/// T21-I1: a process that carries the session id but whose pid no round recorded is
/// never looked for, let alone signalled.
#[test]
fn reconcile_leaves_an_unrecorded_process_with_the_session_id_alone() {
    let data = tempfile::tempdir().unwrap();
    let mut run = plain_run(data.path());
    let id = unique_id("unrecorded-session");
    let orphan = Orphan::start(&id, true);
    let parent = orphan.parent();
    run.tasks[0]
        .rounds
        .push(round(AgentRole::Worker, 1, 1, Some(&id), None));
    let kind = create_window(
        Path::new("/tmp/t1"),
        run_ref("t1", AgentRole::Worker, 1),
        Some(&id),
    );
    pend(&mut run, 1, Some("t1"), kind);

    let result = reconcile_orphans(&run, parent);
    assert!(orphan.alive(), "never signalled: {:?}", result.notes);
}

/// T21-I1: a recorded pid whose argv does not hold the session id as a whole element
/// (here only inside a longer path, as a `tail` of the transcript would) is left alone.
#[test]
fn reconcile_leaves_a_recorded_pid_without_the_session_id_alone() {
    let data = tempfile::tempdir().unwrap();
    let mut run = plain_run(data.path());
    let id = unique_id("mentioned-session");
    let orphan = Orphan::start(&format!("/x/{id}.jsonl"), true);
    let parent = orphan.parent();
    run.tasks[0]
        .rounds
        .push(round(AgentRole::Worker, 1, 1, Some(&id), Some(orphan.pid)));

    let result = reconcile_orphans(&run, parent);
    assert!(orphan.alive(), "never signalled: {:?}", result.notes);
}

/// T21-I1: a recorded pid with the session id whose parent is alive (not orphaned by a
/// dead daemon: here the test itself) is left alone.
#[test]
fn reconcile_leaves_a_session_process_that_is_not_orphaned_alone() {
    let data = tempfile::tempdir().unwrap();
    let mut run = plain_run(data.path());
    let id = unique_id("parented-session");
    let mut child = Sleeper::start(&id);
    assert!(wait_until(
        "the child's argv",
        Duration::from_secs(10),
        || { still_ours(child.pid(), &id) }
    ));
    run.tasks[0]
        .rounds
        .push(round(AgentRole::Worker, 1, 1, Some(&id), Some(child.pid())));

    let result = reconcile(real_git(), &run, &[], &[], T);
    assert!(
        child.wait_for_end(Duration::from_millis(300)).is_none(),
        "never signalled: {:?}",
        result.notes
    );
}
