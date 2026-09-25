//! M8a.18 fix round 1: control that arrives while a send or resume is between the old
//! process and the new one (ruling T18-I1), refused messages, and the three paths the
//! first round left untested (the Codex busy check, the kill on a resume that never
//! starts, and the Codex turn jitter).

use super::support::headless::*;
use super::{
    CLAUDE_UUID, CODEX_THREAD, Cleanup, argvs, claude_recorder, codex_recorder, gone, is_exit,
    is_turn_end, record_argv,
};
use proto::{Role, Runtime};
use std::time::{Duration, Instant};

/// Lets a task spawned on this current-thread runtime run until its first real wait:
/// a send or resume claims the window before it awaits anything.
async fn let_it_start() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

fn user_turns(m: &daemon::manager::WindowManager, id: u32) -> usize {
    m.conversation_snapshot(id, None)
        .expect("a conversation")
        .turns
        .iter()
        .filter(|t| t.role == Role::User)
        .count()
}

/// The reviewer's probe: a kill that lands while a resume waits out its jitter (the old
/// process already gone, the new one not yet started) must stop the resume. Before the
/// fix the kill acted on an ended handle and the resumed process ran anyway.
#[tokio::test]
async fn a_kill_during_a_resume_gap_stops_the_resume() {
    let dir = tempfile::tempdir().unwrap();
    let claude = claude_recorder(dir.path(), false);
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    let id = info.id;
    next_signal(&mut feed, "the first turn's end", is_turn_end).await;

    let resuming = m.clone();
    let resume = tokio::spawn(async move {
        resuming
            .headless_resume(id, CLAUDE_UUID, "go on", Duration::from_secs(60))
            .await
    });
    wait_until("the first process to be retired", || {
        m.child_pid(id).unwrap().is_none()
    })
    .await;
    m.headless_kill(id).unwrap();
    let error = tokio::time::timeout(DEADLINE, resume)
        .await
        .expect("the kill ends the resume's wait")
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("was killed"), "{error}");
    assert_eq!(m.child_pid(id).unwrap(), None, "no resumed process runs");
    assert_eq!(argvs(dir.path()).len(), 1, "no second process was started");
}

/// A kill that lands after the resume recorded its turn and started its process, but
/// before that process is installed: the process is killed at once and never becomes
/// the window's.
#[tokio::test]
async fn a_kill_before_the_install_kills_the_new_process() {
    let dir = tempfile::tempdir().unwrap();
    let claude = claude_recorder(dir.path(), false);
    let m = manager(&claude, &claude, |c| {
        c.headless_install_pause = Duration::from_secs(1)
    });
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    let id = info.id;
    next_signal(&mut feed, "the first turn's end", is_turn_end).await;

    let resuming = m.clone();
    let resume = tokio::spawn(async move {
        resuming
            .headless_resume(id, CLAUDE_UUID, "go on", Duration::ZERO)
            .await
    });
    wait_until("the resumed process", || argvs(dir.path()).len() == 2).await;
    m.headless_kill(id).unwrap();
    let error = tokio::time::timeout(DEADLINE, resume)
        .await
        .expect("the resume ends")
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("was killed"), "{error}");
    let pid: u32 = std::fs::read_to_string(dir.path().join("live.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    wait_until("the new process to be killed", || gone(pid)).await;
    assert_eq!(m.child_pid(id).unwrap(), None);
}

/// An interrupt that lands while a Codex send waits out its jitter stops the turn
/// before its process starts.
#[tokio::test]
async fn an_interrupt_during_a_codex_send_gap_stops_the_turn() {
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |c| {
        c.codex_turn_jitter = Some(Duration::from_secs(60))
    });
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "c", spec(Runtime::Codex, dir.path()), "first").await;
    let id = info.id;
    next_signal(&mut feed, "the first exit", is_exit).await;

    let sending = m.clone();
    let send = tokio::spawn(async move { sending.headless_send(id, "second").await });
    let_it_start().await;
    m.headless_interrupt(id).unwrap();
    let error = tokio::time::timeout(DEADLINE, send)
        .await
        .expect("the interrupt ends the send's wait")
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("was interrupted"), "{error}");
    assert_eq!(argvs(dir.path()).len(), 1, "no second process was started");
    assert_eq!(user_turns(&m, id), 1, "no turn was recorded for it");
    // The window takes the next turn as usual.
    m.headless_resume(id, CODEX_THREAD, "again", Duration::ZERO)
        .await
        .unwrap();
}

