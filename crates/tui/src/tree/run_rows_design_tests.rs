//! Task M9.6.18, ruling T18-1: the run view lists a design run's brainstormers and
//! document reviewer below the run, before its tasks, one row an agent with its session
//! count (a ruling T8-7 relaunch stays one row); the `f` filter and the text filter
//! take them like any agent. A run without the flow lists none.

use super::*;
use crate::tree::run_fixtures::{PROJECT, RUN_ID, headless, task, three_task_fixture};
use proto::{DesignAgentInfo, DesignAgentStatus, DocGateKind, DocKind, Size};

pub(crate) fn agents() -> Vec<DesignAgentInfo> {
    let agent = |role, label: &str, runtime, state, sessions| DesignAgentInfo {
        role,
        label: label.into(),
        runtime,
        state,
        sessions,
        window_id: None,
        doc: None,
        review: None,
    };
    vec![
        agent(
            AgentRole::Brainstormer,
            "claude",
            Runtime::Claude,
            DesignAgentStatus::Failed,
            1,
        ),
        DesignAgentInfo {
            window_id: Some(41),
            ..agent(
                AgentRole::Brainstormer,
                "codex",
                Runtime::Codex,
                DesignAgentStatus::Running,
                2,
            )
        },
        DesignAgentInfo {
            doc: Some(DocKind::Spec),
            review: Some(2),
            ..agent(
                AgentRole::DocReviewer,
                "spec-r2",
                Runtime::Codex,
                DesignAgentStatus::Done,
                1,
            )
        },
    ]
}

/// The spec gate's design run with its agents and one task.
pub(crate) fn with_agents() -> RunInfo {
    let mut run = crate::ui::doc_gate::tests::design_run(DocGateKind::Spec);
    run.tasks = vec![task("t1", "reset", Size::S, TaskState::Pending)];
    run.design_agents = agents();
    run
}

fn texts(rows: &[Row<'_>]) -> Vec<String> {
    rows.iter().map(crate::graph::content_text).collect()
}

#[test]
fn the_run_view_lists_each_design_agent_once() {
    let run = with_agents();
    let windows = vec![headless(41, "3f9a/brainstorm-codex.2", PROJECT, None)];
    let rows = run_rows(&run, &windows, &TreeState::default(), RunFilter::All);
    let key = |label: &str| NodeKey::DesignAgent {
        run: RUN_ID.into(),
        label: label.into(),
    };
    let keys: Vec<NodeKey> = rows.iter().map(|r| r.key.clone()).collect();
    assert_eq!(
        keys[1..],
        [
            key("claude"),
            key("codex"),
            key("spec-r2"),
            NodeKey::Task {
                run: RUN_ID.into(),
                id: "t1".into()
            }
        ]
    );
    assert_eq!(
        texts(&rows)[1..4],
        [
            "brainstormer claude",
            "brainstormer codex  2 sessions",
            "doc reviewer spec-r2 codex",
        ]
    );
    // The relaunched brainstormer's window is its row's.
    let RowKind::DesignAgent { window, .. } = &rows[2].kind else {
        panic!("{:?}", rows[2].kind);
    };
    assert_eq!(window.map(|w| w.id), Some(41));
}

#[test]
fn the_filters_take_the_design_agents() {
    let run = with_agents();
    let labels = |filter, text: &str| {
        let state = TreeState {
            filter: text.into(),
            ..TreeState::default()
        };
        texts(&run_rows(&run, &[], &state, filter))
    };
    assert_eq!(
        labels(RunFilter::Running, "")[1..],
        ["brainstormer codex  2 sessions"]
    );
    assert_eq!(
        labels(RunFilter::Runtime(Runtime::Claude), "")[1..],
        ["brainstormer claude", "t1 reset S"]
    );
    assert_eq!(
        labels(RunFilter::All, "spec-r2")[1..],
        ["doc reviewer spec-r2 codex"]
    );
}

#[test]
fn a_run_without_the_flow_lists_no_design_agents() {
    let (snap, windows) = three_task_fixture();
    let rows = run_rows(
        &snap.runs[0],
        &windows,
        &TreeState::default(),
        RunFilter::All,
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::DesignAgent { .. }))
    );
}
