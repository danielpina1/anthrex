//! Milestone 8c task 4: the run view's filters, folds, scale and hostile snapshots (decision 21).

use super::*;
/// Scouts `S1` (Claude, working) and `S2` (Codex, reported); planner `A`; `t0` merged
/// (Claude, one ended Claude worker); `t1` working in `A` (Claude: a live Claude worker
/// and an ended Codex reviewer); `t2` blocked in `A` (no rounds); `t3` queued (Codex: an
/// ended Codex worker and a live Claude worker).
fn filter_fixture() -> proto::RunInfo {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mut s2 = scout("S2", "which crate?", Runtime::Codex, 20);
    s2.state = ScoutState::Reported;
    s2.ended_at = Some(30);
    info.scouts = vec![scout("S1", "where?", Runtime::Claude, 10), s2];
    info.planners = vec![planner("A", "daemon")];

    let mut t0 = task("t0", "proto", Size::M, TaskState::Merged);
    let mut t0_worker = worker(1, None, Runtime::Claude, 100);
    t0_worker.ended_at = Some(150);
    t0.rounds = vec![t0_worker];

    let mut t1 = in_wave(
        task("t1", "spawn", Size::M, TaskState::Working),
        1,
        Some("A"),
    );
    let mut t1_review = reviewer(1, None, Runtime::Codex, 250);
    t1_review.ended_at = Some(260);
    t1.rounds = vec![worker(1, None, Runtime::Claude, 200), t1_review];

    let t2 = in_wave(
        task("t2", "status", Size::S, TaskState::Blocked),
        1,
        Some("A"),
    );

    let mut t3 = in_wave(task("t3", "docs", Size::S, TaskState::Queued), 2, None);
    t3.route.runtime = Runtime::Codex;
    let mut t3_first = worker(1, None, Runtime::Codex, 300);
    t3_first.ended_at = Some(350);
    t3.rounds = vec![t3_first, worker(2, None, Runtime::Claude, 400)];

    info.tasks = vec![t0, t1, t2, t3];
    info
}

#[test]
fn filters_keep_ancestors_and_drop_the_rest() {
    let info = filter_fixture();
    let (worker, reviewer) = (AgentRole::Worker, AgentRole::Reviewer);

    assert_eq!(
        rows_of(&info, RunFilter::Running),
        vec![
            (root(), 0),
            (sc("S1"), 1),
            (pl("A"), 1),
            (t("t1"), 2),
            (rd("t1", worker, 1, 1), 3),
            (rd("t1", reviewer, 1, 1), 3),
            (t("t3"), 1),
            (rd("t3", worker, 2, 1), 2),
        ]
    );
    assert_eq!(
        rows_of(&info, RunFilter::Blocked),
        vec![(root(), 0), (pl("A"), 1), (t("t2"), 2)]
    );
    assert_eq!(
        rows_of(&info, RunFilter::Runtime(Runtime::Codex)),
        vec![
            (root(), 0),
            (sc("S2"), 1),
            (pl("A"), 1),
            (t("t1"), 2),
            (rd("t1", reviewer, 1, 1), 3),
            (t("t3"), 1),
            (rd("t3", worker, 1, 1), 2),
            (rd("t3", worker, 2, 1), 2),
        ]
    );
    assert_eq!(
        rows_of(&info, RunFilter::Runtime(Runtime::Claude)),
        vec![
            (root(), 0),
            (sc("S1"), 1),
            (t("t0"), 1),
            (rd("t0", worker, 1, 1), 2),
            (pl("A"), 1),
            (t("t1"), 2),
            (rd("t1", worker, 1, 1), 3),
            (rd("t1", reviewer, 1, 1), 3),
            (t("t2"), 2),
            (t("t3"), 1),
            (rd("t3", worker, 2, 1), 2),
        ]
    );
    assert_eq!(
        rows_of(&info, RunFilter::Runtime(Runtime::Shell)),
        vec![(root(), 0)]
    );
    let mut calm = info.clone();
    calm.tasks.retain(|task| task.state != TaskState::Blocked);
    assert_eq!(rows_of(&calm, RunFilter::Blocked), vec![(root(), 0)]);
}

