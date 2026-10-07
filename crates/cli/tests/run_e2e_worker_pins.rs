//! M8a final fix batch F1d (F1c re-review 2, R5), through a real daemon with
//! `fake-agent` as both runtimes: every worker gets a short `TMPDIR` of its own, never
//! the daemon's (which holds the daemon's socket on macOS), and its sandbox settings
//! are pinned closed on the command line, so the user's own `~/.claude` and `~/.codex`
//! settings cannot widen them. What the real CLIs make of those settings is manual
//! check 4e.

mod support;

use serde_json::Value;
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

#[test]
fn e2e_workers_get_their_own_tmpdir_and_pinned_sandbox_settings() {
    let h = RunHarness::with_config("", CODEX_MEDIUM, &[]);
    let seen = h.dir.path().join("seen");
    std::fs::create_dir_all(&seen).unwrap();
    for (task, file) in [("t1", "a.txt"), ("t2", "b.txt")] {
        h.script(
            &format!("worker-{task}-1"),
            &[
                sh(&format!(
                    "printf %s \"$TMPDIR\" > '{}'",
                    seen.join(task).display()
                )),
                commit(file, "x\n"),
                done("added it"),
            ],
        );
        h.script(&format!("reviewer-{task}-1"), &[approve()]);
    }
    let plan = plan(
        "",
        &[task("t1", &["a.txt"], ""), task_m("t2", &["b.txt"], "")],
    );
    let id = h.start(&plan, true);
    h.wait_run(&id, complete, RUN_WAIT);

    let daemon_tmp = std::env::temp_dir();
    let mut tmps = Vec::new();
    for task in ["t1", "t2"] {
        let tmp = std::fs::read_to_string(seen.join(task)).unwrap();
        assert!(
            tmp.len() <= 48,
            "{task}'s TMPDIR is {} bytes: {tmp}",
            tmp.len()
        );
        assert!(
            std::path::Path::new(&tmp).starts_with(daemon::run::git::tmp_root()),
            "{task}: {tmp}"
        );
        // Not the daemon's own (inherited) `TMPDIR`, nor anything under it on macOS,
        // where it holds the daemon's socket directory.
        assert_ne!(std::path::Path::new(&tmp), daemon_tmp, "{task}: {tmp}");
        if cfg!(target_os = "macos") {
            assert!(
                !std::path::Path::new(&tmp).starts_with(&daemon_tmp),
                "{task} is under the daemon's TMPDIR: {tmp}"
            );
        }
        tmps.push(tmp);
    }
    assert_ne!(tmps[0], tmps[1], "two tasks share a TMPDIR");

    // Claude: the sandbox block pins every widening key closed.
    let argv: Vec<String> = serde_json::from_str(&h.io_lines("worker-t1-1", "args")[0]).unwrap();
    let at = argv.iter().position(|a| a == "--settings").unwrap();
    let settings: Value = serde_json::from_str(&argv[at + 1]).unwrap();
    let sandbox = &settings["sandbox"];
    assert_eq!(
        sandbox["network"]["allowUnixSockets"],
        serde_json::json!([])
    );
    assert_eq!(sandbox["network"]["allowAllUnixSockets"], false);
    assert_eq!(sandbox["network"]["allowLocalBinding"], false);
    assert_eq!(sandbox["network"]["allowedDomains"], serde_json::json!([]));
    assert_eq!(sandbox["allowAppleEvents"], false);
    assert_eq!(sandbox["excludedCommands"], serde_json::json!([]));
    assert_eq!(sandbox["filesystem"]["disabled"], false);
    // F2 round 2: the checkout's protected agent-config paths are denied (t1 owns
    // only `a.txt`, so all five are; F4: no any-depth entry).
    let denied: Vec<&str> = sandbox["filesystem"]["denyWrite"]
        .as_array()
        .expect("denyWrite")
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    let protected = [
        "/.claude",
        "/.codex",
        "/.mcp.json",
        "/AGENTS.md",
        "/CLAUDE.md",
    ];
    // On Linux the worker's whole git directory is granted, and these entries of it are
    // denied (the task checkout's own repository has no `commondir`, so none is denied).
    let git_denied: &[&str] = if cfg!(target_os = "linux") {
        &[
            "/git/gitdir",
            "/git/config",
            "/git/config.worktree",
            "/git/locked",
            "/git/packed-refs",
            "/git/logs",
            "/git/refs",
            "/git/info",
            "/git/hooks",
            "/git/modules",
        ]
    } else {
        &[]
    };
    assert_eq!(
        denied.len(),
        protected.len() + git_denied.len(),
        "{denied:?}"
    );
    for tail in protected.iter().chain(git_denied) {
        assert!(
            denied.iter().any(|p| p.ends_with(tail)),
            "{tail}: {denied:?}"
        );
    }
    let writable = sandbox["filesystem"]["allowWrite"].as_array().unwrap();
    assert!(
        writable
            .iter()
            .any(|p| p.as_str() == Some(tmps[0].as_str())),
        "{sandbox}"
    );

    // Codex (fake-agent reports 0.160.1, so the profiles dialect): network off, no
    // legacy flags, the task's own temporary directory writable, its checkout's `.git`
    // read-only (Codex's legacy `workspace-write` protected it silently; a profile does
    // not), and on Linux the same git entries read-only as Claude's. The fixture's
    // checkout has none of the five protected agent-config paths, so none is listed.
    let argv: Vec<String> = serde_json::from_str(&h.io_lines("worker-t2-1", "args")[0]).unwrap();
    for want in [
        "default_permissions=\"anthrex\"",
        "permissions.anthrex.extends=\":read-only\"",
        "permissions.anthrex.network.enabled=false",
    ] {
        assert!(
            argv.iter().any(|a| a == want),
            "{want} missing from {argv:?}"
        );
    }
    assert!(
        !argv.iter().any(|a| a == "-s" || a.starts_with("sandbox_")),
        "{argv:?}"
    );
    let fs = argv
        .iter()
        .find(|a| a.starts_with("permissions.anthrex.filesystem="))
        .unwrap();
    assert!(fs.contains(&format!("\"{}\"=\"write\"", tmps[1])), "{fs}");
    // The profile's first entry is the session's working directory, the checkout.
    let cwd = fs
        .split_once("={\"")
        .and_then(|(_, rest)| rest.split_once("\"=\"write\""))
        .map(|(cwd, _)| cwd)
        .unwrap();
    // The `.git` entry spells the checkout as the grant does: canonically (on macOS
    // `/private/var` for `/var`), whatever spelling the working directory has.
    let git_entry = fs
        .split("\"=\"read\"")
        .filter_map(|head| head.rsplit_once('"').map(|(_, path)| path))
        .find(|path| path.ends_with("/.git"))
        .unwrap_or_else(|| panic!("no .git read entry: {fs}"));
    let canonical = std::fs::canonicalize(h.dir.path()).unwrap();
    assert!(
        std::path::Path::new(git_entry).starts_with(&canonical),
        "{git_entry} is not under {}: {fs}",
        canonical.display()
    );
    assert!(git_entry.ends_with(&format!("{}/.git", cwd.rsplit('/').next().unwrap())));
    for tail in protected {
        assert!(!fs.contains(&format!("{tail}\"=")), "{tail}: {fs}");
    }
    for tail in git_denied {
        assert!(fs.contains(&format!("{tail}\"=\"read\"")), "{tail}: {fs}");
    }
    assert_eq!(
        fs.matches("\"=\"read\"").count(),
        1 + git_denied.len(),
        "{fs}"
    );
}

