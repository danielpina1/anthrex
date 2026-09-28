//! `anthrex mcp`'s argument tests (moved out of `mcp_cmd.rs` with M8b.9's additions,
//! to keep that file within its size budget).

use crate::{Cli, Command};
use clap::Parser;
use daemon::headless::McpTarget;
use proto::AgentRole;
use std::path::{Path, PathBuf};

/// The argv the daemon's headless launcher builds is exactly what `anthrex mcp`
/// parses, with and without a task; and `mcp` stays out of the help.
#[test]
fn mcp_parses_the_daemons_headless_argv_and_is_hidden() {
    let socket = Path::new("/tmp/anthrex-x/d.sock");
    for (role, run, task, scout) in [
        (AgentRole::Worker, "add-reset-3f9a", Some("t1"), None),
        (AgentRole::Reviewer, "add-reset-3f9a", Some("t2"), None),
        (AgentRole::Orchestrator, "add-reset-3f9a", None, None),
        (AgentRole::Planner, "add-reset-3f9a", None, None),
        (AgentRole::Scout, "add-reset-3f9a", None, Some("api-1")),
        (AgentRole::Scout, "", None, Some("onboarding-1")),
    ] {
        let target = McpTarget {
            role,
            run_id: run.into(),
            task_id: task.map(String::from),
            scout_id: scout.map(String::from),
            epic: None,
        };
        let mut argv = vec!["anthrex".to_string()];
        argv.extend(daemon::headless::argv::mcp_args(&target, 12, socket).expect("an mcp role"));
        match Cli::try_parse_from(&argv)
            .unwrap_or_else(|e| panic!("{argv:?}: {e}"))
            .command
        {
            Some(Command::Mcp(args)) => {
                let opts = args.into_options(PathBuf::from("/tmp/unused.sock"));
                assert_eq!(
                    opts,
                    mcp::McpOptions {
                        role,
                        run_id: run.into(),
                        task_id: task.map(String::from),
                        scout_id: scout.map(String::from),
                        window_id: 12,
                        socket: socket.to_path_buf(),
                    }
                );
            }
            other => panic!("expected Mcp, got {other:?}"),
        }
    }

    let help = Cli::try_parse_from(["anthrex", "--help"])
        .err()
        .expect("help exits")
        .to_string();
    assert!(!help.contains("mcp"), "mcp must be hidden: {help}");
}

#[test]
fn mcp_socket_defaults_to_the_daemons() {
    let argv = [
        "anthrex", "mcp", "--role", "worker", "--run", "r", "--window", "1",
    ];
    match Cli::try_parse_from(argv).unwrap().command {
        Some(Command::Mcp(args)) => {
            let opts = args.into_options(PathBuf::from("/tmp/default.sock"));
            assert_eq!(opts.socket, PathBuf::from("/tmp/default.sock"));
            assert_eq!(opts.task_id, None);
        }
        other => panic!("expected Mcp, got {other:?}"),
    }
}

/// M8b decision 15: `--run` may be left out for a scout only.
#[test]
fn run_is_required_except_for_scouts() {
    for role in ["worker", "reviewer", "orchestrator", "planner"] {
        let argv = ["anthrex", "mcp", "--role", role, "--window", "1"];
        let error = match Cli::try_parse_from(argv) {
            Ok(_) => panic!("{role} parsed without --run"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("--run"), "{role}: {error}");
    }
    let argv = [
        "anthrex",
        "mcp",
        "--role",
        "scout",
        "--scout",
        "onboarding-1",
        "--window",
        "7",
    ];
    match Cli::try_parse_from(argv).unwrap().command {
        Some(Command::Mcp(args)) => {
            let opts = args.into_options(PathBuf::from("/tmp/d.sock"));
            assert_eq!(opts.run_id, "");
            assert_eq!(opts.scout_id.as_deref(), Some("onboarding-1"));
            assert_eq!(opts.role, AgentRole::Scout);
        }
        other => panic!("expected Mcp, got {other:?}"),
    }
}

/// M9.2 review ruling 6: the daemon can build a planner's `anthrex mcp`, so it parses.
/// `decider` is not a role `anthrex mcp` accepts: a decider never runs it.
#[test]
fn mcp_role_planner_parses_and_decider_does_not() {
    let argv = [
        "anthrex", "mcp", "--role", "planner", "--run", "r1", "--window", "3",
    ];
    match Cli::try_parse_from(argv).unwrap().command {
        Some(Command::Mcp(args)) => {
            let opts = args.into_options(PathBuf::from("/tmp/d.sock"));
            assert_eq!(opts.role, AgentRole::Planner);
            assert_eq!(opts.run_id, "r1");
        }
        other => panic!("expected Mcp, got {other:?}"),
    }
    let argv = [
        "anthrex", "mcp", "--role", "decider", "--run", "r1", "--window", "3",
    ];
    assert!(Cli::try_parse_from(argv).is_err(), "decider must not parse");
}
