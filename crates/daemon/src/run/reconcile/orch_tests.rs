//! Milestone 9's reconcile rows (Interfaces "Reconcile rows"; task M9.7): an
//! orchestrator window the restart restored answers its `CreateOrchestrator`; a run
//! scout is never replayed, and nothing is killed for it.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::Duration;

use proto::{AgentRole, RunRef, Runtime, Status, WindowInfo, WindowKind, WindowSpec};

use super::{Reconciled, reconcile};
use crate::headless::McpTarget;
use crate::launch::role::RoleLaunch;
use crate::run::engine::{OpKind, OpResult};
use crate::run::journal::JournalLine;
use crate::run::model::{PendingOp, Run};
use crate::run::test_support::{EXAMPLE_PLAN, run_ok};

fn window(id: u32, kind: WindowKind, run: Option<RunRef>) -> WindowInfo {
    WindowInfo {
        id,
        name: format!("w{id}"),
        runtime: Runtime::Claude,
        cwd: PathBuf::from("/tmp/x"),
        project: PathBuf::from("/tmp/p"),
        worktree: None,
        branch: None,
        status: Status::Idle,
        tool: None,
        since_secs: 0,
        last_output_secs: 0,
        session_id: None,
        model: None,
        subagents: Vec::new(),
        exit: None,
        kind,
        run,
        signals_seen: false,
        placeholder: false,
    }
}

fn orchestrator_ref(run: &str, session: u32) -> RunRef {
    RunRef {
        run_id: run.into(),
        task_id: None,
        role: AgentRole::Orchestrator,
        session,
        lane: None,
    }
}

/// `run` with `kind` pending as op 4, its intent journaled and no result.
fn intent_only(kind: OpKind) -> (Run, Vec<JournalLine>) {
    let mut run = run_ok(EXAMPLE_PLAN);
    run.pending_ops.insert(
        4,
        PendingOp {
            op: 4,
            task_id: None,
            kind: kind.clone(),
        },
    );
    (run, vec![JournalLine::Intent { op: 4, kind }])
}

fn create(run_id: &str) -> OpKind {
    let r = orchestrator_ref(run_id, 1);
    OpKind::CreateOrchestrator {
        spec: Box::new(WindowSpec {
            name: Some("3f9a/orchestrator".into()),
            runtime: Runtime::Claude,
            cwd: PathBuf::from("/tmp/x"),
            worktree_branch: None,
            model: None,
            initial_prompt: Some("plan".into()),
        }),
        role: Box::new(RoleLaunch {
            mcp: McpTarget {
                role: AgentRole::Orchestrator,
                run_id: r.run_id.clone(),
                task_id: None,
                scout_id: None,
                epic: None,
                chain: None,
                lane: None,
            },
            run_ref: r,
            instructions: String::new(),
            effort: proto::Effort::High,
            claude_allowed_tools: Vec::new(),
            claude_disallowed_tools: Vec::new(),
            env: Vec::new(),
            remove_env: Vec::new(),
        }),
        project: PathBuf::from("/tmp/p"),
    }
}

const GIT: &str = "/nonexistent/anthrex-test/git";

#[test]
fn create_orchestrator_is_replayed_from_a_restored_window() {
    let run_id = run_ok(EXAMPLE_PLAN).id;
    let (run, journal) = intent_only(create(&run_id));
    let windows = vec![
        // Another run's orchestrator, a headless window of this run, and a plain PTY.
        window(3, WindowKind::Pty, Some(orchestrator_ref("other-1a2b", 1))),
        window(5, WindowKind::Headless, Some(orchestrator_ref(&run_id, 1))),
        window(6, WindowKind::Pty, None),
        // Ours, restored with a later session after a restart.
        window(8, WindowKind::Pty, Some(orchestrator_ref(&run_id, 2))),
    ];
    let git = OsStr::new(GIT);
    let out = reconcile(git, &run, &journal, &windows, Duration::from_secs(1));
    assert_eq!(
        out.ops,
        vec![(
            4,
            Reconciled::Replay(OpResult::Window {
                window_id: 8,
                pid: None
            })
        )]
    );
    // No restored window: it was not started.
    let out = reconcile(git, &run, &journal, &windows[..3], Duration::from_secs(1));
    assert_eq!(out.ops, vec![(4, Reconciled::NotStarted)]);
    assert!(out.notes.is_empty(), "{:?}", out.notes);
}

#[test]
fn start_scout_is_not_replayed() {
    let base = run_ok(EXAMPLE_PLAN);
    let spec = crate::scout::spec::ScoutSpec {
        id: "3f9a-daemon".into(),
        kind: proto::ScoutKind::Area,
        run_id: Some(base.id.clone()),
        question: "What is in the daemon?".into(),
        first_turn: "scout".into(),
        cwd: PathBuf::from("/tmp/x"),
        project: PathBuf::from("/tmp/p"),
        web: false,
        codex_config: Vec::new(),
        base_sha: "b".repeat(40),
        repo_paths: Vec::new(),
    };
    let (run, journal) = intent_only(OpKind::StartScout {
        spec: Box::new(spec),
    });
    // Even a headless window that looks like the scout's is not taken as its result.
    let scout = RunRef {
        run_id: run.id.clone(),
        task_id: None,
        role: AgentRole::Scout,
        session: 1,
        lane: None,
    };
    let windows = vec![window(9, WindowKind::Headless, Some(scout))];
    let out = reconcile(
        OsStr::new(GIT),
        &run,
        &journal,
        &windows,
        Duration::from_secs(1),
    );
    assert_eq!(out.ops, vec![(4, Reconciled::NotStarted)]);
    assert!(out.notes.is_empty(), "nothing is killed: {:?}", out.notes);
    // A restart of the orchestrator and a target resolve are issued again.
    for kind in [
        OpKind::RestartOrchestrator { window_id: 3 },
        OpKind::ResolveTarget {
            root: PathBuf::from("/tmp/x"),
            target: "main..feature".into(),
            base_branch: "main".into(),
        },
    ] {
        let (run, journal) = intent_only(kind);
        let out = reconcile(OsStr::new(GIT), &run, &journal, &[], Duration::from_secs(1));
        assert_eq!(out.ops, vec![(4, Reconciled::NotStarted)]);
    }
}
