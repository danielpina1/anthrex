//! End-to-end persistence tests that need the real `anthrex` binary — a hook fired by a
//! real `anthrex hook` child process, and a restart driven over the real wire protocol —
//! rather than a direct call into `daemon::manager::WindowManager`, the way
//! `crates/daemon/tests/manager.rs` exercises the same machinery in-process.

mod support;

use std::time::{Duration, Instant};

use proto::{ClientMsg, DaemonMsg, Runtime, Status};
use serde_json::json;
use support::TestDaemon;

/// Design decision 9 (state persistence) meeting milestone 3's hook path: a session id
/// `AgentState::on_hook` learns from a real `SessionStart` hook — fired by a real
/// `anthrex hook` child, not injected directly — reaches `state.json` through the
/// manager's `watch()` → `spawn_persister` pipeline.
///
/// M9.11.38 (C7): the window's record is in `state.json` from its creation, before the
/// hook, so the test waits until the record holds a session id, then asserts it is the
/// one `WindowInfo` has; it used to assert on the first record found, which could be
/// the creation's, with no session id yet, inside the hook's save debounce. The bound
/// is derived: the persister writes within `SAVE_DEBOUNCE` of the last change, and
/// never later than `SAVE_MAX_DELAY` after the first unwritten one, however the window
/// keeps changing; then one small fsynced write; plus [`SAVE_SLACK`].
///
/// The fake-agent script fires `SessionStart` and then blocks on `read_line`, mirroring
/// milestone 3's own convention (`crates/cli/tests/claude_status.rs`'s `turn_script`) —
/// enough to observe the session id without the process exiting mid-test.
#[test]
fn session_id_learned_from_a_hook_is_saved() {
    let daemon = TestDaemon::start(&[
        json!({"hook":"SessionStart", "payload":{}}),
        json!({"read_line":true}),
    ]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "persisted");

    let window = client.wait_window(id, "session id learned from the hook", |w| {
        w.session_id.is_some()
    });
    let session_id = window
        .session_id
        .clone()
        .expect("checked by wait_window's predicate above");

    let state_path = daemon.data_dir().join("state.json");
    let within = daemon::state::SAVE_MAX_DELAY + SAVE_SLACK;
    let deadline = Instant::now() + within;
    loop {
        let (loaded, _problems) = daemon::state::load(&state_path);
        let saved = (loaded.windows.iter())
            .find(|w| w.id == id)
            .and_then(|record| record.session_id.clone());
        if let Some(saved) = saved {
            assert_eq!(
                saved, session_id,
                "state.json's session id must equal WindowInfo's, not merely be non-null"
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "state.json never recorded window {id}'s session id within {within:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Slack on top of the persister's own bound: the write and its fsync on a blocking
/// thread, and this test's 20 ms polls, on a loaded host.
const SAVE_SLACK: Duration = Duration::from_secs(5);

/// Design decision 20: a live window's restart kills the old process and waits — up to
/// several real seconds — for it to be gone, and that wait must never stop this
/// connection's request loop from answering anything else in the meantime. Proven here by
/// making the wait actually long: the shell traps `HUP` and `TERM`, so the daemon's kill
/// escalation is forced through its full grace to `SIGKILL` rather than the ordinary
/// "interactive shell dies on HUP at once" case (AGENTS.md) every other test in this
/// workspace relies on.
///
/// The 500ms bound on `ListWindows`'s reply is the brief's own number, not derived from a
/// production constant — there is nothing in this codebase for it to coincide with, only
/// the kill escalation's multi-second wait it has to arrive well ahead of.
#[test]
fn restart_request_does_not_block_the_connection() {
    let daemon = TestDaemon::start(&[]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Shell, "stubborn");
    client.wait_window(id, "shell started", |w| w.status != Status::Starting);

    // Ignoring HUP and TERM on the login shell itself (the process the manager's
    // `signal_group` reaches first) forces the kill path all the way to its own
    // `SIGKILL` escalation instead of dying on the first signal like every other test's
    // shell does.
    client.subscribe(id);
    client.input(
        id,
        b"trap '' HUP TERM; printf 'READY\\n'; while :; do :; done\n",
    );
    client.receive(|m| {
        matches!(
            m,
            DaemonMsg::Output { window_id, bytes }
                if *window_id == id && String::from_utf8_lossy(bytes).contains("READY")
        )
    });

    // Drain whatever is already queued (an unrelated `WindowsChanged` broadcast from the
    // status change the trap command itself just caused, in particular) so the specific
    // `WindowsChanged` timed below cannot be mistaken for one already in flight.
    while client.try_receive(Duration::from_millis(150)).is_some() {}

    client.send(ClientMsg::Restart { window_id: id });
    let sent_list_windows_at = Instant::now();
    client.send(ClientMsg::ListWindows);

    // Minor 6 (fix wave 5 review): `WindowsChanged` carries no request id, so nothing in
    // the wire protocol lets this tell "the `ListWindows` reply" apart from an unrelated
    // broadcast by shape alone. The best this predicate can do is correlate the message to
    // this test's own window rather than accept any `WindowsChanged` whatsoever — matching
    // one that lists `id` still with the status it had before the kill escalation could
    // plausibly have changed anything (the drain above already removed the one broadcast
    // that status change was expected to cause). The ordering property this test actually
    // depends on is enforced separately, by `receive_within` discarding non-matching
    // frames: an `Ack { "restart" }` arriving first would be swallowed here and the later
    // 10s Ack wait below would time out and panic instead.
    client.receive_within(Duration::from_millis(500), |m| {
        matches!(
            m,
            DaemonMsg::WindowsChanged { windows }
                if windows.iter().any(|w| w.id == id && w.status == Status::Working)
        )
    });
    assert!(
        sent_list_windows_at.elapsed() < Duration::from_millis(500),
        "ListWindows was answered {:?} after being sent, while a live window's restart \
         was still killing its old process — the request loop was blocked on it",
        sent_list_windows_at.elapsed()
    );

    // The restart itself must still complete — this is what actually proves the wait
    // above was for the real kill escalation and not a coincidence.
    let ack = client.receive_within(Duration::from_secs(10), |m| {
        matches!(m, DaemonMsg::Ack { request } if request == "restart")
            || matches!(m, DaemonMsg::Error { request, .. } if request == "restart")
    });
    match ack {
        DaemonMsg::Ack { .. } => {}
        DaemonMsg::Error { message, .. } => panic!("restart of window {id} failed: {message}"),
        other => unreachable!("filtered by the predicate above: {other:?}"),
    }
}
