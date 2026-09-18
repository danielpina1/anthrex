use daemon::launch::LaunchPlan;
use daemon::window::{Window, WindowEvent};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

fn plan(program: &str, args: &[&str]) -> LaunchPlan {
    LaunchPlan {
        program: program.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        cwd: std::env::temp_dir(),
        env: vec![("TERM".to_string(), "xterm-256color".to_string())],
    }
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_event(
    rx: &mut mpsc::UnboundedReceiver<(u32, WindowEvent)>,
    pred: impl Fn(&WindowEvent) -> bool,
) -> WindowEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (_, ev) = rx.recv().await.expect("event channel closed");
            if pred(&ev) {
                return ev;
            }
        }
    })
    .await
    .expect("timed out waiting for event")
}

#[tokio::test]
async fn shows_output_accepts_input_and_reports_exit_code() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let w = Window::spawn(
        1,
        &plan("sh", &["-c", "echo hello-anthrex; cat"]),
        80,
        24,
        tx,
    )
    .unwrap();
    wait_until("greeting", || w.screen_text().contains("hello-anthrex")).await;
    w.write_input(b"ping-pong\n").unwrap();
    wait_until("echoed input", || w.screen_text().contains("ping-pong")).await;
    w.write_input(&[0x04]).unwrap(); // Ctrl-D ends `cat`
    let ev = wait_event(&mut rx, |e| matches!(e, WindowEvent::Exited { .. })).await;
    assert_eq!(
        ev,
        WindowEvent::Exited {
            code: Some(0),
            signal: None
        }
    );
}

#[tokio::test]
async fn nonzero_exit_code_is_reported() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _w = Window::spawn(2, &plan("sh", &["-c", "exit 3"]), 80, 24, tx).unwrap();
    let ev = wait_event(&mut rx, |e| matches!(e, WindowEvent::Exited { .. })).await;
    assert_eq!(
        ev,
        WindowEvent::Exited {
            code: Some(3),
            signal: None
        }
    );
}

#[tokio::test]
async fn missing_binary_shows_error_in_window_and_exits_127() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let w = Window::spawn(3, &plan("definitely-not-a-binary-anthrex", &[]), 80, 24, tx).unwrap();
    let ev = wait_event(&mut rx, |e| matches!(e, WindowEvent::Exited { .. })).await;
    assert_eq!(
        ev,
        WindowEvent::Exited {
            code: Some(127),
            signal: None
        }
    );
    assert!(
        w.screen_text().contains("not found"),
        "screen: {:?}",
        w.screen_text()
    );
}

#[tokio::test]
async fn resize_changes_the_size_the_child_sees() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let w = Window::spawn(4, &plan("sh", &["-c", "sleep 0.5; stty size"]), 80, 24, tx).unwrap();
    w.resize(100, 30).unwrap();
    assert_eq!(w.size(), (100, 30));
    wait_until("stty output", || w.screen_text().contains("30 100")).await;
}

#[tokio::test]
async fn bell_and_title_are_reported_as_events() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _w = Window::spawn(
        5,
        &plan(
            "sh",
            &[
                "-c",
                "printf '\\033]0;hello-title\\007'; printf '\\007'; sleep 0.2",
            ],
        ),
        80,
        24,
        tx,
    )
    .unwrap();
    let title = wait_event(&mut rx, |e| matches!(e, WindowEvent::Title(_))).await;
    assert_eq!(title, WindowEvent::Title("hello-title".into()));
    wait_event(&mut rx, |e| *e == WindowEvent::Bell).await;
}

#[tokio::test]
async fn signal_terminates_the_child() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let w = Window::spawn(6, &plan("sleep", &["30"]), 80, 24, tx).unwrap();
    w.signal(libc::SIGTERM).unwrap();
    let ev = wait_event(&mut rx, |e| matches!(e, WindowEvent::Exited { .. })).await;
    match ev {
        // On macOS, portable-pty derives the signal string from libc::strsignal(15),
        // which is "Terminated: 15" rather than a string literally containing "TERM".
        // Match case-insensitively so this still verifies it was a TERM signal.
        WindowEvent::Exited {
            code: None,
            signal: Some(s),
        } => {
            assert!(s.to_uppercase().contains("TERM"), "{s}")
        }
        other => panic!("expected signal exit, got {other:?}"),
    }
}

/// I2: a client attaching to a window whose program is on the alternate screen (vim,
/// less, htop, codex) must land on the alternate screen too.
#[tokio::test]
async fn snapshot_carries_the_alternate_screen_state() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let w = Window::spawn(
        8,
        &plan(
            "sh",
            &["-c", "printf '\\033[?1049h'; printf 'ALT'; sleep 2"],
        ),
        80,
        24,
        tx,
    )
    .unwrap();
    wait_until("alternate screen text", || w.screen_text().contains("ALT")).await;

    let att = w.attach();
    let mut mirror = vt100::Parser::new(att.rows, att.cols, 0);
    mirror.process(&att.snapshot);
    assert!(
        mirror.screen().alternate_screen(),
        "the snapshot did not restore the alternate screen"
    );
    assert!(
        mirror.screen().contents().contains("ALT"),
        "screen: {:?}",
        mirror.screen().contents()
    );
    // The screen must appear exactly once: `state_formatted()` already includes the
    // contents, so emitting `contents_formatted()` as well would write it twice.
    assert_eq!(mirror.screen().contents().matches("ALT").count(), 1);
    let _ = w.signal(libc::SIGKILL);
}

#[tokio::test]
async fn attach_gives_a_snapshot_and_live_output_without_duplicates() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let w = Window::spawn(
        7,
        &plan(
            "sh",
            &["-c", "echo first; sleep 0.3; echo second; sleep 0.3"],
        ),
        80,
        24,
        tx,
    )
    .unwrap();
    wait_until("first line", || w.screen_text().contains("first")).await;
    let mut att = w.attach();
    assert_eq!((att.cols, att.rows), (80, 24));
    let mut mirror = vt100::Parser::new(att.rows, att.cols, 0);
    mirror.process(&att.snapshot);
    assert!(mirror.screen().contents().contains("first"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !mirror.screen().contents().contains("second") {
        assert!(Instant::now() < deadline, "never saw second line");
        if let Ok(Ok(chunk)) =
            tokio::time::timeout(Duration::from_millis(200), att.output.recv()).await
        {
            mirror.process(&chunk);
        }
    }
    assert_eq!(mirror.screen().contents().matches("first").count(), 1);
}