#[test]
fn only_the_unended_piece_of_a_bounced_session_is_running() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    // A task whose own state is not running, so only its rounds can match.
    let mut t1 = task("t1", "spawn", Size::M, TaskState::Queued);
    let mut session = worker(1, None, Runtime::Claude, 100);
    session.sent_back_at = vec![300];
    t1.rounds = vec![session];
    info.tasks = vec![t1];

    assert_eq!(
        rows_of(&info, RunFilter::Running),
        vec![
            (root(), 0),
            (t("t1"), 1),
            (rd("t1", AgentRole::Worker, 1, 2), 2),
        ]
    );
}

#[test]
fn a_planner_never_matches_a_runtime_by_its_own_route() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mut codex_planner = planner("A", "daemon");
    codex_planner.route.runtime = Runtime::Codex;
    info.planners = vec![codex_planner];
    let mut t1 = in_wave(pending("t1"), 0, Some("A"));
    t1.rounds = vec![worker(1, None, Runtime::Claude, 100)];
    info.tasks = vec![t1];

    assert_eq!(
        rows_of(&info, RunFilter::Runtime(Runtime::Codex)),
        vec![(root(), 0)]
    );
    assert_eq!(
        rows_of(&info, RunFilter::Runtime(Runtime::Claude)),
        vec![
            (root(), 0),
            (pl("A"), 1),
            (t("t1"), 2),
            (rd("t1", AgentRole::Worker, 1, 1), 3),
        ]
    );
}

#[test]
fn the_text_filter_keeps_a_matching_task_with_all_its_rounds() {
    let (snapshot, windows) = three_task_fixture();
    let info = &snapshot.runs[0];
    let filtered = |text: &str, filter: RunFilter| {
        let state = TreeState {
            filter: text.into(),
            ..TreeState::default()
        };
        keys_and_depths(&run_rows(info, &windows, &state, filter))
    };

    assert_eq!(
        filtered("PROTO", RunFilter::All),
        vec![
            (root(), 0),
            (t("t0"), 1),
            (rd("t0", AgentRole::Worker, 1, 1), 2),
            (rd("t0", AgentRole::Reviewer, 1, 1), 2),
        ]
    );
    assert_eq!(filtered("zzz", RunFilter::All), vec![(root(), 0)]);
    // The root's own text matches: everything below it is kept.
    assert_eq!(
        filtered("orchestrator", RunFilter::All).len(),
        run_rows(info, &windows, &TreeState::default(), RunFilter::All).len()
    );
    // Both filters at once: a node must pass both, or lead to one that does.
    assert_eq!(
        filtered("proto", RunFilter::Runtime(Runtime::Codex)),
        vec![
            (root(), 0),
            (t("t0"), 1),
            (rd("t0", AgentRole::Reviewer, 1, 1), 2),
        ]
    );
}

#[test]
fn the_text_filter_reaches_sub_agents_through_their_round() {
    let (snapshot, mut windows) = three_task_fixture();
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
    let state = TreeState {
        filter: "hooks".into(),
        ..TreeState::default()
    };

    assert_eq!(
        keys_and_depths(&run_rows(
            &snapshot.runs[0],
            &windows,
            &state,
            RunFilter::All
        )),
        vec![
            (root(), 0),
            (t("t1"), 1),
            (rd("t1", AgentRole::Worker, 1, 1), 2),
            (sub(6, "a"), 3),
        ]
    );
}

#[test]
fn a_collapsed_task_hides_its_rounds() {
    let (snapshot, windows) = three_task_fixture();
    let info = &snapshot.runs[0];
    let mut state = TreeState::default();
    state.collapsed.insert(t("t0"));

    assert_eq!(
        keys_and_depths(&run_rows(info, &windows, &state, RunFilter::All)),
        vec![
            (root(), 0),
            (t("t0"), 1),
            (t("t1"), 1),
            (rd("t1", AgentRole::Worker, 1, 1), 2),
            (t("t2"), 1),
        ]
    );

    state.collapsed.insert(root());
    assert_eq!(
        keys_and_depths(&run_rows(info, &windows, &state, RunFilter::All)),
        vec![(root(), 0)]
    );

    // A filter shows its matches whatever is folded, as the project tree does.
    state.filter = "proto".into();
    assert_eq!(
        run_rows(info, &windows, &state, RunFilter::All).len(),
        4,
        "t0 and both its rounds under the root"
    );
}

