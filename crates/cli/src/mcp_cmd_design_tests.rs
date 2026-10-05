//! Milestone 9.6 task M9.6.6: `anthrex mcp --role brainstormer|doc_reviewer`, and ruling
//! T1-O3's `--agent-label`, which a design agent's argv names and the server ignores.

use crate::{Cli, Command};
use clap::Parser;
use daemon::headless::McpTarget;
use proto::AgentRole;
use std::path::{Path, PathBuf};

fn options(argv: &[&str]) -> Result<mcp::McpOptions, String> {
    let mut full = vec!["anthrex", "mcp"];
    full.extend_from_slice(argv);
    match Cli::try_parse_from(&full)
        .map_err(|e| e.to_string())?
        .command
    {
        Some(Command::Mcp(args)) => args
            .into_options(PathBuf::from("/tmp/d.sock"))
            .map_err(|e| e.to_string()),
        other => panic!("expected Mcp, got {other:?}"),
    }
}

/// The daemon's argv for a brainstormer and a document reviewer parses, with and
/// without a label; the label reaches no option (the daemon never reads it back).
#[test]
fn design_roles_parse_the_daemons_argv() {
    let socket = Path::new("/tmp/anthrex-x/d.sock");
    for (role, label) in [
        (AgentRole::Brainstormer, Some("codex")),
        (AgentRole::Brainstormer, Some("A")),
        (AgentRole::Brainstormer, None),
        (AgentRole::DocReviewer, Some("spec-r1")),
        (AgentRole::DocReviewer, None),
    ] {
        let target = McpTarget {
            role,
            run_id: "add-reset-3f9a".into(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
            agent_label: label.map(String::from),
        };
        let argv = daemon::headless::argv::mcp_args(&target, 12, socket).expect("an mcp role");
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        assert_eq!(
            options(&argv[1..]).unwrap_or_else(|e| panic!("{argv:?}: {e}")),
            mcp::McpOptions {
                role,
                run_id: "add-reset-3f9a".into(),
                task_id: None,
                scout_id: None,
                epic: None,
                chain: None,
                lane: None,
                window_id: 12,
                socket: socket.to_path_buf(),
            },
            "{argv:?}"
        );
    }
}

/// A design agent belongs to a run; `--agent-label` is a design agent's alone.
#[test]
fn design_roles_need_a_run_and_only_they_take_a_label() {
    for role in ["brainstormer", "doc_reviewer"] {
        let error = options(&["--role", role, "--window", "1"]).expect_err("no run");
        assert!(error.contains("--run"), "{role}: {error}");
    }
    for argv in [
        &["--role", "worker", "--run", "r", "--task", "t1"][..],
        &["--role", "orchestrator", "--run", "r"],
        &["--role", "scout", "--scout", "s-1"],
    ] {
        let mut argv = argv.to_vec();
        argv.extend(["--agent-label", "A", "--window", "1"]);
        let error = options(&argv).expect_err("a label outside a design role");
        assert!(
            error.contains(
                "--agent-label <text> is accepted only with --role brainstormer or doc_reviewer"
            ),
            "{argv:?}: {error}"
        );
    }
}
