//! `anthrex --version` prints the workspace version and never reaches a daemon.

use std::process::Command;

#[test]
fn version_flag_prints_the_package_version_without_starting_a_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("anthrex.sock");
    let output = Command::new(env!("CARGO_BIN_EXE_anthrex"))
        .arg("--version")
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
