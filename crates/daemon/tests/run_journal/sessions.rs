//! Decision 44's reconcile of sessions: a `CreateWindow` whose window the manager
//! restored, and decision 28's leftover session processes, found by the session id on
//! their command line. Every process signalled here is one this test started.

use crate::fixture::{create_window, intents, pend, plain_run, round, run_ref, window};
use crate::support::run_git::{T, real_git};
use daemon::run::engine::{OpKind, OpResult};
use daemon::run::reconcile::{Reconciled, reconcile};
use proto::{AgentRole, WindowKind};
use std::os::unix::process::{CommandExt, ExitStatusExt};
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

/// Waits (with a deadline) until `ps` shows `needle` on some command line, so the
/// sleep has exec'd with its `argv[0]` before reconcile looks.
fn visible(needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let out = Command::new("ps")
            .args(["-ww", "-A", "-o", "args="])
            .output()
            .unwrap();
        if String::from_utf8_lossy(&out.stdout).contains(needle) {
            return;
        }
        assert!(Instant::now() < deadline, "{needle} never appeared in ps");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn reconcile_kills_a_leftover_session_process_by_its_session_id() {
    let data = tempfile::tempdir().unwrap();
    let mut run = plain_run(data.path());
    let leftover_id = unique_id("leftover-session");
    let other_id = unique_id("other-session");
    let mut leftover = Sleeper::start(&format!("claude --resume {leftover_id}"));
    let mut bystander = Sleeper::start("sleep");
    visible(&leftover_id);
    // t1's worker was the leftover's session; t2's worker recorded the bystander's pid
    // (reused by the OS, say) under a session id its command line does not carry.
    run.tasks[0].rounds.push(round(
        AgentRole::Worker,
        1,
        1,
        Some(&leftover_id),
        Some(leftover.pid()),
    ));
    run.tasks[1].rounds.push(round(
        AgentRole::Worker,
        1,
        2,
        Some(&other_id),
        Some(bystander.pid()),
    ));

    let result = reconcile(real_git(), &run, &[], &[], T);
    assert!(result.ops.is_empty());

    let ended = leftover
        .wait_for_end(Duration::from_secs(10))
        .expect("the leftover session process was killed");
    assert_eq!(ended.signal(), Some(libc::SIGTERM), "{ended:?}");
    assert!(
        result
            .notes
            .iter()
            .any(|n| n.contains(&leftover_id) && n.contains(&leftover.pid().to_string())),
        "{:?}",
        result.notes
    );
    assert!(
        bystander.wait_for_end(Duration::from_millis(200)).is_none(),
        "a process without the session id is never signalled"
    );
    drop(bystander);
}
