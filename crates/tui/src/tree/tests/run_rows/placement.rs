//! Milestone 8c task 4: where the run view hangs a window's sub-agents (decision 12),
//! and how they pass the `f` filter (decision 21).

use super::*;

fn running_subagent(id: &str) -> proto::SubagentInfo {
    subagent(id, None, "Explore", "grep", SubagentState::Running, 1)
}

#[test]
fn a_planners_sub_agents_follow_its_tasks() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mut daemon = planner("A", "daemon");
    daemon.window_id = Some(8);
    info.planners = vec![daemon];
    info.tasks = vec![in_wave(pending("t1"), 0, Some("A"))];
    let mut window = headless(8, "3f9a/plan-A", PROJECT, None);
    window.subagents = vec![running_subagent("x")];
    let windows = [window];

    let rows = run_rows(&info, &windows, &TreeState::default(), RunFilter::All);
    assert_eq!(
        keys_and_depths(&rows),
        vec![(root(), 0), (pl("A"), 1), (t("t1"), 2), (sub(8, "x"), 2)]
    );
    // `t1` is not the planner's last child: its sub-agent follows it.
    let guides: Vec<&str> = rows.iter().map(|row| row.guides.as_str()).collect();
    assert_eq!(guides, ["", "└─", "  ├─", "  └─"]);
}

#[test]
fn the_orchestrators_window_never_gets_sub_agents() {
    let (mut snapshot, mut windows) = three_task_fixture();
    // `t1`'s worker names window 3, the orchestrator's, which has a sub-agent.
    snapshot.runs[0].tasks[1].rounds[0].window_id = Some(3);
    assert_eq!(windows[1].id, 3);
    windows[1].subagents = vec![running_subagent("o")];

    let rows = run_rows(
        &snapshot.runs[0],
        &windows,
        &TreeState::default(),
        RunFilter::All,
    );
    assert_unique(&rows);
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row.kind, RowKind::Subagent { .. })),
        "no sub-agent of the orchestrator's window is drawn"
    );
    let window = rows.iter().find_map(|row| match &row.kind {
        RowKind::AgentRound { task, round, .. } if task.id == "t1" => Some(round.window),
        _ => None,
    });
    assert_eq!(window.flatten().map(|window| window.id), Some(3));
}

#[test]
fn a_finished_rounds_sub_agents_do_not_keep_it_in_the_running_view() {
    let (snapshot, mut windows) = three_task_fixture();
    // `t0`'s ended worker window 4 is still listed (inside `RETIRE_AFTER`).
    let mut retired = headless(
        4,
        "3f9a/t0.w1",
        PROJECT,
        Some(run_ref(RUN_ID, Some("t0"), AgentRole::Worker, 1)),
    );
    retired.subagents = vec![running_subagent("late")];
    windows.push(retired);
    let info = &snapshot.runs[0];

    let all = keys(&run_rows(
        info,
        &windows,
        &TreeState::default(),
        RunFilter::All,
    ));
    assert!(all.contains(&sub(4, "late")), "drawn under t0's worker");
    assert_eq!(
        keys_and_depths(&run_rows(
            info,
            &windows,
            &TreeState::default(),
            RunFilter::Running
        )),
        vec![
            (root(), 0),
            (t("t1"), 1),
            (rd("t1", AgentRole::Worker, 1, 1), 2),
        ]
    );
}
