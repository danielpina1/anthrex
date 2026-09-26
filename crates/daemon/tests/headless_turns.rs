//! M8a.18: engine messages delivered to headless sessions as turns, interrupted, and
//! resumed (decisions 28, 29 and 52), against a real manager and recording `/bin/sh`
//! stand-ins for the CLIs (never the real binaries).
//!
//! Split by seam (AGENTS.md rule 8): `headless_turns/cursor.rs` holds the conversation
//! cursor's delivery-order tests carried from M8a.7's reviews, `headless_turns/control.rs`
//! the kills, interrupts and refusals of fix round 1, `headless_turns/ending.rs` the
//! windows the engine retires or kills (M8a.22).

mod support;

#[path = "headless_turns/control.rs"]
mod control;
#[path = "headless_turns/cursor.rs"]
mod cursor;
#[path = "headless_turns/ending.rs"]
mod ending;

use daemon::headless::argv::{CLI_CAPS, claude_args, codex_args};
use daemon::headless::claude_stream::{interrupt_request, user_message};
use daemon::headless::{HeadlessSpec, SessionArg, SessionEvent};
use daemon::manager::{WindowManager, WindowSignal, WindowSignalKind};
use proto::{Runtime, Status};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use support::headless::*;

/// The Claude session id `create` passes as `--session-id`.
const CLAUDE_UUID: &str = "00000000-0000-4000-8000-00000000c1a0";
/// The thread id every recording Codex prints.
const CODEX_THREAD: &str = "00000000-0000-4000-8000-000000000157";
/// The manager's `exe` and socket as `support::headless::manager` builds it.
const EXE: &str = "anthrex";
const SOCKET: &str = "/tmp/anthrex-m8a17-unused.sock";

/// Removes every window at the end of a test, failing or not, so no recording script
/// outlives it.
pub struct Cleanup(pub Arc<WindowManager>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        for window in self.0.list() {
            let _ = self.0.remove(window.id);
        }
    }
}

/// Shell text that appends this invocation's argv to `args.log` (each argument
/// NUL-terminated, the record ended by `\x1e`), and notes in `overlap.log` when the
/// previous invocation of the same program is still alive (decision 52).
pub fn record_argv(dir: &Path) -> String {
    format!(
        r#"D='{d}'
for a in "$@"; do printf '%s\0' "$a"; done >> "$D/args.log"; printf '\036' >> "$D/args.log"
if [ -f "$D/live.pid" ] && kill -0 "$(cat "$D/live.pid")" 2>/dev/null; then echo overlap >> "$D/overlap.log"; fi
echo $$ > "$D/live.pid"
"#,
        d = dir.display()
    )
}

/// A recording Claude: its argv to `args.log`, each stdin line to `stdin.log`, and for
/// each user message a whole turn (`init` with the session id it was given, `reply <n>`,
/// a success `result`). With `gated`, turn `n` waits for the file `go<n>` first, so a
/// test can apply the turn's prompt hook before any of its lines, as Claude does.
pub fn claude_recorder(dir: &Path, gated: bool) -> PathBuf {
    let gate = if gated {
        r#"while [ ! -f "$D/go$n" ]; do sleep 0.02; done"#
    } else {
        ""
    };
    script(
        dir,
        "claude",
        &format!(
            r#"{record}
sid=; prev=
for a in "$@"; do case "$prev" in --session-id|--resume) sid=$a;; esac; prev=$a; done
n=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$D/stdin.log"
  case "$line" in *'"control_request"'*) continue;; esac
  n=$((n+1))
  {gate}
  printf '{{"type":"system","subtype":"init","session_id":"%s","model":"m"}}\n' "$sid"
  printf '{{"type":"assistant","parent_tool_use_id":null,"message":{{"content":[{{"type":"text","text":"reply %s"}}]}}}}\n' "$n"
  printf '{{"type":"result","subtype":"success","is_error":false}}\n'
done"#,
            record = record_argv(dir),
        ),
    )
}

/// A recording Codex: reads stdin to EOF first, as codex-cli 0.156.1 does (ruling
/// T17-C1), records its argv, then prints one turn whose prose is `reply <n>`, `n` its
/// invocation count. `hold` runs before `turn.completed` (a gate, to keep a turn open).
pub fn codex_recorder(dir: &Path, hold: &str) -> PathBuf {
    script(
        dir,
        "codex",
        &format!(
            r#"cat >/dev/null
{record}
n=$(tr -cd '\036' < "$D/args.log" | wc -c | tr -d ' ')
printf '{{"type":"thread.started","thread_id":"{CODEX_THREAD}"}}\n'
printf '{{"type":"turn.started"}}\n'
printf '{{"type":"item.completed","item":{{"id":"item_0","type":"agent_message","text":"reply %s"}}}}\n' "$n"
{hold}
printf '{{"type":"turn.completed","usage":{{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2}}}}\n'"#,
            record = record_argv(dir),
        ),
    )
}

