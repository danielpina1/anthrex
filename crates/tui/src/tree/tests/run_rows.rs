//! Milestone 8c task 4: the run view's rows (decisions 12–15 and 21).

use super::super::*;
use super::run_fixtures::{
    PROJECT, RUN_ID, headless, planner, reviewer, run, run_ref, scout, task, three_task_fixture,
    worker,
};
use super::subagent;
use proto::{AgentRole, RunState, Runtime, ScoutState, Size, SubagentState, TaskState};
use std::collections::HashSet;

fn keys_and_depths(rows: &[Row<'_>]) -> Vec<(NodeKey, u16)> {
    rows.iter()
        .map(|row| (row.key.clone(), row.depth))
        .collect()
}

fn keys(rows: &[Row<'_>]) -> Vec<NodeKey> {
    rows.iter().map(|row| row.key.clone()).collect()
}

fn root() -> NodeKey {
    NodeKey::Run(RUN_ID.into())
}

fn t(id: &str) -> NodeKey {
    NodeKey::Task {
        run: RUN_ID.into(),
        id: id.into(),
    }
}

fn sc(id: &str) -> NodeKey {
    NodeKey::Scout {
        run: RUN_ID.into(),
        id: id.into(),
    }
}

fn pl(epic: &str) -> NodeKey {
    NodeKey::Planner {
        run: RUN_ID.into(),
        epic: epic.into(),
    }
}

fn rd(task: &str, role: AgentRole, session: u32, round: u32) -> NodeKey {
    NodeKey::AgentRound {
        run: RUN_ID.into(),
        task: task.into(),
        role,
        session,
        round,
    }
}

fn sub(window_id: u32, id: &str) -> NodeKey {
    NodeKey::Subagent {
        window_id,
        id: id.into(),
    }
}

fn round_labels(rows: &[Row<'_>]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match &row.kind {
            RowKind::AgentRound { round, .. } => Some(round_label(
                round.info.role,
                round.info.session,
                round.number,
            )),
            _ => None,
        })
        .collect()
}

fn assert_unique(rows: &[Row<'_>]) {
    let mut seen = HashSet::new();
    for row in rows {
        assert!(seen.insert(row.key.clone()), "repeated key {:?}", row.key);
    }
}

fn rows_of(run: &proto::RunInfo, filter: RunFilter) -> Vec<(NodeKey, u16)> {
    keys_and_depths(&run_rows(run, &[], &TreeState::default(), filter))
}

fn in_wave(mut info: proto::TaskInfo, wave: u32, epic: Option<&str>) -> proto::TaskInfo {
    info.wave = wave;
    info.epic = epic.map(str::to_owned);
    info
}

fn pending(id: &str) -> proto::TaskInfo {
    task(id, id, Size::S, TaskState::Pending)
}

#[test]
fn tiers_follow_the_information_flow() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    info.scouts = vec![
        scout("S2", "where are hooks parsed?", Runtime::Claude, 20),
        scout("S1", "what owns the socket?", Runtime::Claude, 10),
    ];
    info.planners = vec![planner("A", "daemon")];
    // Plan order puts t6 first, so only the wave can put it last.
    info.tasks = vec![
        in_wave(pending("t6"), 2, None),
        in_wave(pending("t0"), 0, None),
        in_wave(pending("t1"), 1, Some("A")),
        in_wave(pending("t2"), 1, Some("A")),
    ];

    assert_eq!(
        rows_of(&info, RunFilter::All),
        vec![
            (root(), 0),
            (sc("S1"), 1),
            (sc("S2"), 1),
            (t("t0"), 1),
            (pl("A"), 1),
            (t("t1"), 2),
            (t("t2"), 2),
            (t("t6"), 1),
        ]
    );
}

#[test]
fn scouts_order_by_start_then_id() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    info.scouts = vec![
        scout("S3", "c", Runtime::Claude, 30),
        scout("S1", "a", Runtime::Claude, 30),
        scout("S9", "z", Runtime::Claude, 5),
    ];

    assert_eq!(
        rows_of(&info, RunFilter::All),
        vec![(root(), 0), (sc("S9"), 1), (sc("S1"), 1), (sc("S3"), 1)]
    );
}

#[test]
fn the_root_is_the_run_with_its_orchestrator() {
    let (snapshot, windows) = three_task_fixture();
    let rows = run_rows(
        &snapshot.runs[0],
        &windows,
        &TreeState::default(),
        RunFilter::All,
    );

    let guides: Vec<&str> = rows.iter().map(|row| row.guides.as_str()).collect();
    assert_eq!(guides, ["", "├─", "│ ├─", "│ └─", "├─", "│ └─", "└─"]);
    match &rows[0].kind {
        RowKind::Run {
            run, orchestrator, ..
        } => {
            assert_eq!(run.run_id, RUN_ID);
            assert_eq!(orchestrator.map(|window| window.id), Some(3));
        }
        other => panic!("the root is a run, not {other:?}"),
    }
}

