//! `anthrex --version` prints the workspace version and never reaches a daemon.

use std::process::Command;

#[test]
fn version_flag_prints_the_package_version_without_starting_a_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("anthrex.sock");
    let missing = dir.path().join("no-such-agent");
    let mut command = Command::new(env!("CARGO_BIN_EXE_anthrex"));
    // Pinned like every test that starts anthrex (`support::pin_agents`, which also pins
    // `TZ=UTC0`), so nothing can reach a real agent or gh.
    let output = crate::support::pin_agents(&mut command)
        .arg("--version")
        .env("ANTHREX_CLAUDE_BIN", &missing)
        .env("ANTHREX_CODEX_BIN", &missing)
        .env("ANTHREX_DECIDER_BIN", &missing)
        .env("ANTHREX_GH_BIN", &missing)
        .env("ANTHREX_SOCKET", &socket)
        .env("ANTHREX_DATA_DIR", dir.path().join("data"))
        .env("ANTHREX_CONFIG", dir.path().join("config.toml"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("anthrex {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(!socket.exists(), "--version must not start a daemon");
    assert!(!dir.path().join("data").exists());
}
