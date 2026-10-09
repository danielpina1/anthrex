//! M9.8.6 fix round 1 (MR §4.1, ruling F29): a model probe runs with the same scrubbed
//! environment as a headless session (`headless::session::session_env`), not only the
//! credential list: an inherited `CLAUDECODE` or `ANTHREX_WINDOW_ID` never reaches it.
//!
//! Alone in its own test binary, as `git_env_scrub.rs` is and for its reason: setting a
//! variable races any thread that spawns a process, and only a binary with no other
//! test has no such thread. Keep this file to this one test.

use std::sync::Mutex;
use std::time::Instant;

use daemon::models::{DISCOVERY_TIMEOUT, claude_probe};
use tokio_util::sync::CancellationToken;

/// Held while the environment is changed (AGENTS.md: edition 2024's `set_var`).
static ENV_LOCK: Mutex<()> = Mutex::new(());

const REPLY: &str = r#"{"type":"control_response","response":{"subtype":"success","request_id":"anthrex-models-1","response":{"models":[{"value":"claude-ok","displayName":"OK","description":"","supportsEffort":false}]}}}"#;

#[test]
fn a_probe_sees_neither_claudecode_nor_the_window_identity() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let seen = dir.path().join("env");
    // A stand-in `claude` (never the real one): it records the two variables, answers
    // `initialize`, and reads stdin to its end.
    let program = dir.path().join("claude");
    testexec::write_executable(
        &program,
        format!(
            "#!/bin/sh\nread -r line\necho \"CLAUDECODE=${{CLAUDECODE-unset}} ANTHREX_WINDOW_ID=${{ANTHREX_WINDOW_ID-unset}}\" > '{}'\necho '{REPLY}'\ncat > /dev/null\n",
            seen.display()
        ),
    );
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    unsafe {
        std::env::set_var("CLAUDECODE", "1");
        std::env::set_var("ANTHREX_WINDOW_ID", "7");
    }
    let result = claude_probe::probe(
        program.to_str().unwrap(),
        Instant::now() + DISCOVERY_TIMEOUT,
        &CancellationToken::new(),
        &[],
    );
    unsafe {
        std::env::remove_var("CLAUDECODE");
        std::env::remove_var("ANTHREX_WINDOW_ID");
    }
    let (models, _) = result.unwrap();
    assert_eq!(models[0].id, "claude-ok");
    assert_eq!(
        std::fs::read_to_string(&seen).unwrap().trim(),
        "CLAUDECODE=unset ANTHREX_WINDOW_ID=unset"
    );
}
