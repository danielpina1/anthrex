//! Milestone 9.6 task M9.6.19 (ruling T1-O3): `fake-agent`'s scripts for the design
//! flow's headless agents, keyed by the `--agent-label` their `anthrex mcp` server
//! names. A brainstormer's is `brainstormer-<label>-<n>.jsonl` (`claude`, `codex`, `A`,
//! `B`); a document reviewer's is `doc_reviewer-<doc>-r<k>-<n>.jsonl`, `k` the run-wide
//! review number (rulings T10-1, T15-9). Each is claimed from the argv the daemon's
//! headless launcher builds for that role (`daemon::headless::argv`, with T8-4's
//! `--no-session-persistence` and `--ephemeral`). Real pipes, a real git repository;
//! the reviewer's MCP calls go through the real `anthrex mcp` to a stub daemon.

mod headless_support;

use std::path::{Path, PathBuf};

use daemon::headless::McpTarget;
use daemon::scout::design_spec::BRAINSTORMER_NUDGE;
use headless_support::*;
use proto::AgentRole;
use serde_json::{Value, json};

const CLAUDE_ID: &str = "00000000-0000-4000-8000-000000000096";

/// The MCP target of run `r1`'s design agent in `role`, labelled `label`.
fn target(role: AgentRole, label: &str) -> McpTarget {
    McpTarget {
        role,
        run_id: "r1".into(),
        task_id: None,
        scout_id: None,
        epic: None,
        chain: None,
        lane: None,
        agent_label: Some(label.into()),
    }
}

fn unused() -> (PathBuf, PathBuf) {
    (
        PathBuf::from("/nonexistent/anthrex"),
        PathBuf::from("/tmp/nonexistent.sock"),
    )
}

/// One Codex process of `target` in `repo` (a first turn: Codex design agents are
/// never resumed, ruling T8-7): what it printed, and its argv.
fn codex(repo: &Path, target: McpTarget) -> (Vec<String>, Vec<String>) {
    let (exe, socket) = unused();
    let argv = codex_argv_for(target, &exe, &socket, "go");
    let mut agent = Agent::codex(&argv, repo, &[]);
    assert!(agent.wait(RUN).success(), "{}", agent.stderr());
    (texts(&agent), argv)
}

fn claimed(repo: &Path, name: &str) -> bool {
    repo.join(format!(".git/fake-agent/{name}.jsonl.claimed"))
        .exists()
}

fn print(text: &str) -> Value {
    json!({"print": text})
}

#[test]
fn fake_agent_claims_brainstormer_scripts_by_label() {
    let dir = tempdir();
    let repo = repo(dir.path());
    // Claude's turn ends without a draft, and its nudge arrives on the same process's
    // stdin (ruling T8-4): the same script goes on.
    let nudge = json!({"read_message": {"expect": BRAINSTORMER_NUDGE}});
    let claude_steps = [
        print("claude 1"),
        json!({"end_turn": {}}),
        nudge,
        print("claude nudged"),
    ];
    role_script(&repo, "brainstormer-claude-1", &claude_steps);
    // Codex's first attempt ends without submitting; its fresh relaunch (ruling T8-7)
    // is a new process and takes the next script.
    role_script(
        &repo,
        "brainstormer-codex-1",
        &[print("codex 1"), json!({"end_turn": {}})],
    );
    role_script(&repo, "brainstormer-codex-2", &[print("codex 2")]);
    role_script(&repo, "brainstormer-A-1", &[print("lens A")]);
    role_script(
        &repo,
        "brainstormer-B-1",
        &[json!({"fail_turn": {"error": "overloaded"}})],
    );
    role_script(
        &repo,
        "doc_reviewer-claude-1",
        &[print("not a brainstormer's")],
    );

    let (said, argv) = codex(&repo, target(AgentRole::Brainstormer, "codex"));
    assert_eq!(said, ["codex 1"]);
    assert!(argv.contains(&"--ephemeral".to_string()), "{argv:?}");
    let (said, _) = codex(&repo, target(AgentRole::Brainstormer, "codex"));
    assert_eq!(said, ["codex 2"], "the relaunch takes the next script");

    let (exe, socket) = unused();
    let argv = claude_argv_for(
        target(AgentRole::Brainstormer, "claude"),
        &exe,
        &socket,
        CLAUDE_ID,
    );
    assert!(
        argv.contains(&"--no-session-persistence".to_string()),
        "{argv:?}"
    );
    let mut agent = Agent::spawn(&argv, &repo, &[]);
    agent.send(&user_message("brainstorm", CLAUDE_ID));
    agent.until(RUN, is_result);
    agent.send(&user_message(BRAINSTORMER_NUDGE, CLAUDE_ID));
    agent.until(RUN, is_result);
    agent.close_stdin();
    assert!(agent.wait(RUN).success(), "{}", agent.stderr());
    assert_eq!(texts(&agent), ["claude 1", "claude nudged"]);

    let (said, _) = codex(&repo, target(AgentRole::Brainstormer, "A"));
    assert_eq!(said, ["lens A"]);
    let (exe, socket) = unused();
    let argv = codex_argv_for(target(AgentRole::Brainstormer, "B"), &exe, &socket, "go");
    let mut agent = Agent::codex(&argv, &repo, &[]);
    agent.wait(RUN);
    let failed = agent.of_type("turn.failed");
    assert_eq!(
        failed.len(),
        1,
        "B's script fails its turn: {:?}",
        agent.seen
    );

    for name in [
        "brainstormer-claude-1",
        "brainstormer-codex-1",
        "brainstormer-codex-2",
        "brainstormer-A-1",
        "brainstormer-B-1",
    ] {
        assert!(claimed(&repo, name), "{name} was claimed");
    }
    assert!(
        !claimed(&repo, "doc_reviewer-claude-1"),
        "a reviewer's script is its own"
    );
}