#[test]
fn tasks_order_by_wave_then_plan_order() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    info.tasks = vec![
        in_wave(pending("t3"), 1, None),
        in_wave(pending("t1"), 0, None),
        in_wave(pending("t2"), 1, None),
    ];

    assert_eq!(
        rows_of(&info, RunFilter::All),
        vec![(root(), 0), (t("t1"), 1), (t("t3"), 1), (t("t2"), 1)]
    );
}

#[test]
fn a_planner_with_no_tasks_sorts_after_the_root_tasks() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    info.planners = vec![
        planner("Z", "cli"),
        planner("A", "daemon"),
        planner("Y", ""),
    ];
    info.tasks = vec![
        in_wave(pending("t0"), 0, None),
        in_wave(pending("t1"), 1, Some("A")),
        in_wave(pending("t5"), 3, None),
    ];

    assert_eq!(
        rows_of(&info, RunFilter::All),
        vec![
            (root(), 0),
            (t("t0"), 1),
            (pl("A"), 1),
            (t("t1"), 2),
            (t("t5"), 1),
            (pl("Z"), 1),
            (pl("Y"), 1),
        ]
    );
}

#[test]
fn a_task_whose_epic_has_no_planner_hangs_from_the_root() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    info.planners = vec![planner("A", "daemon")];
    info.tasks = vec![
        in_wave(pending("t1"), 0, Some("B")),
        in_wave(pending("t2"), 1, Some("A")),
    ];

    assert_eq!(
        rows_of(&info, RunFilter::All),
        vec![(root(), 0), (t("t1"), 1), (pl("A"), 1), (t("t2"), 2)]
    );
}

#[test]
fn worker_rounds_split_where_they_were_sent_back() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mut t1 = task("t1", "spawn", Size::M, TaskState::Working);
    let mut session = worker(1, None, Runtime::Claude, 100);
    session.sent_back_at = vec![300, 500];
    session.ended_at = Some(600);
    let mut review_1 = reviewer(1, None, Runtime::Codex, 200);
    review_1.ended_at = Some(250);
    let review_2 = reviewer(2, None, Runtime::Codex, 400);
    // Listed out of start order: the rows sort by start time.
    t1.rounds = vec![review_2, session, review_1];
    info.tasks = vec![t1];

    let rows = run_rows(&info, &[], &TreeState::default(), RunFilter::All);
    assert_eq!(
        keys_and_depths(&rows),
        vec![
            (root(), 0),
            (t("t1"), 1),
            (rd("t1", AgentRole::Worker, 1, 1), 2),
            (rd("t1", AgentRole::Reviewer, 1, 1), 2),
            (rd("t1", AgentRole::Worker, 1, 2), 2),
            (rd("t1", AgentRole::Reviewer, 2, 2), 2),
            (rd("t1", AgentRole::Worker, 1, 3), 2),
        ]
    );
    assert_eq!(
        round_labels(&rows),
        [
            "worker #1",
            "review #1",
            "worker #1 r2",
            "review #2",
            "worker #1 r3"
        ]
    );

    let rounds = display_rounds(&info.tasks[0], &[]);
    let spans: Vec<_> = rounds
        .iter()
        .map(|round| {
            (
                round.info.role,
                round.number,
                round.started_at,
                round.ended_at,
            )
        })
        .collect();
    assert_eq!(
        spans,
        vec![
            (AgentRole::Worker, 1, 100, Some(300)),
            (AgentRole::Reviewer, 1, 200, Some(250)),
            (AgentRole::Worker, 2, 300, Some(500)),
            (AgentRole::Reviewer, 2, 400, None),
            (AgentRole::Worker, 3, 500, Some(600)),
        ]
    );
    let worker_last: Vec<bool> = rounds
        .iter()
        .filter(|round| round.info.role == AgentRole::Worker)
        .map(|round| round.last)
        .collect();
    assert_eq!(worker_last, [false, false, true]);
    // A review round is its session's only display round.
    assert!(
        rounds
            .iter()
            .filter(|round| round.info.role == AgentRole::Reviewer)
            .all(|round| round.last)
    );
}

#[test]
fn a_fresh_session_is_worker_2() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mut t1 = task("t1", "spawn", Size::M, TaskState::Working);
    let mut first = worker(1, None, Runtime::Claude, 100);
    first.ended_at = Some(200);
    let second = worker(2, None, Runtime::Claude, 300);
    assert_eq!((first.round, second.round), (1, 2));
    t1.rounds = vec![first, second];
    info.tasks = vec![t1];

    let rows = run_rows(&info, &[], &TreeState::default(), RunFilter::All);
    assert_eq!(round_labels(&rows), ["worker #1", "worker #2"]);
    assert_eq!(
        keys(&rows)[2..],
        [
            rd("t1", AgentRole::Worker, 1, 1),
            rd("t1", AgentRole::Worker, 2, 1)
        ]
    );
    assert!(display_rounds(&info.tasks[0], &[]).iter().all(|r| r.last));
}

