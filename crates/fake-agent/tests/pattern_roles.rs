//! Milestone 9.5 decision 31: `fake-agent`'s scripts for a racer
//! (`racer-<task>-<lane>-<n>.jsonl`, the lane from its MCP server's `--lane`) and a
//! test writer (`test_writer-<task>-<n>.jsonl`), claimed from the argv the daemon's
//! headless launcher builds for each (`daemon::headless::argv`). Real pipes, a real git
//! repository; the MCP server is never started.

mod headless_support;

use std::fs;
use std::path::{Path, PathBuf};

use daemon::headless::McpTarget;
use headless_support::*;
use proto::{AgentRole, RaceLane};
use serde_json::json;

/// The MCP target of task `t1`'s session in `role`, on `lane`.
fn target(role: AgentRole, lane: Option<RaceLane>) -> McpTarget {
    McpTarget {
        role,
        run_id: "r1".into(),
        task_id: Some("t1".into()),
        scout_id: None,
        epic: None,
        chain: None,
        lane,
    }
}

/// One Codex process of `target` in `repo`: what it printed.
fn session(repo: &Path, target: McpTarget) -> Vec<String> {
    let (exe, socket) = (
        PathBuf::from("/nonexistent/anthrex"),
        PathBuf::from("/tmp/nonexistent.sock"),
    );
    let argv = codex_argv_for(target, &exe, &socket, "go");
    let mut agent = Agent::codex(&argv, repo, &[]);
    assert!(agent.wait(RUN).success(), "{}", agent.stderr());
    texts(&agent)
}

fn claimed(repo: &Path, name: &str) -> bool {
    repo.join(format!(".git/fake-agent/{name}.jsonl.claimed"))
        .exists()
}

#[test]
fn racer_scripts_are_claimed_by_lane() {
    // Whichever lane's racer starts first, each claims its own lane's script.
    for first in [RaceLane::B, RaceLane::A] {
        let dir = tempdir();
        let repo = repo(dir.path());
        role_script(&repo, "racer-t1-a-1", &[json!({"print": "lane a"})]);
        role_script(&repo, "racer-t1-b-1", &[json!({"print": "lane b"})]);
        role_script(&repo, "worker-t1-1", &[json!({"print": "worker"})]);
        let second = match first {
            RaceLane::A => RaceLane::B,
            RaceLane::B => RaceLane::A,
        };
        for lane in [first, second] {
            let said = session(&repo, target(AgentRole::Racer, Some(lane)));
            assert_eq!(said, [format!("lane {}", lane.label())], "{first:?} first");
        }
        assert!(claimed(&repo, "racer-t1-a-1") && claimed(&repo, "racer-t1-b-1"));
        assert!(!claimed(&repo, "worker-t1-1"), "a racer is not a worker");
    }
}

#[test]
fn test_writer_scripts_are_claimed() {
    let dir = tempdir();
    let repo = repo(dir.path());
    role_script(&repo, "test_writer-t1-2", &[json!({"print": "writer 2"})]);
    role_script(&repo, "test_writer-t1-1", &[json!({"print": "writer 1"})]);
    role_script(&repo, "worker-t1-1", &[json!({"print": "implementer"})]);
    for expected in ["writer 1", "writer 2"] {
        let said = session(&repo, target(AgentRole::TestWriter, None));
        assert_eq!(said, [expected]);
    }
    assert!(
        !claimed(&repo, "worker-t1-1"),
        "a test writer is not a worker"
    );
    // The implementer that follows is a worker and takes the worker's script.
    let said = session(&repo, target(AgentRole::Worker, None));
    assert_eq!(said, ["implementer"]);
    let scripts: Vec<String> = fs::read_dir(repo.join(".git/fake-agent"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".claimed"))
        .collect();
    assert_eq!(scripts.len(), 3, "{scripts:?}");
}
