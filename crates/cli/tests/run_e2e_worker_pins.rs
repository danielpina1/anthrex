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
    let h = RunHarness::new("");
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
        &[task("t1", &["a.txt"], ""), task("t2", &["b.txt"], CODEX)],
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
    assert_eq!(denied.len(), 5, "{denied:?}");
    for tail in [
        "/.claude",
        "/.codex",
        "/.mcp.json",
        "/AGENTS.md",
        "/CLAUDE.md",
    ] {
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

    // Codex: network, `$TMPDIR` and `/tmp` pinned off, and the task's own
    // temporary directory among the writable roots.
    let argv: Vec<String> = serde_json::from_str(&h.io_lines("worker-t2-1", "args")[0]).unwrap();
    for pin in [
        "sandbox_workspace_write.network_access=false",
        "sandbox_workspace_write.exclude_tmpdir_env_var=true",
        "sandbox_workspace_write.exclude_slash_tmp=true",
    ] {
        assert!(argv.iter().any(|a| a == pin), "{pin} missing from {argv:?}");
    }
    let roots = argv
        .iter()
        .find(|a| a.starts_with("sandbox_workspace_write.writable_roots="))
        .unwrap();
    assert!(roots.contains(&tmps[1]), "{roots}");
}
