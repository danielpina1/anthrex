//! M8a.17: the headless session driver (`headless::session`) against real processes and
//! real pipes. The "agent" is a `/bin/sh -c` program until M8a.20's `fake-agent` modes.

use daemon::headless::argv::InterruptMode;
use daemon::headless::claude_stream::ClaudeStream;
use daemon::headless::session::{HeadlessHandle, STDOUT_LINE_MAX, WRITER_QUEUE_MAX};
use daemon::headless::{FailureKind, SessionEvent, TurnOutcome};
use proto::Runtime;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Generous: every wait here ends on an event a live child produces within
/// milliseconds; the bound only turns a hang into a failure.
const DEADLINE: Duration = Duration::from_secs(10);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/headless")
        .join(name)
}

/// Every `(pid, event)` a session reported, in delivery order.
#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<(u32, SessionEvent)>>>);

impl Events {
    fn sink(&self) -> impl Fn(u32, SessionEvent) + Send + Sync + 'static {
        let events = self.0.clone();
        move |pid, event| events.lock().unwrap().push((pid, event))
    }

    fn all(&self) -> Vec<(u32, SessionEvent)> {
        self.0.lock().unwrap().clone()
    }

    fn wait_for(&self, what: &str, done: impl Fn(&[(u32, SessionEvent)]) -> bool) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if done(&self.all()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {:?}",
                self.all()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_exit(&self) {
        self.wait_for("the process exit", |events| {
            events
                .iter()
                .any(|(_, e)| matches!(e, SessionEvent::ProcessExited { .. }))
        });
    }

    fn wait_unknown(&self, line: &str) {
        self.wait_for(line, |events| {
            events
                .iter()
                .any(|(_, e)| matches!(e, SessionEvent::Unknown { line: l } if l == line))
        });
    }
}

fn sh(script: &str, runtime: Runtime, events: &Events) -> HeadlessHandle {
    HeadlessHandle::spawn(
        runtime,
        OsStr::new("/bin/sh"),
        &["-c".to_string(), script.to_string()],
        Path::new("/"),
        &[],
        events.sink(),
    )
    .expect("spawn /bin/sh")
}

/// Sends `SIGKILL` to a test's process group when dropped, even by a failed assertion.
struct KillGroupOnDrop(u32);

impl Drop for KillGroupOnDrop {
    fn drop(&mut self) {
        // SAFETY: a plain signal to the test's own child's group.
        unsafe { libc::killpg(self.0 as libc::pid_t, libc::SIGKILL) };
    }
}

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the pid exists.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[test]
fn spawn_reads_events_in_order_and_reports_exit() {
    let path = fixture("claude-2.1.278-stream.jsonl");
    let events = Events::default();
    let handle = sh(
        &format!("cat '{}'; exit 3", path.display()),
        Runtime::Claude,
        &events,
    );
    let pid = handle.spawned_pid().expect("a spawned handle has a pid");
    events.wait_exit();

    let mut parser = ClaudeStream::default();
    let mut expected = vec![SessionEvent::ProcessStarted { pid }];
    for line in std::fs::read_to_string(&path).unwrap().lines() {
        if !line.trim().is_empty() {
            expected.extend(parser.parse_line(line));
        }
    }
    expected.push(SessionEvent::ProcessExited {
        code: Some(3),
        signal: None,
    });
    let got = events.all();
    assert!(
        got.iter().all(|(p, _)| *p == pid),
        "every event carries its pid"
    );
    let got: Vec<SessionEvent> = got.into_iter().map(|(_, e)| e).collect();
    assert_eq!(got, expected);
    assert_eq!(handle.pid(), None, "a reaped process has no signalable pid");
}

#[test]
fn send_line_never_blocks_on_a_full_pipe() {
    let events = Events::default();
    let handle = sh("exec sleep 30", Runtime::Claude, &events);
    let sender = handle.clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let line = "x".repeat(8 * 1024);
        let results: Vec<bool> = (0..300)
            .map(|_| sender.send_line(line.clone()).is_ok())
            .collect();
        let _ = done_tx.send(results);
    });
    let results = done_rx
        .recv_timeout(DEADLINE)
        .expect("300 send_line calls returned; none blocked on the pipe");
    assert_eq!(results.len(), 300);
    assert!(
        results.iter().any(|ok| !ok),
        "{WRITER_QUEUE_MAX} queued lines plus a pipe's worth cannot hold 300 of 8 KiB"
    );
    assert!(results[0], "the first line is accepted");
    handle.kill(Duration::from_secs(1));
    events.wait_exit();
}

