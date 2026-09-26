use super::*;
use crate::launch::LaunchPlan;
use crate::window::Window;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// I1: after `Lagged`, the forwarder must not replay the chunks the fresh snapshot
/// already contains. The mirror a client would build from the forwarded messages has
/// to match the daemon's own screen exactly.
///
/// The child is long finished before anything is drained, so the snapshot taken on
/// the lag provably contains every chunk still retained by the broadcast channel -
/// continuing with the old receiver replays exactly those, and the twelve printed
/// lines all fit on one screen, so a duplicate cannot scroll out of sight.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lagged_subscriber_is_resynced_without_replaying_retained_chunks() {
    let plan = LaunchPlan {
        program: "sh".into(),
        args: vec![
            "-c".into(),
            "i=1; while [ $i -le 12 ]; do echo line-$i; sleep 0.03; i=$((i+1)); done".into(),
        ],
        cwd: std::env::temp_dir(),
        env: vec![("TERM".into(), "xterm-256color".into())],
    };
    let (events, _events_rx) = mpsc::unbounded_channel();
    // `Window` is Send but not Sync, so the mutex is what lets the forwarder task
    // and the test share it. Capacity 2: the forwarder lags as soon as it waits.
    let window = Arc::new(std::sync::Mutex::new(
        Window::spawn_with_output_capacity(1, &plan, 80, 24, events, 2).unwrap(),
    ));

    let first = window.lock().unwrap().attach();
    let mut mirror = vt100::Parser::new(first.rows, first.cols, 0);
    mirror.process(&first.snapshot);

    // A one-slot outgoing channel that nobody drains: exactly the shape of a client
    // that cannot keep up.
    let (out, mut out_rx) = mpsc::channel::<DaemonMsg>(1);
    let forwarder = {
        let window = Arc::clone(&window);
        tokio::spawn(forward_output_from(1, first.output, out, move || {
            Ok(window.lock().unwrap().attach())
        }))
    };

    // Wait until every line is printed (about 0.4 s of output, far longer on a
    // loaded CI runner), then a little longer for the child to exit.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut expected = window.lock().unwrap().screen_text();
    while !expected.contains("line-12") && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        expected = window.lock().unwrap().screen_text();
    }
    assert!(
        expected.contains("line-12"),
        "the child did not finish: {expected:?}"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    let expected = window.lock().unwrap().screen_text();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut lagged = false;
    loop {
        match tokio::time::timeout(Duration::from_millis(500), out_rx.recv()).await {
            Ok(Some(DaemonMsg::Snapshot {
                cols, rows, bytes, ..
            })) => {
                lagged = true;
                mirror = vt100::Parser::new(rows, cols, 0);
                mirror.process(&bytes);
            }
            Ok(Some(DaemonMsg::Output { bytes, .. })) => mirror.process(&bytes),
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => break, // quiet for 500 ms: nothing more is coming
        }
        assert!(Instant::now() < deadline, "the forwarder never went quiet");
    }
    forwarder.abort();

    assert!(
        lagged,
        "the subscriber never lagged; the test did not exercise the recovery path"
    );
    assert_eq!(
        mirror.screen().contents(),
        expected,
        "the mirror diverged from the daemon's screen (chunks replayed on top of a snapshot that held them)"
    );
}

/// Design decision 21: a restart swaps the window's `Process`, dropping whatever the
/// forwarder's receiver was subscribed to. Before this task, `RecvError::Closed` ended
/// the loop outright — a window that got restarted would silently stop forwarding to
/// every subscriber watching it. This drives `forward_output_from` directly, with a
/// `reattach` that hands back a *second* window's `Attachment` the first time it is
/// called (standing in for the freshly restarted window) and fails the second time
/// (standing in for the window being gone by then), and checks both the recovery and
/// the eventual end are real: a fresh `Snapshot` from the second attachment, that
/// window's own output forwarded afterward, and the loop actually stopping once
/// `reattach` fails rather than spinning.
///
/// The two `timeout(..., 5s)` calls below are failsafes, not the expected cost: every
/// step is local channel plumbing with no PTY or shell involved, so in the ordinary
/// case each resolves in microseconds. 5 s only bounds how long a genuinely broken
/// forwarder would hang the test.
#[tokio::test]
async fn forwarder_reattaches_when_the_channel_closes() {
    // The first "window": its sender is dropped before the forwarder ever polls it,
    // so its very first `recv()` sees `Closed` at once.
    let (first_tx, first_rx) = broadcast::channel::<Bytes>(4);
    drop(first_tx);

    // The second "window": what `reattach()` hands back on its first call.
    let (second_tx, second_rx) = broadcast::channel::<Bytes>(4);
    let second_snapshot = b"second-window-snapshot".to_vec();

    let calls = Arc::new(AtomicUsize::new(0));
    let reattach = {
        let calls = calls.clone();
        let second_rx = Arc::new(std::sync::Mutex::new(Some(second_rx)));
        let second_snapshot = second_snapshot.clone();
        move || -> anyhow::Result<Attachment> {
            let call = calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                let output = second_rx
                    .lock()
                    .unwrap()
                    .take()
                    .expect("reattach's first call is the only one that succeeds");
                Ok(Attachment {
                    output,
                    snapshot: second_snapshot.clone(),
                    cols: 40,
                    rows: 10,
                })
            } else {
                Err(anyhow::anyhow!("window gone"))
            }
        }
    };

    let (out, mut out_rx) = mpsc::channel::<DaemonMsg>(8);
    let forwarder = tokio::spawn(forward_output_from(7, first_rx, out, reattach));

    // The first thing out of the forwarder must be a Snapshot built from the second
    // attachment — exactly like the Lagged recovery path, not the raw first chunk of
    // whatever the second window happens to send later.
    let msg = tokio::time::timeout(Duration::from_secs(5), out_rx.recv())
        .await
        .expect("no message from the forwarder within the 5s failsafe")
        .expect("the forwarder's outgoing channel closed with nothing sent");
    match msg {
        DaemonMsg::Snapshot {
            window_id,
            cols,
            rows,
            bytes,
        } => {
            assert_eq!(window_id, 7);
            assert_eq!((cols, rows), (40, 10));
            assert_eq!(bytes, second_snapshot);
        }
        other => panic!("expected a Snapshot built from the reattach, got {other:?}"),
    }

    // The forwarder is now reading the second window's output.
    second_tx
        .send(Bytes::from_static(b"second-window-output"))
        .unwrap();
    let msg = tokio::time::timeout(Duration::from_secs(5), out_rx.recv())
        .await
        .expect("no forwarded output within the 5s failsafe")
        .expect("the forwarder's outgoing channel closed with nothing sent");
    match msg {
        DaemonMsg::Output { window_id, bytes } => {
            assert_eq!(window_id, 7);
            assert_eq!(&bytes[..], b"second-window-output");
        }
        other => panic!("expected the second window's output forwarded, got {other:?}"),
    }

    // Close the second channel too. `reattach`'s second call fails, standing in for
    // the window having been removed by then, so the forwarder must end rather than
    // loop forever trying to recover.
    drop(second_tx);
    tokio::time::timeout(Duration::from_secs(5), forwarder)
        .await
        .expect("the forwarder did not end within the 5s failsafe after reattach failed")
        .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "reattach must be called exactly twice: once to recover, once to fail"
    );
}
