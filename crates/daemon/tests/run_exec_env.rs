//! M8a.10, decision 26 for engine commands: `setup`, `check` and proof runs lose every
//! inherited `CLAUDE_CODE_*` variable, `CLAUDECODE` and `ANTHREX_WINDOW_ID`, and get the
//! profile's `env` — alone in its own test binary, because it sets variables in this
//! process's environment, and in edition 2024 that races any process spawn on another
//! libtest thread (`worktree_env.rs`'s module doc is the precedent). Keep this file to
//! this one test.

use daemon::run::env::profile_env;
use daemon::run::exec::run_shell;
use daemon::run::model::Profile;
use std::collections::BTreeMap;
use std::time::Duration;

#[test]
fn engine_commands_get_the_profile_env_and_lose_agent_variables() {
    // SAFETY: this binary has exactly one test, so no other thread reads or spawns while
    // the environment changes.
    unsafe {
        std::env::set_var("CLAUDE_CODE_CHILD_SESSION", "1");
        std::env::set_var("CLAUDECODE", "1");
        std::env::set_var("ANTHREX_WINDOW_ID", "9");
        std::env::set_var("ANTHREX_M8A10_KEPT", "kept");
    }
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path().canonicalize().unwrap();
    let profile = Profile {
        modules: Vec::new(),
        hub: Vec::new(),
        source: Vec::new(),
        check: None,
        check_timeout_secs: 60,
        single_test: None,
        test_passed: None,
        setup: None,
        generated: Vec::new(),
        protected: Vec::new(),
        env: BTreeMap::from([(
            "CARGO_TARGET_DIR".to_string(),
            "{worktree}/target".to_string(),
        )]),
        cache_dirs: Vec::new(),
        confined_network: false,
        confined_unix_sockets: Vec::new(),
        confined_localhost_ports: Vec::new(),
    };
    let env = profile_env(&profile, &worktree);
    // Filtered so that a long environment cannot push a line out of the 200-line tail.
    let outcome = run_shell(
        &worktree,
        "env | grep -E '^(CLAUDE|ANTHREX|CARGO_TARGET_DIR)'; true",
        &env,
        Duration::from_secs(60),
    );
    assert!(outcome.ok, "{outcome:?}");
    let lines: Vec<&str> = outcome.tail.lines().collect();
    for line in &lines {
        assert!(
            !line.starts_with("CLAUDE_CODE_") && !line.starts_with("CLAUDECODE="),
            "an agent variable reached the engine command: {line}"
        );
        assert!(
            !line.starts_with("ANTHREX_WINDOW_ID="),
            "an engine command belongs to no window: {line}"
        );
    }
    let target = format!("CARGO_TARGET_DIR={}/target", worktree.display());
    assert!(lines.contains(&target.as_str()), "{lines:?}");
    assert!(
        lines.contains(&"ANTHREX_M8A10_KEPT=kept"),
        "only the named variables are removed: {lines:?}"
    );
}
