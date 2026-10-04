//! Milestone 8c task 3: runs in the project tree (decisions 6–10).

use super::super::*;
use super::run_fixtures::{
    PROJECT, RUN_ID, gate_fixture, headless, pty, run, run_ref, task, three_task_fixture,
};
use proto::{AgentRole, RunState, Size, Status, TaskState};
use std::path::PathBuf;

fn keys_and_depths(rows: &[Row<'_>]) -> Vec<(NodeKey, u16)> {
    rows.iter()
        .map(|row| (row.key.clone(), row.depth))
        .collect()
}

fn project(root: &str) -> NodeKey {
    NodeKey::Project(PathBuf::from(root))
}

fn run_key(id: &str) -> NodeKey {
    NodeKey::Run(id.into())
}

fn run_position(rows: &[Row<'_>], id: &str) -> Option<usize> {
    rows.iter()
        .find_map(|row| match &row.kind {
            RowKind::Run { run, position, .. } if run.run_id == id => Some(*position),
            _ => None,
        })
        .expect("the run has a row")
}

fn window_position(rows: &[Row<'_>], id: u32) -> usize {
    rows.iter()
        .find_map(|row| match &row.kind {
            RowKind::Window { info, position, .. } if info.id == id => Some(*position),
            _ => None,
        })
        .expect("the window has a row")
}

fn project_status(rows: &[Row<'_>], root: &str) -> (Status, RuntimeCounts) {
    rows.iter()
        .find_map(|row| match &row.kind {
            RowKind::Project {
                root: r,
                status,
                counts,
                ..
            } if *r == std::path::Path::new(root) => Some((*status, *counts)),
            _ => None,
        })
        .expect("the project has a row")
}

#[test]
fn build_still_means_no_runs() {
    let (_, windows) = three_task_fixture();
    let rows = build(&windows, &TreeState::default());
    assert_eq!(
        keys_and_depths(&rows),
        vec![
            (project(PROJECT), 0),
            (NodeKey::Window(1), 1),
            (NodeKey::Window(3), 1),
            (NodeKey::Window(6), 1),
        ]
    );
}

#[test]
fn a_run_is_one_node_above_the_plain_windows() {
    let (snap, mut windows) = gate_fixture();
    windows.push(pty(2, "api", PROJECT, Status::Idle));
    let rows = build_with_runs(&windows, &snap.runs, &TreeState::default());
    assert_eq!(
        keys_and_depths(&rows),
        vec![
            (project(PROJECT), 0),
            (run_key(RUN_ID), 1),
            (NodeKey::Window(1), 1),
            (NodeKey::Window(2), 1),
        ]
    );
    assert_eq!(rows[1].guides, "├─", "the run has later siblings");
    assert_eq!(rows[3].guides, "└─");
}

#[test]
fn a_runs_windows_are_not_listed_as_plain_windows() {
    let (snap, windows) = three_task_fixture();
    let rows = build_with_runs(&windows, &snap.runs, &TreeState::default());
    let keys: Vec<_> = rows.iter().map(|row| row.key.clone()).collect();
    assert!(!keys.contains(&NodeKey::Window(3)), "{keys:?}");
    assert!(!keys.contains(&NodeKey::Window(6)), "{keys:?}");
    assert!(keys.contains(&NodeKey::Window(1)), "{keys:?}");
    assert_eq!(
        keys,
        vec![project(PROJECT), run_key(RUN_ID), NodeKey::Window(1)]
    );
}

#[test]
fn a_project_with_only_a_run_still_has_a_row() {
    let (snap, _) = gate_fixture();
    let rows = build_with_runs(&[], &snap.runs, &TreeState::default());
    assert_eq!(
        keys_and_depths(&rows),
        vec![(project(PROJECT), 0), (run_key(RUN_ID), 1)]
    );
    let RowKind::Project { name, counts, .. } = &rows[0].kind else {
        panic!("a project row first");
    };
    assert_eq!(name, "demo");
    assert_eq!(*counts, RuntimeCounts::default());
}

#[test]
fn terminal_runs_are_not_shown() {
    let runs = vec![
        run("accepted-0001", PROJECT, RunState::Accepted),
        run("discarded-0002", PROJECT, RunState::Discarded),
        run("failed-0003", PROJECT, RunState::Failed),
    ];
    assert!(build_with_runs(&[], &runs, &TreeState::default()).is_empty());
    assert_eq!(shown_runs(&runs).count(), 0);

    for state in [
        RunState::Complete,
        RunState::AwaitingApproval,
        RunState::Running,
        RunState::Paused,
        RunState::Halted,
    ] {
        let mut all = runs.clone();
        all.push(run("shown-0004", PROJECT, state));
        let rows = build_with_runs(&[], &all, &TreeState::default());
        assert_eq!(
            keys_and_depths(&rows),
            vec![(project(PROJECT), 0), (run_key("shown-0004"), 1)],
            "{state:?}"
        );
    }
}

#[test]
fn windows_of_an_unknown_run_are_plain() {
    let (mut snap, _) = gate_fixture();
    snap.runs
        .push(run("dropped-0000", PROJECT, RunState::Discarded));
    let windows = vec![
        headless(
            7,
            "0000/t1.r1",
            PROJECT,
            Some(run_ref("gone-0000", Some("t1"), AgentRole::Reviewer, 1)),
        ),
        headless(
            5,
            "0000/t2.w1",
            PROJECT,
            Some(run_ref("dropped-0000", Some("t2"), AgentRole::Worker, 1)),
        ),
        headless(4, "onboarding scout", PROJECT, None),
    ];
    let rows = build_with_runs(&windows, &snap.runs, &TreeState::default());
    assert_eq!(
        keys_and_depths(&rows),
        vec![
            (project(PROJECT), 0),
            (run_key(RUN_ID), 1),
            (NodeKey::Window(4), 1),
            (NodeKey::Window(5), 1),
            (NodeKey::Window(7), 1),
        ]
    );

    // No snapshot received yet: every window is plain.
    let rows = build_with_runs(&windows, &[], &TreeState::default());
    assert_eq!(
        keys_and_depths(&rows),
        vec![
            (project(PROJECT), 0),
            (NodeKey::Window(4), 1),
            (NodeKey::Window(5), 1),
            (NodeKey::Window(7), 1),
        ]
    );
}

#[test]
fn runs_are_ordered_oldest_first() {
    let mut newest = run("run-cccc", PROJECT, RunState::Running);
    newest.created_at = 300;
    let mut tie_b = run("run-bbbb", PROJECT, RunState::Running);
    tie_b.created_at = 100;
    let mut tie_a = run("run-aaaa", PROJECT, RunState::Running);
    tie_a.created_at = 100;
    let mut oldest = run("run-zzzz", PROJECT, RunState::Running);
    oldest.created_at = 50;
    // Newest first, as the daemon sends them (`snapshot.rs:23-28`).
    let runs = vec![newest, tie_b, tie_a, oldest];
    let rows = build_with_runs(&[], &runs, &TreeState::default());
    let run_keys: Vec<_> = rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::Run { .. }))
        .map(|row| row.key.clone())
        .collect();
    assert_eq!(
        run_keys,
        vec![
            run_key("run-zzzz"),
            run_key("run-aaaa"),
            run_key("run-bbbb"),
            run_key("run-cccc"),
        ]
    );
    let shown: Vec<_> = shown_runs(&runs).map(|r| r.run_id.as_str()).collect();
    assert_eq!(shown, ["run-zzzz", "run-aaaa", "run-bbbb", "run-cccc"]);
}

#[test]
fn project_status_includes_its_runs() {
    let windows = vec![pty(1, "shell", PROJECT, Status::Idle)];
    let mut running = run(RUN_ID, PROJECT, RunState::Running);
    running.tasks = vec![
        task("t1", "a", Size::S, TaskState::Working),
        task("t2", "b", Size::S, TaskState::Blocked),
    ];
    let rows = build_with_runs(
        &windows,
        std::slice::from_ref(&running),
        &TreeState::default(),
    );
    let (status, counts) = project_status(&rows, PROJECT);
    assert_eq!(status, Status::Attention, "a blocked task needs attention");
    assert_eq!(
        counts,
        RuntimeCounts {
            claude: 0,
            codex: 0,
            shell: 1
        }
    );

    running.tasks[1].state = TaskState::Queued;
    let rows = build_with_runs(
        &windows,
        std::slice::from_ref(&running),
        &TreeState::default(),
    );
    assert_eq!(project_status(&rows, PROJECT).0, Status::Working);

    for (state, expected) in [
        (RunState::AwaitingApproval, Status::Attention),
        (RunState::Paused, Status::Attention),
        (RunState::Halted, Status::Attention),
        (RunState::Complete, Status::Done),
    ] {
        running.state = state;
        let rows = build_with_runs(
            &windows,
            std::slice::from_ref(&running),
            &TreeState::default(),
        );
        assert_eq!(project_status(&rows, PROJECT).0, expected, "{state:?}");
        assert_eq!(run_status(&running), expected, "{state:?}");
    }

    // Hidden windows count neither toward the status nor the runtime counts.
    let (snap, windows) = three_task_fixture();
    let mut windows = windows;
    windows[2].status = Status::Attention;
    let rows = build_with_runs(&windows, &snap.runs, &TreeState::default());
    assert_eq!(
        project_status(&rows, PROJECT),
        (
            Status::Working,
            RuntimeCounts {
                claude: 0,
                codex: 0,
                shell: 1
            }
        )
    );
}

#[test]
fn the_orchestrator_takes_a_position_and_the_number_keys_reach_it() {
    let (snap, windows) = three_task_fixture();
    let rows = build_with_runs(&windows, &snap.runs, &TreeState::default());
    assert_eq!(run_position(&rows, RUN_ID), Some(1));
    assert_eq!(window_position(&rows, 1), 2);
    assert_eq!(agent_order(&rows), vec![3, 1]);
    let RowKind::Run { orchestrator, .. } = &rows[1].kind else {
        panic!("the run row");
    };
    assert_eq!(orchestrator.map(|w| w.id), Some(3));
}

#[test]
fn a_run_without_an_orchestrator_has_no_position() {
    let (snap, windows) = gate_fixture();
    let rows = build_with_runs(&windows, &snap.runs, &TreeState::default());
    assert_eq!(run_position(&rows, RUN_ID), None);
    assert_eq!(window_position(&rows, 1), 1);
    assert_eq!(agent_order(&rows), vec![1]);
}

#[test]
fn an_orchestrator_window_of_another_run_is_plain_and_positions_nothing() {
    // The shown run's orchestrator is not in the window list; the only orchestrator
    // window listed belongs to a run the snapshot does not name.
    let (snap, mut windows) = gate_fixture();
    let mut stray = pty(9, "orchestrator", PROJECT, Status::Idle);
    stray.run = Some(run_ref("other-1111", None, AgentRole::Orchestrator, 1));
    windows.push(stray);
    let rows = build_with_runs(&windows, &snap.runs, &TreeState::default());
    assert_eq!(run_position(&rows, RUN_ID), None);
    assert_eq!(agent_order(&rows), vec![1, 9]);
    assert_eq!(window_position(&rows, 9), 2);
}

#[test]
fn two_runs_in_one_project() {
    let (_, windows) = three_task_fixture();
    let (three, _) = three_task_fixture();
    let mut second = run("second-9999", PROJECT, RunState::AwaitingApproval);
    second.created_at = 1;
    let mut runs = three.runs.clone();
    runs.insert(0, second);
    let rows = build_with_runs(&windows, &runs, &TreeState::default());
    assert_eq!(
        keys_and_depths(&rows),
        vec![
            (project(PROJECT), 0),
            (run_key("second-9999"), 1),
            (run_key(RUN_ID), 1),
            (NodeKey::Window(1), 1),
        ]
    );
    assert_eq!(run_position(&rows, "second-9999"), None);
    assert_eq!(run_position(&rows, RUN_ID), Some(1));
    assert_eq!(agent_order(&rows), vec![3, 1]);
}

#[test]
fn a_run_sits_under_its_exact_project_not_a_prefix() {
    let windows = vec![
        pty(1, "shell", "/r/demo", Status::Idle),
        pty(2, "shell", "/r/demo2", Status::Idle),
    ];
    let runs = vec![run("in-demo2-0002", "/r/demo2", RunState::Running)];
    let rows = build_with_runs(&windows, &runs, &TreeState::default());
    assert_eq!(
        keys_and_depths(&rows),
        vec![
            (project("/r/demo2"), 0),
            (run_key("in-demo2-0002"), 1),
            (NodeKey::Window(2), 1),
            (project("/r/demo"), 0),
            (NodeKey::Window(1), 1),
        ]
    );
}

#[test]
fn the_filter_matches_a_runs_goal_and_id() {
    let (snap, mut windows) = gate_fixture();
    windows.push(pty(2, "api", PROJECT, Status::Idle));
    for filter in ["reset", "3f9a", "RESET", "password"] {
        let state = TreeState {
            filter: filter.into(),
            ..TreeState::default()
        };
        let rows = build_with_runs(&windows, &snap.runs, &state);
        assert_eq!(
            keys_and_depths(&rows),
            vec![(project(PROJECT), 0), (run_key(RUN_ID), 1)],
            "{filter}"
        );
        assert_eq!(rows[1].guides, "└─", "{filter}");
    }
    // Review M4: a project matched by its own name keeps its run as well as its
    // windows, though the run's goal and id do not contain the filter.
    let state = TreeState {
        filter: "demo".into(),
        ..TreeState::default()
    };
    let rows = build_with_runs(&windows, &snap.runs, &state);
    assert_eq!(
        keys_and_depths(&rows),
        vec![
            (project(PROJECT), 0),
            (run_key(RUN_ID), 1),
            (NodeKey::Window(1), 1),
            (NodeKey::Window(2), 1),
        ]
    );
    // A filter naming the window drops the run.
    let state = TreeState {
        filter: "api".into(),
        ..TreeState::default()
    };
    let rows = build_with_runs(&windows, &snap.runs, &state);
    assert_eq!(
        keys_and_depths(&rows),
        vec![(project(PROJECT), 0), (NodeKey::Window(2), 1)]
    );
    // A hidden window's name matches nothing: the orchestrator is not a plain row.
    let (snap, windows) = three_task_fixture();
    let state = TreeState {
        filter: "orchestrator".into(),
        ..TreeState::default()
    };
    assert!(build_with_runs(&windows, &snap.runs, &state).is_empty());
}

#[test]
fn a_non_ascii_goal_matches_the_filter_and_an_empty_goal_matches_its_id() {
    let mut accented = run("reinit-aaaa", PROJECT, RunState::Running);
    accented.goal = "Réinitialiser le MOT DE PASSE 🔑".into();
    let mut empty = run("blank-bbbb", PROJECT, RunState::Running);
    empty.goal = String::new();
    let runs = vec![accented, empty];
    for (filter, expected) in [
        ("RÉINIT", "reinit-aaaa"),
        ("mot de passe 🔑", "reinit-aaaa"),
        ("blank", "blank-bbbb"),
    ] {
        let state = TreeState {
            filter: filter.into(),
            ..TreeState::default()
        };
        let rows = build_with_runs(&[], &runs, &state);
        assert_eq!(
            keys_and_depths(&rows),
            vec![(project(PROJECT), 0), (run_key(expected), 1)],
            "{filter}"
        );
    }
}

#[test]
fn a_collapsed_project_hides_its_runs() {
    let (snap, windows) = three_task_fixture();
    let mut state = TreeState::default();
    assert!(state.toggle(&project(PROJECT)));
    let rows = build_with_runs(&windows, &snap.runs, &state);
    assert_eq!(keys_and_depths(&rows), vec![(project(PROJECT), 0)]);
    assert!(agent_order(&rows).is_empty());
}

#[test]
fn prune_runs_drops_keys_of_runs_that_left() {
    let mut shown = run("a", "/r/solo", RunState::Running);
    shown.tasks = vec![task("t1", "x", Size::S, TaskState::Working)];
    let task_key = NodeKey::Task {
        run: "a".into(),
        id: "t1".into(),
    };
    let round_key = NodeKey::AgentRound {
        run: "a".into(),
        task: "t1".into(),
        role: AgentRole::Worker,
        lane: None,
        session: 1,
        round: 1,
    };
    let other_task = NodeKey::Task {
        run: "b".into(),
        id: "t1".into(),
    };
    let solo = project("/r/solo");
    let windows = vec![pty(1, "shell", PROJECT, Status::Idle)];

    let mut state = TreeState::default();
    for key in [&task_key, &round_key, &run_key("a"), &other_task, &solo] {
        assert!(state.toggle(key), "{key:?} folds");
    }

    let runs = vec![shown.clone()];
    state.prune_runs(&runs);
    state.prune(&windows);
    for key in [&task_key, &round_key, &run_key("a"), &solo] {
        assert!(state.is_collapsed(key), "{key:?} kept while a is shown");
    }
    assert!(!state.is_collapsed(&other_task), "run b is not shown");

    shown.state = RunState::Discarded;
    let runs = vec![shown];
    state.prune(&windows);
    assert!(state.is_collapsed(&task_key), "prune leaves run keys alone");
    state.prune_runs(&runs);
    state.prune(&windows);
    for key in [&task_key, &round_key, &run_key("a"), &solo] {
        assert!(!state.is_collapsed(key), "{key:?} gone with its run");
    }
}

#[test]
fn toggle_folds_every_new_key() {
    let mut state = TreeState::default();
    let keys = [
        run_key("a"),
        NodeKey::Planner {
            run: "a".into(),
            epic: "A".into(),
        },
        NodeKey::Scout {
            run: "a".into(),
            id: "s1".into(),
        },
    ];
    for key in &keys {
        assert!(state.toggle(key));
        assert!(state.is_collapsed(key));
    }
    // `prune` keeps them: run keys are `prune_runs`'s.
    state.prune(&[]);
    for key in &keys {
        assert!(state.is_collapsed(key));
    }
}

/// Review M2: cancelled tasks count in neither the merged nor the total figure.
#[test]
fn cancelled_tasks_are_left_out_of_a_runs_progress() {
    let mut three = run(RUN_ID, PROJECT, RunState::Running);
    three.tasks = vec![
        task("t0", "a", Size::S, TaskState::Merged),
        task("t1", "b", Size::S, TaskState::Cancelled),
        task("t2", "c", Size::S, TaskState::Working),
        task("t3", "d", Size::S, TaskState::Cancelled),
    ];
    assert_eq!(run_progress(&three), (1, 2));
}

/// Review M2: a whitespace-only goal is blank, so the run is known by its id.
#[test]
fn a_blank_goal_falls_back_to_the_run_id() {
    let mut blank = run(RUN_ID, PROJECT, RunState::Running);
    blank.goal = " \t\n ".into();
    assert_eq!(run_title(&blank), RUN_ID);
    blank.goal = "  Add reset ".into();
    assert_eq!(run_title(&blank), "  Add reset ");
}

/// Review M1: a run listed twice in one snapshot is shown once, the first copy by the
/// shown order, so no two rows share a key and positions are not handed out twice.
#[test]
fn a_run_listed_twice_is_shown_once() {
    let (snap, windows) = three_task_fixture();
    let mut later = snap.runs[0].clone();
    later.created_at += 100;
    later.goal = "the later copy".into();
    let runs = vec![later, snap.runs[0].clone()];
    let shown: Vec<_> = shown_runs(&runs).collect();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].goal, super::run_fixtures::GOAL);
    let rows = build_with_runs(&windows, &runs, &TreeState::default());
    assert_eq!(
        keys_and_depths(&rows),
        vec![
            (project(PROJECT), 0),
            (run_key(RUN_ID), 1),
            (NodeKey::Window(1), 1),
        ]
    );
    assert_eq!(agent_order(&rows), vec![3, 1]);
}