#[test]
fn fake_agent_claims_reviewer_scripts_by_doc_and_round() {
    let dir = tempdir();
    let repo = repo(dir.path());
    let stub = StubDaemon::start(true, "{}");
    let finding = json!({"id": "F1", "severity": "minor", "place": "R2", "text": "vague"});
    role_script(
        &repo,
        "doc_reviewer-spec-r2-1",
        &[
            print("spec review 2"),
            json!({"mcp_call": {"tool": "get_doc", "args": {"kind": "spec", "draft": 2}}}),
            json!({"mcp_call": {"tool": "submit_findings", "args": {"findings": [finding]}}}),
        ],
    );
    role_script(
        &repo,
        "doc_reviewer-spec-r1-1",
        &[print("spec review 1"), json!({"end_turn": {}})],
    );
    role_script(
        &repo,
        "doc_reviewer-spec-r1-2",
        &[print("spec review 1, relaunched")],
    );
    role_script(&repo, "doc_reviewer-spec-r10-1", &[print("spec review 10")]);
    role_script(&repo, "doc_reviewer-plan-r1-1", &[print("plan review 1")]);
    role_script(
        &repo,
        "brainstormer-spec-r1-1",
        &[print("not a reviewer's")],
    );

    // Review 2's reviewer reads its draft and submits, through the real `anthrex mcp`.
    let argv = codex_argv_for(
        target(AgentRole::DocReviewer, "spec-r2"),
        &anthrex(),
        &stub.socket,
        "review",
    );
    assert!(argv.contains(&"--ephemeral".to_string()), "{argv:?}");
    let mut agent = Agent::codex(&argv, &repo, &[]);
    assert!(agent.wait(MCP_RUN).success(), "{}", agent.stderr());
    assert_eq!(texts(&agent), ["spec review 2"]);
    let calls: Vec<(AgentRole, String, Value)> = stub
        .calls()
        .into_iter()
        .map(|c| (c.role, c.tool, c.args))
        .collect();
    assert_eq!(
        calls,
        [
            (
                AgentRole::DocReviewer,
                "get_doc".to_string(),
                json!({"kind": "spec", "draft": 2})
            ),
            (
                AgentRole::DocReviewer,
                "submit_findings".to_string(),
                json!({"findings": [finding]})
            ),
        ]
    );

    for (label, expected) in [
        ("spec-r1", "spec review 1"),
        ("spec-r1", "spec review 1, relaunched"),
        ("spec-r10", "spec review 10"),
        ("plan-r1", "plan review 1"),
    ] {
        let (said, _) = codex(&repo, target(AgentRole::DocReviewer, label));
        assert_eq!(said, [expected], "{label}");
    }
    for name in [
        "doc_reviewer-spec-r1-1",
        "doc_reviewer-spec-r1-2",
        "doc_reviewer-spec-r2-1",
        "doc_reviewer-spec-r10-1",
        "doc_reviewer-plan-r1-1",
    ] {
        assert!(claimed(&repo, name), "{name} was claimed");
    }
    assert!(
        !claimed(&repo, "brainstormer-spec-r1-1"),
        "a brainstormer's script is its own"
    );
}