/// Every recorded invocation's argv, in order.
pub fn argvs(dir: &Path) -> Vec<Vec<String>> {
    let text = std::fs::read(dir.join("args.log")).unwrap_or_default();
    String::from_utf8(text)
        .unwrap()
        .split('\x1e')
        .filter(|record| !record.is_empty())
        .map(|record| {
            record
                .split('\0')
                .filter(|a| !a.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .collect()
}

pub fn stdin_lines(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("stdin.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

pub fn is_turn_end(signal: &WindowSignal) -> bool {
    matches!(
        signal.kind,
        WindowSignalKind::Session(SessionEvent::TurnEnded { .. })
    )
}

pub fn is_exit(signal: &WindowSignal) -> bool {
    matches!(
        signal.kind,
        WindowSignalKind::Session(SessionEvent::ProcessExited { .. })
    )
}

fn claude_resume_args(spec: &HeadlessSpec, id: u32, session_id: &str) -> Vec<String> {
    claude_args(
        spec,
        &SessionArg::Resume {
            session_id: session_id.into(),
        },
        Path::new(EXE),
        id,
        Path::new(SOCKET),
        &CLI_CAPS,
    )
}

fn codex_resume_args(spec: &HeadlessSpec, id: u32, message: &str) -> Vec<String> {
    codex_args(
        spec,
        &SessionArg::Resume {
            session_id: CODEX_THREAD.into(),
        },
        message,
        Path::new(EXE),
        id,
        Path::new(SOCKET),
        &CLI_CAPS,
    )
}

/// `args` without the flag `flag` and its value.
fn without(args: &[String], flag: &str) -> Vec<String> {
    let at = args
        .iter()
        .position(|a| a == flag)
        .unwrap_or_else(|| panic!("{flag} in {args:?}"));
    let mut out = args.to_vec();
    out.drain(at..at + 2);
    out
}

pub fn gone(pid: u32) -> bool {
    // SAFETY: a signal-0 probe of a pid.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

#[tokio::test]
async fn claude_send_writes_one_envelope_line() {
    let dir = tempfile::tempdir().unwrap();
    let claude = claude_recorder(dir.path(), false);
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    next_signal(&mut feed, "the first turn's end", is_turn_end).await;

    m.headless_send(info.id, "hi").await.unwrap();
    next_signal(&mut feed, "the second turn's end", is_turn_end).await;
    assert_eq!(
        stdin_lines(dir.path()),
        [
            user_message("first", Some(CLAUDE_UUID)),
            user_message("hi", Some(CLAUDE_UUID)),
        ]
    );
    assert_eq!(argvs(dir.path()).len(), 1, "one process served both turns");
}

#[tokio::test]
async fn codex_send_spawns_exec_resume_with_the_message_last() {
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let spec = spec(Runtime::Codex, dir.path());
    let info = create(&m, "c", spec.clone(), "first").await;
    next_signal(&mut feed, "the first process's exit", is_exit).await;
    assert_eq!(find(&m, info.id).status, Status::Idle);

    m.headless_send(info.id, "hi").await.unwrap();
    next_signal(&mut feed, "the second process's exit", is_exit).await;
    let argvs = argvs(dir.path());
    assert_eq!(argvs.len(), 2, "{argvs:?}");
    let resume = codex_resume_args(&spec, info.id, "hi");
    assert_eq!(argvs[1], resume);
    assert_eq!(argvs[1][..4], ["exec", "resume", CODEX_THREAD, "--json"]);
    assert_eq!(argvs[1][argvs[1].len() - 2..], ["--", "hi"]);
    assert!(
        !dir.path().join("overlap.log").exists(),
        "one process at a time"
    );
    assert_eq!(find(&m, info.id).status, Status::Idle);
}

#[tokio::test]
async fn a_send_while_a_codex_turn_is_running_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), &gate(dir.path(), "done"));
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "c", spec(Runtime::Codex, dir.path()), "first").await;
    wait_until("the turn to open", || {
        find(&m, info.id).status == Status::Working
    })
    .await;

    let error = m.headless_send(info.id, "hi").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("a turn is already running for window {}", info.id)
    );
    open_gate(dir.path(), "done");
    next_signal(&mut feed, "the exit", is_exit).await;
    assert_eq!(argvs(dir.path()).len(), 1, "no second process was started");
}

#[tokio::test]
async fn claude_resume_kills_a_live_process_first() {
    let dir = tempfile::tempdir().unwrap();
    let claude = claude_recorder(dir.path(), false);
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let spec = spec(Runtime::Claude, dir.path());
    let info = create(&m, "w", spec.clone(), "first").await;
    let first = next_signal(&mut feed, "the first turn's end", is_turn_end).await;
    let first_pid = first.pid.unwrap();
    assert!(!gone(first_pid));

    m.headless_resume(info.id, CLAUDE_UUID, "go on", Duration::ZERO)
        .await
        .unwrap();
    assert!(gone(first_pid), "the first process was stopped");
    assert!(
        !dir.path().join("overlap.log").exists(),
        "the resumed process started only after the first was gone"
    );
    let argvs = argvs(dir.path());
    assert_eq!(argvs.len(), 2, "{argvs:?}");
    assert_eq!(argvs[1], claude_resume_args(&spec, info.id, CLAUDE_UUID));
    // Every flag of the first argv is re-passed; only the session flag differs.
    assert_eq!(
        without(&argvs[0], "--session-id"),
        without(&argvs[1], "--resume")
    );
    // Settings isolation (decision 53) survives the resume.
    for flag in CLI_CAPS.claude_user_settings_only.unwrap() {
        assert!(argvs[1].iter().any(|a| a == flag), "{flag}");
    }
    let resumed = next_signal(&mut feed, "the resumed turn's end", is_turn_end).await;
    assert_ne!(resumed.pid, Some(first_pid));
    assert_eq!(
        stdin_lines(dir.path()).last().unwrap(),
        &user_message("go on", Some(CLAUDE_UUID))
    );
    let window = find(&m, info.id);
    assert_eq!(window.status, Status::Idle);
    assert_eq!(window.session_id.as_deref(), Some(CLAUDE_UUID));
    assert_eq!(m.child_pid(info.id).unwrap(), resumed.pid);
}

/// The stale-pid rule end to end (M8a.17 fix round 1's carry): a killed process's last
/// words (a `result` and an `init` naming another session, printed on `SIGTERM`) land
/// before the resumed process starts, and never move the resumed session's window.
#[tokio::test]
async fn a_replaced_processes_events_never_move_the_resumed_window() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = claude_recorder(dir.path(), false);
    let body = std::fs::read_to_string(&recorder).unwrap();
    // The first process says its last words however it is stopped (on SIGTERM, or when
    // its stdin closes first), and takes a while to die: a resume that did not wait for
    // it would start the second process beside it (`overlap.log`), and its words would
    // arrive after the second process's first events.
    let last_words = r#"last() {
  trap '' TERM
  sleep 0.3
  printf '%s\n' '{"type":"system","subtype":"init","session_id":"stale","model":"m"}' '{"type":"result","subtype":"success","is_error":false}'
  exit 0
}
dying=
if [ ! -f "$D/first" ]; then : > "$D/first"; dying=1; trap last TERM; fi
n=0
"#;
    let dying = body.replacen("n=0\n", last_words, 1) + "\n[ -n \"$dying\" ] && last\n";
    let claude = script(
        dir.path(),
        "claude",
        dying.trim_start_matches("#!/bin/sh\n"),
    );
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    let first = next_signal(&mut feed, "the first turn's end", is_turn_end).await;
    let first_pid = first.pid.unwrap();

    m.headless_resume(info.id, CLAUDE_UUID, "go on", Duration::ZERO)
        .await
        .unwrap();
    // Everything the first process said came before the second one's first event.
    let mut seen_new = false;
    let mut stale_init = false;
    loop {
        let signal = next_signal(&mut feed, "the resumed turn", |_| true).await;
        if signal.pid == Some(first_pid) {
            assert!(
                !seen_new,
                "a late event of the replaced process: {signal:?}"
            );
            stale_init |= matches!(
                &signal.kind,
                WindowSignalKind::Session(SessionEvent::Init { session_id, .. }) if session_id == "stale"
            );
        } else {
            seen_new = true;
            if is_turn_end(&signal) {
                break;
            }
        }
    }
    assert!(stale_init, "the first process's last words were read");
    assert!(
        !dir.path().join("overlap.log").exists(),
        "the resumed process started only after the first was gone"
    );
    let window = find(&m, info.id);
    assert_eq!(window.session_id.as_deref(), Some(CLAUDE_UUID));
    assert_eq!(window.status, Status::Idle);
    assert!(m.child_pid(info.id).unwrap().is_some());
}

