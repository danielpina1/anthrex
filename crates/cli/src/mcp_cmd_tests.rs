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
    for (role, run, task, scout, epic) in [
        (AgentRole::Worker, "add-reset-3f9a", Some("t1"), None, None),
        (
            AgentRole::Reviewer,
            "add-reset-3f9a",
            Some("t2"),
            None,
            None,
        ),
        (AgentRole::Orchestrator, "add-reset-3f9a", None, None, None),
        (
            AgentRole::Planner,
            "add-reset-3f9a",
            None,
            None,
            Some("mail"),
        ),
        (
            AgentRole::Scout,
            "add-reset-3f9a",
            None,
            Some("api-1"),
            None,
        ),
        (AgentRole::Scout, "", None, Some("onboarding-1"), None),
        // Milestone 9 decision 35: a research task's scout window names its task.
        (AgentRole::Scout, "add-reset-3f9a", Some("t3"), None, None),
    ] {
        let target = McpTarget {
            role,
            run_id: run.into(),
            task_id: task.map(String::from),
            scout_id: scout.map(String::from),
            epic: epic.map(String::from),
        };
        let mut argv = vec!["anthrex".to_string()];
        argv.extend(daemon::headless::argv::mcp_args(&target, 12, socket).expect("an mcp role"));
        match Cli::try_parse_from(&argv)
            .unwrap_or_else(|e| panic!("{argv:?}: {e}"))
            .command
        {
            Some(Command::Mcp(args)) => {
                let opts = args
                    .into_options(PathBuf::from("/tmp/unused.sock"))
                    .unwrap_or_else(|e| panic!("{argv:?}: {e}"));
                assert_eq!(
                    opts,
                    mcp::McpOptions {
                        role,
                        run_id: run.into(),
                        task_id: task.map(String::from),
                        scout_id: scout.map(String::from),
                        epic: epic.map(String::from),
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
            let opts = args
                .into_options(PathBuf::from("/tmp/default.sock"))
                .unwrap();
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
            let opts = args.into_options(PathBuf::from("/tmp/d.sock")).unwrap();
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
        "anthrex", "mcp", "--role", "planner", "--run", "r1", "--epic", "a", "--window", "3",
    ];
    match Cli::try_parse_from(argv).unwrap().command {
        Some(Command::Mcp(args)) => {
            let opts = args.into_options(PathBuf::from("/tmp/d.sock")).unwrap();
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

/// Parses `argv` and turns it into options: `Err` holds the usage error's text.
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

/// Milestone 9, Interfaces "MCP": `--epic` is required for a planner and refused for
/// every other role.
#[test]
fn planner_needs_epic_and_orchestrator_refuses_it() {
    let error = options(&["--role", "planner", "--run", "r1", "--window", "3"])
        .expect_err("a planner without --epic");
    assert!(error.contains("--epic"), "{error}");
    let opts = options(&[
        "--role", "planner", "--run", "r1", "--epic", "mail", "--window", "3",
    ])
    .unwrap();
    assert_eq!(opts.epic.as_deref(), Some("mail"));
    assert_eq!(opts.role, AgentRole::Planner);
    for role in ["orchestrator", "worker", "reviewer"] {
        let error = options(&[
            "--role", role, "--run", "r1", "--epic", "mail", "--window", "3",
        ])
        .expect_err("--epic outside the planner");
        assert!(error.contains("--epic"), "{role}: {error}");
    }
    let error = options(&[
        "--role", "scout", "--scout", "s-1", "--epic", "mail", "--window", "3",
    ])
    .expect_err("--epic for a scout");
    assert!(error.contains("--epic"), "{error}");
    let opts = options(&["--role", "orchestrator", "--run", "r1", "--window", "3"]).unwrap();
    assert_eq!(opts.epic, None);
}

/// Milestone 9 defect 19 (decision 35): `--role scout` takes exactly one of `--scout`
/// and `--task`; neither or both is a usage error naming the two flags.
#[test]
fn scout_accepts_task_instead_of_scout() {
    let opts = options(&[
        "--role", "scout", "--run", "r1", "--task", "t3", "--window", "4",
    ])
    .unwrap();
    assert_eq!(opts.task_id.as_deref(), Some("t3"));
    assert_eq!(opts.scout_id, None);
    assert_eq!(opts.run_id, "r1");
    for argv in [
        &["--role", "scout", "--run", "r1", "--window", "4"][..],
        &[
            "--role", "scout", "--run", "r1", "--task", "t3", "--scout", "s-1", "--window", "4",
        ][..],
    ] {
        let error = options(argv).expect_err("neither or both");
        assert!(
            error.contains("--scout") && error.contains("--task"),
            "{argv:?}: {error}"
        );
    }
    let opts = options(&["--role", "scout", "--scout", "s-1", "--window", "4"]).unwrap();
    assert_eq!(opts.scout_id.as_deref(), Some("s-1"));
    assert_eq!(opts.task_id, None);
}
