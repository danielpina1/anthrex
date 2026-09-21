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
/// manager's `watch()` → `spawn_persister` pipeline within `SAVE_DEBOUNCE` (100ms) of the
/// change, so 2s is a wide failsafe over that real cost, not the expected one.
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
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let (loaded, _problems) = daemon::state::load(&state_path);
        if let Some(record) = loaded.windows.iter().find(|w| w.id == id) {
            assert_eq!(
                record.session_id.as_deref(),
                Some(session_id.as_str()),
                "state.json's session id must equal WindowInfo's, not merely be non-null"
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "state.json never recorded window {id} within 2s"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

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

    client.receive_within(Duration::from_millis(500), |m| {
        matches!(m, DaemonMsg::WindowsChanged { .. })
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