#[test]
fn interrupt_by_sigint_reaches_the_child() {
    let events = Events::default();
    let handle = sh(
        "trap 'echo interrupted; exit 0' INT; echo ready; while :; do sleep 0.05; done",
        Runtime::Claude,
        &events,
    );
    // Before `ready` the trap may not be installed, and SIGINT's default would kill it.
    events.wait_unknown("ready");
    handle.interrupt(InterruptMode::Sigint, 1).unwrap();
    events.wait_unknown("interrupted");
    events.wait_exit();
}

#[test]
fn kill_terminates_the_process_group() {
    let events = Events::default();
    let handle = sh("sleep 30 & echo $!; wait", Runtime::Claude, &events);
    events.wait_for("the background pid", |events| {
        events
            .iter()
            .any(|(_, e)| matches!(e, SessionEvent::Unknown { .. }))
    });
    let background: u32 = events
        .all()
        .iter()
        .find_map(|(_, e)| match e {
            SessionEvent::Unknown { line } => line.parse().ok(),
            _ => None,
        })
        .expect("the child printed its background pid");
    let leader = handle.spawned_pid().unwrap();
    assert!(alive(leader) && alive(background));

    handle.kill(Duration::from_secs(1));
    let deadline = Instant::now() + Duration::from_secs(2);
    while (alive(leader) || alive(background)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!alive(leader), "the leader {leader} is gone within 2 s");
    assert!(
        !alive(background),
        "the background {background} is gone within 2 s"
    );
    events.wait_exit();
}

/// The `SIGTERM` step alone reaches the whole group: a leader that traps it keeps
/// running through a long grace, while its background child is gone at once.
#[test]
fn kill_sends_sigterm_to_the_whole_group_first() {
    let events = Events::default();
    let handle = sh(
        "trap 'echo term' TERM; sleep 30 & echo $!; while :; do sleep 0.05; done",
        Runtime::Claude,
        &events,
    );
    events.wait_for("the background pid", |events| {
        events
            .iter()
            .any(|(_, e)| matches!(e, SessionEvent::Unknown { .. }))
    });
    let background: u32 = events
        .all()
        .iter()
        .find_map(|(_, e)| match e {
            SessionEvent::Unknown { line } => line.parse().ok(),
            _ => None,
        })
        .unwrap();
    let leader = handle.spawned_pid().unwrap();
    // The leader traps TERM and loops forever: a failing assertion below must not leave
    // it running under PID 1.
    let _cleanup = KillGroupOnDrop(leader);
    handle.kill(Duration::from_secs(60));
    events.wait_unknown("term");
    let deadline = Instant::now() + Duration::from_secs(2);
    while alive(background) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !alive(background),
        "the group's SIGTERM reached {background}"
    );
    assert!(
        alive(leader),
        "the leader trapped it and waits out the grace"
    );
    handle.kill(Duration::ZERO);
    events.wait_exit();
    assert!(!alive(leader));
}

#[test]
fn a_long_line_is_cut_and_parsed_as_unknown() {
    let path = fixture("codex-0.155.0-exec.jsonl");
    let first = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let dir = tempfile::tempdir().unwrap();
    let next = dir.path().join("next.jsonl");
    std::fs::write(&next, format!("{first}\n")).unwrap();
    let long = STDOUT_LINE_MAX + 1024;
    let events = Events::default();
    let handle = sh(
        &format!(
            "head -c {long} /dev/zero | tr '\\0' a; echo; cat '{}'",
            next.display()
        ),
        Runtime::Codex,
        &events,
    );
    let pid = handle.spawned_pid().unwrap();
    events.wait_exit();

    let mut expected = vec![
        SessionEvent::ProcessStarted { pid },
        SessionEvent::Unknown {
            line: "a".repeat(daemon::headless::UNKNOWN_LINE_CHARS),
        },
    ];
    expected.extend(daemon::headless::codex_stream::parse_line(&first));
    expected.push(SessionEvent::ProcessExited {
        code: Some(0),
        signal: None,
    });
    let got: Vec<SessionEvent> = events.all().into_iter().map(|(_, e)| e).collect();
    assert_eq!(got, expected);
}

