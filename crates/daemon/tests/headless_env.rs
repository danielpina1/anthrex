//! M8a.17, decision 26 for headless sessions: every session process loses the inherited
//! `CLAUDE_CODE_*` variables and `CLAUDECODE`, has `ANTHREX_WINDOW_ID` and
//! `ANTHREX_SOCKET` set to its own window and socket, and gets the profile's `env`.
//! Since final fix batch F2 a login session also loses `ANTHROPIC_API_KEY` and
//! `ANTHROPIC_AUTH_TOKEN`.
//!
//! Alone in its own test binary, because it sets variables in this process's
//! environment, and in edition 2024 that races any process spawn on another libtest
//! thread (`run_exec_env.rs` and `worktree_env.rs` are the precedent). Keep this file to
//! this one test.

mod support;

use daemon::run::env::profile_env;
use daemon::run::model::Profile;
use proto::Runtime;
use std::collections::BTreeMap;
use support::headless::*;

#[test]
fn the_environment_is_scrubbed() {
    // SAFETY: this binary has exactly one test, and the runtime below is built after
    // this, so no other thread reads or spawns while the environment changes.
    unsafe {
        std::env::set_var("CLAUDE_CODE_CHILD_SESSION", "1");
        std::env::set_var("CLAUDECODE", "1");
        std::env::set_var("ANTHREX_WINDOW_ID", "9");
        // Final fix batch F2 (review C, M2): a login session never sees an API key the
        // daemon inherited, so `claude -p` cannot prefer it over the user's login.
        std::env::set_var("ANTHROPIC_API_KEY", "sk-inherited");
        std::env::set_var("ANTHROPIC_AUTH_TOKEN", "tok-inherited");
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().canonicalize().unwrap();
        let out = worktree.join("env.txt");
        let claude = script(
            &worktree,
            "claude",
            &format!(
                "env > '{out}.tmp' && mv '{out}.tmp' '{out}'; exec sleep 30",
                out = out.display()
            ),
        );
        let codex_out = worktree.join("codex-env.txt");
        let codex = script(
            &worktree,
            "codex",
            &format!(
                "env > '{out}.tmp' && mv '{out}.tmp' '{out}'; exec sleep 30",
                out = codex_out.display()
            ),
        );
        let m = manager(&claude, &codex, |_| {});
        // The profile's env as the engine builds a session's (`worker_spec`).
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
        let mut spec = spec(Runtime::Claude, &worktree);
        spec.env = profile_env(&profile, &worktree);
        let info = create(&m, "env", spec, "hi").await;
        assert_ne!(
            info.id, 9,
            "the test's inherited id must not be the window's"
        );
        wait_until("the env dump", || out.exists()).await;
        let env = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<&str> = env.lines().collect();
        assert!(
            !lines
                .iter()
                .any(|l| l.starts_with("CLAUDE_CODE_") || l.starts_with("CLAUDECODE=")),
            "{env}"
        );
        assert!(
            !lines
                .iter()
                .any(|l| l.starts_with("ANTHROPIC_API_KEY=")
                    || l.starts_with("ANTHROPIC_AUTH_TOKEN=")),
            "{env}"
        );
        assert!(
            lines.contains(&format!("ANTHREX_WINDOW_ID={}", info.id).as_str()),
            "{env}"
        );
        assert!(
            lines.contains(&"ANTHREX_SOCKET=/tmp/anthrex-m8a17-unused.sock"),
            "{env}"
        );
        assert!(
            lines.contains(&format!("CARGO_TARGET_DIR={}/target", worktree.display()).as_str()),
            "{env}"
        );
        m.remove(info.id).unwrap();

        // Final fix batch F2 (review C, M5): a Codex session gets neither the window id
        // nor the socket (only Claude's hooks read them; its MCP server has `--socket`
        // on its argv), nor the inherited ones.
        let mut codex_spec = support::headless::spec(Runtime::Codex, &worktree);
        codex_spec.env = profile_env(&profile, &worktree);
        let info = create(&m, "codex-env", codex_spec, "hi").await;
        wait_until("the codex env dump", || codex_out.exists()).await;
        let env = std::fs::read_to_string(&codex_out).unwrap();
        assert!(
            !env.lines().any(|l| l.starts_with("ANTHREX_WINDOW_ID=")
                || l.starts_with("ANTHREX_SOCKET=")
                || l.starts_with("ANTHROPIC_API_KEY=")),
            "{env}"
        );
        assert!(
            env.lines()
                .any(|l| l == format!("CARGO_TARGET_DIR={}/target", worktree.display())),
            "{env}"
        );
        m.remove(info.id).unwrap();
    });
}