/// Ruling T17-C1 on the resume path: `codex exec resume` goes through the same spawn,
/// so its stdin is closed at once and a Codex that reads stdin to EOF first still runs.
#[tokio::test]
async fn codex_resume_closes_stdin_and_runs_the_turn() {
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let spec = spec(Runtime::Codex, dir.path());
    let info = create(&m, "c", spec.clone(), "first").await;
    next_signal(&mut feed, "the first exit", is_exit).await;

    m.headless_resume(info.id, CODEX_THREAD, "carry on", Duration::ZERO)
        .await
        .unwrap();
    next_signal(&mut feed, "the resumed turn's exit", is_exit).await;
    assert_eq!(
        argvs(dir.path())[1],
        codex_resume_args(&spec, info.id, "carry on")
    );
    assert_eq!(find(&m, info.id).status, Status::Idle);
}

#[tokio::test]
async fn interrupt_follows_cli_caps() {
    // Claude, under CLI_CAPS (ControlRequest): a control request on stdin.
    let dir = tempfile::tempdir().unwrap();
    let claude = claude_recorder(dir.path(), false);
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    next_signal(&mut feed, "the first turn's end", is_turn_end).await;
    m.headless_interrupt(info.id).unwrap();
    wait_until("the control request", || stdin_lines(dir.path()).len() == 2).await;
    let line: serde_json::Value = serde_json::from_str(&stdin_lines(dir.path())[1]).unwrap();
    let id: u64 = line["request_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(stdin_lines(dir.path())[1], interrupt_request(id));

    // Codex: always SIGINT, to the running turn's process.
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("interrupted");
    let codex = codex_recorder(
        dir.path(),
        &format!(
            "trap 'echo int > \"{}\"; exit 130' INT\n: > \"$D/ready\"\nwhile :; do sleep 0.02; done",
            marker.display()
        ),
    );
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let info = create(&m, "c", spec(Runtime::Codex, dir.path()), "first").await;
    wait_until("the trap", || dir.path().join("ready").exists()).await;
    m.headless_interrupt(info.id).unwrap();
    wait_until("the SIGINT", || marker.exists()).await;
}

#[tokio::test]
async fn a_send_to_an_ended_session_is_an_error() {
    // Claude: the long-lived process exited. It reads its first message before exiting,
    // so the exit cannot race create_headless's write of that message (CI saw "the
    // session has ended" from create itself when the script exited first).
    let dir = tempfile::tempdir().unwrap();
    let claude = script(dir.path(), "claude", "head -n 1 >/dev/null; exit 0");
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    next_signal(&mut feed, "the exit", is_exit).await;
    let error = m.headless_send(info.id, "hi").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("session for window {} has ended; resume it", info.id)
    );

    // Codex: its process died before its turn ended (T17-I1), so the session is over.
    let dir = tempfile::tempdir().unwrap();
    let codex = script(dir.path(), "codex", "cat >/dev/null; echo boom >&2; exit 2");
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "c", spec(Runtime::Codex, dir.path()), "first").await;
    next_signal(&mut feed, "the exit", is_exit).await;
    assert_eq!(find(&m, info.id).status, Status::Exited);
    let error = m.headless_send(info.id, "hi").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("session for window {} has ended; resume it", info.id)
    );
}

#[tokio::test]
async fn resume_failure_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    // M8a.1 item 4: a resume of an unknown session exits 1 before any `init`, with the
    // text on stderr and in a failed `result`.
    let claude = script(
        dir.path(),
        "claude",
        r#"case " $* " in
  *" --resume "*)
    echo "No conversation found with session ID: nope" >&2
    printf '%s\n' '{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":0,"errors":["No conversation found with session ID: nope"]}'
    exit 1;;
esac
exec sleep 30"#,
    );
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    let error = m
        .headless_resume(info.id, "nope", "go on", Duration::ZERO)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("No conversation found with session ID: nope"),
        "{error}"
    );
    assert!(error.contains("exited with code 1"), "{error}");
    assert_eq!(find(&m, info.id).status, Status::Exited);
}