/// Ruling T18-minors: a NUL byte cannot reach an argv; the message is refused before its
/// turn is recorded, so no phantom `User` turn is left behind.
#[tokio::test]
async fn a_message_with_a_nul_byte_is_refused_before_it_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "c", spec(Runtime::Codex, dir.path()), "first").await;
    next_signal(&mut feed, "the first exit", is_exit).await;
    for error in [
        m.headless_send(info.id, "a\0b").await.unwrap_err(),
        m.headless_resume(info.id, CODEX_THREAD, "a\0b", Duration::ZERO)
            .await
            .unwrap_err(),
    ] {
        assert_eq!(
            error.to_string(),
            format!("a message for window {} contains a NUL byte", info.id)
        );
    }
    assert_eq!(user_turns(&m, info.id), 1);
    assert_eq!(argvs(dir.path()).len(), 1);
    m.headless_send(info.id, "ok").await.unwrap();
    next_signal(&mut feed, "the second exit", is_exit).await;
}

/// Two sends at once on one Codex window: one runs, the other is refused, and the
/// session never has two processes.
#[tokio::test]
async fn two_codex_sends_at_once_start_one_turn() {
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "c", spec(Runtime::Codex, dir.path()), "first").await;
    next_signal(&mut feed, "the first exit", is_exit).await;
    let (a, b) = tokio::join!(m.headless_send(info.id, "A"), m.headless_send(info.id, "B"));
    assert!(a.is_ok() != b.is_ok(), "{a:?} / {b:?}");
    let refused = a.err().or(b.err()).unwrap().to_string();
    assert_eq!(
        refused,
        format!("a turn is already running for window {}", info.id)
    );
    next_signal(&mut feed, "the second exit", is_exit).await;
    assert_eq!(argvs(dir.path()).len(), 2);
    assert!(!dir.path().join("overlap.log").exists());
}

/// A Claude whose `--resume` launch never prints `Init`; with `ignore_term`, it also
/// ignores `SIGTERM`. Its pid goes to `hung.pid`.
fn hung_on_resume(dir: &std::path::Path, ignore_term: bool) -> std::path::PathBuf {
    let trap = if ignore_term { "trap '' TERM; " } else { "" };
    script(
        dir,
        "claude",
        &format!(
            r#"{record}
case " $* " in *" --resume "*) echo $$ > "$D/hung.pid"; {trap}while :; do sleep 0.05; done;; esac
while IFS= read -r line; do
  printf '{{"type":"system","subtype":"init","session_id":"{CLAUDE_UUID}","model":"m"}}\n'
  printf '{{"type":"result","subtype":"success","is_error":false}}\n'
done"#,
            record = record_argv(dir)
        ),
    )
}

/// A kill while a resume waits for its process's `Init` is reported as the kill, not
/// as a failed resume (which would start a fresh session, decision 28).
#[tokio::test]
async fn a_kill_during_the_init_wait_is_reported_as_the_kill() {
    let dir = tempfile::tempdir().unwrap();
    let claude = hung_on_resume(dir.path(), false);
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    let id = info.id;
    next_signal(&mut feed, "the first turn's end", is_turn_end).await;
    let resuming = m.clone();
    let resume = tokio::spawn(async move {
        resuming
            .headless_resume(id, CLAUDE_UUID, "go on", Duration::ZERO)
            .await
    });
    wait_until("the resumed process", || {
        dir.path().join("hung.pid").exists()
    })
    .await;
    wait_until("its install", || m.child_pid(id).unwrap().is_some()).await;
    m.headless_kill(id).unwrap();
    let error = tokio::time::timeout(DEADLINE, resume)
        .await
        .expect("the resume ends")
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("was killed"), "{error}");
}

/// A resumed process that never prints `Init` (and ignores `SIGTERM`) is killed when
/// the resume times out.
#[tokio::test]
async fn a_resume_that_never_starts_is_killed_at_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let claude = hung_on_resume(dir.path(), true);
    let m = manager(&claude, &claude, |c| {
        c.kill_grace = Duration::from_millis(300);
        c.resume_start_timeout = Duration::from_millis(500);
    });
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    next_signal(&mut feed, "the first turn's end", is_turn_end).await;
    let error = m
        .headless_resume(info.id, CLAUDE_UUID, "go on", Duration::ZERO)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("did not start within"), "{error}");
    let pid: u32 = std::fs::read_to_string(dir.path().join("hung.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    wait_until("the hung process to be killed", || gone(pid)).await;
}

/// A Codex turn's process starts only after the launch jitter. The jitter is injected
/// long, so the check is a lower bound on the send's own duration, never a race.
#[tokio::test]
async fn a_codex_send_waits_out_the_launch_jitter() {
    const JITTER: Duration = Duration::from_millis(1500);
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |c| c.codex_turn_jitter = Some(JITTER));
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "c", spec(Runtime::Codex, dir.path()), "first").await;
    next_signal(&mut feed, "the first exit", is_exit).await;
    let started = Instant::now();
    m.headless_send(info.id, "second").await.unwrap();
    assert!(started.elapsed() >= JITTER, "{:?}", started.elapsed());
    next_signal(&mut feed, "the second exit", is_exit).await;
}
