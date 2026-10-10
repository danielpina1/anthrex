//! Decision 29 (milestone 9.11, milestone 7's SIGWINCH item): a client that attaches to a
//! window at the window's own size gets a repaint from a full-screen app. `vt100` 0.16's
//! snapshot encodes no scroll region, origin mode, saved cursor or character set, so the
//! only way to give an attaching client a correct screen is to have the app draw it
//! again; a `SIGWINCH` asks it to. A size change already delivers one, so only an attach
//! at the same size needs the nudge, and only a `Subscribe` nudges (pre-flight P2): the
//! client's ordinary debounced `Resize` at the same size must not.
//!
//! The window runs `sh -c` with a `WINCH` trap and a loop of short sleeps (no job
//! control, so the kernel's `SIGWINCH` reaches the `sh` itself; the trap runs when the
//! current `sleep 0.05` ends). The markers are arithmetic (`W-$((1+1))` prints `W-2`) so
//! the terminal's echo of the typed command never contains them.
mod support;

use proto::{ClientMsg, DaemonMsg, PROTO_VERSION};
use std::time::{Duration, Instant};
use support::{Client, TestDaemon, shell_spec, start_daemon};

/// The hang guard for one request's effect, and the negative window. Derived: the nudge
/// is two `TIOCSWINSZ` ioctls made synchronously inside the `Subscribe`'s handling, before
/// its `Snapshot` is queued; the trap then runs when the loop's current `sleep 0.05`
/// ends (50 ms), plus one `sleep` spawn (`/bin/sh` spawn, 31-90 ms loaded, "Measured
/// primitive costs") and the forwarder's hop to the socket. Well under 1 s; 5 s is the
/// hang guard, and the negative test waits the same so a nudge would have shown in it
/// many times over.
const REQUEST_WAIT: Duration = Duration::from_secs(5);

const COLS: u16 = 80;
const ROWS: u16 = 24;

const WINCH_LOOP: &[u8] =
    b"exec sh -c 'trap \"echo W-$((1+1))\" WINCH; echo A-$((2+3)); while :; do sleep 0.05; done'\n";

async fn create(c: &mut Client) -> u32 {
    c.send(ClientMsg::CreateWindow {
        spec: shell_spec("winch"),
        cols: COLS,
        rows: ROWS,
    })
    .await;
    let DaemonMsg::Created { window_id } = c
        .recv_until(|m| matches!(m, DaemonMsg::Created { .. }))
        .await
    else {
        unreachable!()
    };
    window_id
}

async fn subscribe(c: &mut Client, window_id: u32) -> (u16, u16, Vec<u8>) {
    c.send(ClientMsg::Subscribe {
        window_id,
        cols: COLS,
        rows: ROWS,
    })
    .await;
    let DaemonMsg::Snapshot {
        cols, rows, bytes, ..
    } = c
        .recv_until(|m| matches!(m, DaemonMsg::Snapshot { window_id: w, .. } if *w == window_id))
        .await
    else {
        unreachable!()
    };
    (cols, rows, bytes)
}

/// Collects the window's output until it contains `needle` or `wait` passes; returns
/// whether it was seen.
async fn output_shows(c: &mut Client, window_id: u32, needle: &str, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        let Ok(msg) = tokio::time::timeout(left, proto::read_frame(&mut c.rd)).await else {
            return false;
        };
        let msg: DaemonMsg = msg.unwrap().expect("daemon closed");
        if let DaemonMsg::Output {
            window_id: w,
            bytes,
        } = msg
            && w == window_id
        {
            seen.extend_from_slice(&bytes);
            if String::from_utf8_lossy(&seen).contains(needle) {
                return true;
            }
        }
    }
}

/// A window running the trap loop, subscribed once at its own size, the loop armed.
async fn armed_window(d: &TestDaemon) -> (Client, u32) {
    let (mut c, _) = Client::connect(d, PROTO_VERSION).await;
    let id = create(&mut c).await;
    subscribe(&mut c, id).await;
    c.send(ClientMsg::Input {
        window_id: id,
        bytes: WINCH_LOOP.to_vec(),
    })
    .await;
    assert!(
        output_shows(&mut c, id, "A-5", REQUEST_WAIT).await,
        "the trap loop never started"
    );
    (c, id)
}

#[tokio::test]
async fn subscribing_at_the_same_size_repaints() {
    let d = start_daemon().await;
    let (mut c, id) = armed_window(&d).await;

    let (cols, rows, _) = subscribe(&mut c, id).await;
    assert_eq!((cols, rows), (COLS, ROWS));
    assert!(
        output_shows(&mut c, id, "W-2", REQUEST_WAIT).await,
        "a subscribe at the window's own size sent the app no SIGWINCH"
    );
}

#[tokio::test]
async fn the_parser_keeps_its_size_through_the_nudge() {
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;
    let id = create(&mut c).await;
    subscribe(&mut c, id).await;
    // Text on the last row, the cursor parked at the top, then idle (no trap: the
    // default action of SIGWINCH is to ignore it, so nothing redraws the screen).
    c.send(ClientMsg::Input {
        window_id: id,
        bytes: b"exec sh -c 'printf \"\\033[24;1HLAST-$((3+4))\\033[1;1H\"; while :; do sleep 0.05; done'\n"
            .to_vec(),
    })
    .await;
    assert!(output_shows(&mut c, id, "LAST-7", REQUEST_WAIT).await);

    for round in 0..2 {
        let (cols, rows, bytes) = subscribe(&mut c, id).await;
        assert_eq!((cols, rows), (COLS, ROWS), "round {round}");
        let mut parser = vt100::Parser::new(ROWS, COLS, 0);
        parser.process(&bytes);
        let last = parser
            .screen()
            .rows(0, COLS)
            .nth(usize::from(ROWS) - 1)
            .unwrap();
        assert_eq!(last.trim_end(), "LAST-7", "round {round}");
    }
}

/// Pinning: the client's ordinary `Resize` at the window's own size repaints nothing. The
/// subscribe afterwards proves the negative is not vacuous: the same window, the same
/// loop, does answer a nudge.
#[tokio::test]
async fn a_same_size_resize_does_not_nudge() {
    let d = start_daemon().await;
    let (mut c, id) = armed_window(&d).await;

    c.send(ClientMsg::Resize {
        window_id: id,
        cols: COLS,
        rows: ROWS,
    })
    .await;
    assert!(
        !output_shows(&mut c, id, "W-2", REQUEST_WAIT).await,
        "a same-size Resize sent the app a SIGWINCH"
    );

    subscribe(&mut c, id).await;
    assert!(
        output_shows(&mut c, id, "W-2", REQUEST_WAIT).await,
        "the subscribe after it sent no SIGWINCH either"
    );
}