#[test]
fn a_worker_sorts_before_a_reviewer_on_a_tie() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mut t1 = task("t1", "spawn", Size::M, TaskState::Review);
    t1.rounds = vec![
        reviewer(1, None, Runtime::Codex, 100),
        worker(1, None, Runtime::Claude, 100),
    ];
    info.tasks = vec![t1];

    let rows = run_rows(&info, &[], &TreeState::default(), RunFilter::All);
    assert_eq!(round_labels(&rows), ["worker #1", "review #1"]);
}

#[test]
fn an_unexpected_role_on_a_task_is_labelled_not_dropped() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mut t1 = task("t1", "spawn", Size::M, TaskState::Working);
    let mut odd = worker(1, None, Runtime::Claude, 100);
    odd.role = AgentRole::Scout;
    odd.sent_back_at = vec![150];
    let mut other = worker(2, None, Runtime::Claude, 200);
    other.role = AgentRole::Orchestrator;
    t1.rounds = vec![odd, other];
    info.tasks = vec![t1];

    let rows = run_rows(&info, &[], &TreeState::default(), RunFilter::All);
    // Milestone 9: a scout round on a task is a research session.
    assert_eq!(round_labels(&rows), ["research #1", "orchestrator #2"]);
    assert_eq!(rows.len(), 4);
    assert_eq!(round_label(AgentRole::Reviewer, 7, 3), "review #3");
}

#[test]
fn sub_agents_hang_under_the_last_display_round_only() {
    let (mut snapshot, mut windows) = three_task_fixture();
    let t1 = &mut snapshot.runs[0].tasks[1];
    t1.rounds[0].sent_back_at = vec![9_800];
    windows[2].subagents = vec![
        subagent(
            "a",
            None,
            "Explore",
            "find hooks",
            SubagentState::Running,
            5,
        ),
        subagent("b", None, "Plan", "draft", SubagentState::Running, 9),
    ];

    let rows = run_rows(
        &snapshot.runs[0],
        &windows,
        &TreeState::default(),
        RunFilter::All,
    );
    assert_unique(&rows);
    let found = keys_and_depths(&rows);
    let at = found
        .iter()
        .position(|(key, _)| *key == t("t1"))
        .expect("t1 has a row");
    assert_eq!(
        found[at..at + 5],
        [
            (t("t1"), 1),
            (rd("t1", AgentRole::Worker, 1, 1), 2),
            (rd("t1", AgentRole::Worker, 1, 2), 2),
            (sub(6, "b"), 3),
            (sub(6, "a"), 3),
        ]
    );
    let windowed: Vec<Option<u32>> = rows
        .iter()
        .filter_map(|row| match &row.kind {
            RowKind::AgentRound { task, round, .. } if task.id == "t1" => {
                Some(round.window.map(|window| window.id))
            }
            _ => None,
        })
        .collect();
    assert_eq!(windowed, [Some(6), Some(6)]);
}

#[test]
fn a_round_whose_window_is_not_listed_has_no_sub_agents_and_no_panic() {
    let (mut snapshot, mut windows) = three_task_fixture();
    snapshot.runs[0].tasks[1].rounds[0].window_id = Some(99);
    // Window 6, `t1`'s worker's by its run reference, is listed with a sub-agent; the
    // round names window 99, so the sub-agent is not reattached to it.
    windows[2].subagents = vec![subagent(
        "a",
        None,
        "Explore",
        "grep",
        SubagentState::Running,
        1,
    )];

    let rows = run_rows(
        &snapshot.runs[0],
        &windows,
        &TreeState::default(),
        RunFilter::All,
    );
    let round = rows
        .iter()
        .find_map(|row| match &row.kind {
            RowKind::AgentRound { task, round, .. } if task.id == "t1" => Some(round.clone()),
            _ => None,
        })
        .expect("the round keeps its row");
    assert_eq!(round.window, None);
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row.kind, RowKind::Subagent { .. }))
    );
}

#[test]
fn a_scout_with_a_listed_window_shows_its_sub_agents() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mut s1 = scout("S1", "where?", Runtime::Claude, 10);
    s1.window_id = Some(7);
    info.scouts = vec![s1];
    let mut window = headless(
        7,
        "3f9a/scout",
        PROJECT,
        Some(run_ref(RUN_ID, None, AgentRole::Scout, 1)),
    );
    window.subagents = vec![subagent(
        "x",
        None,
        "Explore",
        "grep",
        SubagentState::Running,
        1,
    )];
    let windows = [window];

    let rows = run_rows(&info, &windows, &TreeState::default(), RunFilter::All);
    assert_eq!(
        keys_and_depths(&rows),
        vec![(root(), 0), (sc("S1"), 1), (sub(7, "x"), 2)]
    );
}

mod filters;
mod patterns;
mod placement;