/// Final review M2: a Codex older than permission profiles (the daemon's startup probe
/// reads 0.155.0) keeps the legacy flags end to end: `-s workspace-write`, the three
/// pins, and the exact git files plus the task's temporary directory as writable roots;
/// no profile key.
#[test]
fn e2e_a_codex_worker_below_profiles_gets_the_legacy_flags() {
    let h = RunHarness::with_env_and_config("", &[("FAKE_CODEX_VERSION", "0.155.0")], CODEX_MEDIUM);
    let seen = h.dir.path().join("seen");
    h.script(
        "worker-t1-1",
        &[
            sh(&format!("printf %s \"$TMPDIR\" > '{}'", seen.display())),
            commit("a.txt", "x\n"),
            done("added it"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task_m("t1", &["a.txt"], "")]), true);
    h.wait_run(&id, complete, RUN_WAIT);
    let tmp = std::fs::read_to_string(&seen).unwrap();

    let argv: Vec<String> = serde_json::from_str(&h.io_lines("worker-t1-1", "args")[0]).unwrap();
    assert!(
        argv.windows(2).any(|w| w == ["-s", "workspace-write"]),
        "{argv:?}"
    );
    for pin in daemon::headless::argv::CODEX_SANDBOX_PINS {
        assert!(argv.iter().any(|a| a == pin), "{pin} missing: {argv:?}");
    }
    let roots = argv
        .iter()
        .find(|a| a.starts_with("sandbox_workspace_write.writable_roots="))
        .unwrap_or_else(|| panic!("no writable roots: {argv:?}"));
    assert!(roots.contains(&format!("\"{tmp}\"")), "{tmp}: {roots}");
    assert!(
        !argv
            .iter()
            .any(|a| a.starts_with("default_permissions=") || a.starts_with("permissions.")),
        "{argv:?}"
    );
}