/// M8a.12's carry: Claude refuses to start without its sandbox, on stderr and before
/// `system/init` (M8a.1 item 4b). The driver reports that as a failed turn of kind
/// `SandboxUnavailable` before the exit, so the engine can block the task on it.
#[test]
fn a_sandbox_failure_before_init_is_a_failed_turn_before_the_exit() {
    let text = "Error: sandbox required but unavailable: bubblewrap is not installed";
    let events = Events::default();
    sh(
        &format!("echo '{text}' >&2; exit 1"),
        Runtime::Claude,
        &events,
    );
    events.wait_exit();
    let got: Vec<SessionEvent> = events.all().into_iter().map(|(_, e)| e).collect();
    let tail = &got[got.len() - 3..];
    assert_eq!(
        tail,
        [
            SessionEvent::StderrLine { line: text.into() },
            SessionEvent::TurnEnded {
                outcome: TurnOutcome::Failed {
                    error: text.into(),
                    kind: FailureKind::SandboxUnavailable,
                },
                usage: None,
                denials: Vec::new(),
            },
            SessionEvent::ProcessExited {
                code: Some(1),
                signal: None,
            },
        ]
    );
}

/// The same text after `Init` is an ordinary stderr line: the session did start.
#[test]
fn a_sandbox_text_after_init_is_only_a_stderr_line() {
    let path = fixture("claude-2.1.278-stream.jsonl");
    let init = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .find(|l| l.contains("\"subtype\":\"init\""))
        .unwrap()
        .to_string();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("init.jsonl");
    std::fs::write(&file, format!("{init}\n")).unwrap();
    let events = Events::default();
    sh(
        &format!(
            "cat '{}'; sleep 0.2; echo 'sandbox required but unavailable: x' >&2; exit 1",
            file.display()
        ),
        Runtime::Claude,
        &events,
    );
    events.wait_exit();
    assert!(
        !events
            .all()
            .iter()
            .any(|(_, e)| matches!(e, SessionEvent::TurnEnded { .. })),
        "{:?}",
        events.all()
    );
}

#[test]
fn an_ended_handle_refuses_everything() {
    let handle = HeadlessHandle::ended();
    assert_eq!(handle.pid(), None);
    assert!(handle.send_line("x".into()).is_err());
    assert!(handle.interrupt(InterruptMode::Sigint, 1).is_err());
    handle.kill(Duration::from_secs(1));
    handle.close_stdin();
}

/// A leader that exits normally takes its group with it: whatever it left running in the
/// background (an `anthrex mcp` server, a hook) is killed before the exit is reported.
#[test]
fn a_leader_that_exits_takes_its_background_children_with_it() {
    let events = Events::default();
    let handle = sh("sleep 30 & echo $!; exit 0", Runtime::Claude, &events);
    let _cleanup = KillGroupOnDrop(handle.spawned_pid().unwrap());
    events.wait_exit();
    let background: u32 = events
        .all()
        .iter()
        .find_map(|(_, e)| match e {
            SessionEvent::Unknown { line } => line.parse().ok(),
            _ => None,
        })
        .expect("the child printed its background pid");
    // Killed before the exit was reported; only its reaping by init may lag.
    let deadline = Instant::now() + Duration::from_secs(2);
    while alive(background) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !alive(background),
        "the background {background} outlived its leader"
    );
}

/// Once the process is reaped, a send fails at once, even while the writer thread has
/// not yet noticed (review M2: the first send after the exit used to return `Ok`).
#[test]
fn send_line_fails_once_the_process_has_exited() {
    let events = Events::default();
    let handle = sh("exit 0", Runtime::Claude, &events);
    events.wait_exit();
    assert!(handle.send_line("late".into()).is_err());
}