#[test]
fn two_hundred_tasks_build_in_order() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    info.tasks = (0..200u64)
        .map(|index| {
            let id = format!("t{index}");
            let mut info = task(&id, "work", Size::S, TaskState::Review);
            let start = 1_000 + index * 10;
            let mut first = reviewer(1, None, Runtime::Codex, start + 1);
            first.ended_at = Some(start + 2);
            info.rounds = vec![
                reviewer(2, None, Runtime::Codex, start + 3),
                worker(1, None, Runtime::Claude, start),
                first,
            ];
            info
        })
        .collect();

    let rows = run_rows(&info, &[], &TreeState::default(), RunFilter::All);
    assert_eq!(rows.len(), 801);
    assert_unique(&rows);
    assert_eq!(
        rows.last().map(|row| row.key.clone()),
        Some(rd("t199", AgentRole::Reviewer, 2, 2))
    );
    let task_order: Vec<NodeKey> = rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::Task { .. }))
        .map(|row| row.key.clone())
        .collect();
    let expected: Vec<NodeKey> = (0..200).map(|index| t(&format!("t{index}"))).collect();
    assert_eq!(task_order, expected);
    assert_eq!(
        keys(&rows)[1..5],
        [
            t("t0"),
            rd("t0", AgentRole::Worker, 1, 1),
            rd("t0", AgentRole::Reviewer, 1, 1),
            rd("t0", AgentRole::Reviewer, 2, 2),
        ]
    );
}

#[test]
fn hostile_snapshots_never_panic_and_keep_keys_unique() {
    let (mut snapshot, mut windows) = three_task_fixture();
    windows[2].subagents = vec![subagent(
        "a",
        None,
        "Explore",
        "x",
        SubagentState::Running,
        1,
    )];
    let info = &mut snapshot.runs[0];
    info.planners = vec![planner("Z", "nothing"), planner("Z", "twice")];
    info.scouts = vec![
        scout("S1", "q", Runtime::Claude, 5),
        scout("S1", "again", Runtime::Claude, 1),
    ];
    let t1 = &mut info.tasks[1];
    t1.deps = vec!["nope".into(), "t1".into()];
    t1.implicit_deps = vec!["ghost".into()];
    // Out of order, before the start, and repeated.
    t1.rounds[0].sent_back_at = vec![9_900, 9_800, 9_800, 1];
    // A reviewer that started before any worker, on the worker's own window.
    t1.rounds.push(reviewer(1, Some(6), Runtime::Codex, 50));
    // A second copy of the same session and of the same review round.
    t1.rounds.push(worker(1, Some(6), Runtime::Claude, 9_950));
    t1.rounds.push(reviewer(1, Some(6), Runtime::Codex, 60));
    let mut stray = task("t9", "stray", Size::S, TaskState::Working);
    stray.epic = Some("nobody".into());
    stray.deps = vec!["t404".into()];
    info.tasks.push(stray.clone());
    info.tasks.push(stray);

    let every_filter = [
        RunFilter::All,
        RunFilter::Running,
        RunFilter::Blocked,
        RunFilter::Runtime(Runtime::Claude),
        RunFilter::Runtime(Runtime::Codex),
        RunFilter::Runtime(Runtime::Shell),
    ];
    for text in ["", "x", "worker", "t1", "zzz"] {
        for filter in every_filter {
            let state = TreeState {
                filter: text.into(),
                ..TreeState::default()
            };
            let rows = run_rows(info, &windows, &state, filter);
            assert_unique(&rows);
            assert_eq!(rows[0].key, root());
        }
    }

    let rounds = display_rounds(&info.tasks[1], &windows);
    let starts: Vec<u64> = rounds.iter().map(|round| round.started_at).collect();
    let mut sorted = starts.clone();
    sorted.sort_unstable();
    assert_eq!(starts, sorted, "rounds come in start order");
    let workers: Vec<(u32, u64, Option<u64>, bool)> = rounds
        .iter()
        .filter(|round| round.info.role == AgentRole::Worker)
        .map(|round| (round.number, round.started_at, round.ended_at, round.last))
        .collect();
    // Bounces sorted and never earlier than the session's start (9 700).
    assert_eq!(
        workers,
        [
            (1, 9_700, Some(9_700)),
            (2, 9_700, Some(9_800)),
            (3, 9_800, Some(9_800)),
            (4, 9_800, Some(9_900)),
            (5, 9_900, None),
        ]
        .map(|(number, start, end)| (number, start, end, number == 5))
    );
    let rows = run_rows(info, &windows, &TreeState::default(), RunFilter::All);
    let subagents = rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::Subagent { .. }))
        .count();
    assert_eq!(subagents, 1, "one window's sub-agents are drawn once");
}
