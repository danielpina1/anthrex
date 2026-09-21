//! The projection is asserted by exact field lists (decision 13): each test
//! builds a small app, inspects one row, and compares every label and value it
//! produced, in order. A field that quietly appeared or went missing is a diff,
//! not a property several wrong projections could satisfy.

use super::*;
use crate::keymap::Keymap;
use crate::tree::NodeKey;
use proto::{
    GitOperation, GitState, Head, Runtime, Status, SubagentInfo, SubagentState, WindowInfo,
};
use std::path::PathBuf;

#[path = "tests/render.rs"]
mod render_tests;

fn window(id: u32, project: &str, name: &str, runtime: Runtime) -> WindowInfo {
    WindowInfo {
        id,
        name: name.to_owned(),
        runtime,
        cwd: project.into(),
        project: project.into(),
        worktree: None,
        branch: None,
        status: Status::Idle,
        tool: None,
        since_secs: 0,
        last_output_secs: 0,
        session_id: None,
        model: None,
        subagents: vec![],
        exit: None,
    }
}

fn subagent(id: &str, kind: &str, label: Option<&str>) -> SubagentInfo {
    SubagentInfo {
        id: id.to_owned(),
        parent_id: None,
        kind: kind.to_owned(),
        label: label.map(str::to_owned),
        model: None,
        state: SubagentState::Running,
        tool: None,
        started_secs: 0,
        ended_secs: None,
        needs_permission: false,
    }
}

fn app(windows: Vec<WindowInfo>) -> App {
    App::new(windows, "/tmp".into(), Keymap::default_prefix())
}

fn clean_git() -> GitState {
    GitState {
        head: Head::Branch("main".into()),
        upstream: None,
        ahead: 0,
        behind: 0,
        dirty: 0,
        untracked: 0,
        conflicts: 0,
        operation: None,
        stale: false,
    }
}

/// Every label and value the projection produced, in order.
fn pairs(inspection: &Inspection) -> Vec<(&str, &str)> {
    inspection
        .fields
        .iter()
        .map(|field| (field.label, field.value.as_str()))
        .collect()
}

/// Inspects the row `key` names, which is how every test below reaches one.
fn inspect_key(app: &App, key: &NodeKey) -> Inspection {
    let rows = app.rows();
    let row = rows
        .iter()
        .find(|row| &row.key == key)
        .expect("the key names a visible row");
    inspect(row, app)
}

fn home() -> PathBuf {
    dirs::home_dir().expect("a home directory to shorten against")
}

#[test]
fn a_project_lists_path_status_and_agents() {
    let root = home().join("code/shop");
    let root = root.to_str().expect("a utf-8 home directory");
    let app = app(vec![
        window(1, root, "api", Runtime::Claude),
        window(2, root, "sh", Runtime::Shell),
    ]);

    let inspection = inspect_key(&app, &NodeKey::Project(root.into()));

    assert_eq!(inspection.name, "shop");
    assert_eq!(
        pairs(&inspection),
        vec![
            ("path", "~/code/shop"),
            ("status", "idle"),
            ("agents", "cl 1 · sh 1"),
        ]
    );
}

#[test]
fn a_project_on_one_worktree_shows_its_branch_and_changes() {
    let mut first = window(1, "/r/shop", "api", Runtime::Claude);
    first.worktree = Some("/r/shop".into());
    let mut second = window(2, "/r/shop", "web", Runtime::Claude);
    second.worktree = Some("/r/shop".into());
    let mut app = app(vec![first, second]);
    let mut state = clean_git();
    state.dirty = 3;
    state.untracked = 1;
    app.git.insert("/r/shop".into(), state);

    let inspection = inspect_key(&app, &NodeKey::Project("/r/shop".into()));

    assert_eq!(
        pairs(&inspection),
        vec![
            ("path", "/r/shop"),
            ("status", "idle"),
            ("agents", "cl 2"),
            ("branch", "main"),
            ("changes", "●3 ?1"),
        ]
    );
}

#[test]
fn a_project_across_worktrees_shows_a_count() {
    let mut first = window(1, "/r/shop", "api", Runtime::Claude);
    first.worktree = Some("/r/shop".into());
    let mut second = window(2, "/r/shop", "web", Runtime::Claude);
    second.worktree = Some("/r/shop-wt".into());
    let mut app = app(vec![first, second]);
    app.git.insert("/r/shop".into(), clean_git());
    app.git.insert("/r/shop-wt".into(), clean_git());

    let inspection = inspect_key(&app, &NodeKey::Project("/r/shop".into()));

    assert_eq!(
        pairs(&inspection),
        vec![
            ("path", "/r/shop"),
            ("status", "idle"),
            ("agents", "cl 2"),
            ("branch", "2 worktrees"),
        ],
        "with more than one worktree the branch degrades to a count and changes is omitted"
    );
}

#[test]
fn a_window_lists_its_runtime_model_and_timings() {
    let mut info = window(1, "/r/shop", "api-worker", Runtime::Claude);
    info.model = Some("claude-opus-5".into());
    info.status = Status::Working;
    info.tool = Some("Edit".into());
    info.since_secs = 120;
    info.session_id = Some("0f3c".into());
    let app = app(vec![info]);

    let inspection = inspect_key(&app, &NodeKey::Window(1));

    assert_eq!(inspection.name, "1 api-worker");
    assert_eq!(
        pairs(&inspection),
        vec![
            ("runtime", "claude"),
            ("model", "claude-opus-5"),
            ("status", "working · Edit"),
            ("for", "2m"),
            ("dir", "/r/shop"),
            ("session", "0f3c"),
        ]
    );
}

#[test]
fn a_window_without_a_model_shows_a_dash() {
    let mut info = window(1, "/r/shop", "sh", Runtime::Shell);
    // An hour reads as `1h` for the next hour, so the seconds this test itself
    // takes cannot move the string it asserts.
    info.since_secs = 3600;
    let app = app(vec![info]);

    let inspection = inspect_key(&app, &NodeKey::Window(1));

    assert_eq!(
        pairs(&inspection),
        vec![
            ("runtime", "shell"),
            ("model", "-"),
            ("status", "idle"),
            ("for", "1h"),
            ("dir", "/r/shop"),
        ]
    );
}

#[test]
fn a_window_omits_its_worktree_when_it_equals_its_dir() {
    let mut same = window(1, "/r/shop", "api", Runtime::Claude);
    same.worktree = Some("/r/shop".into());
    let matching = app(vec![same]);
    assert!(
        !pairs(&inspect_key(&matching, &NodeKey::Window(1)))
            .iter()
            .any(|(label, _)| *label == "worktree"),
        "a worktree equal to the dir says nothing the dir has not said"
    );

    let mut different = window(1, "/r/shop", "api", Runtime::Claude);
    different.worktree = Some("/r/shop-wt".into());
    let diverging = app(vec![different]);
    assert_eq!(
        pairs(&inspect_key(&diverging, &NodeKey::Window(1)))
            .into_iter()
            .find(|(label, _)| *label == "worktree"),
        Some(("worktree", "/r/shop-wt")),
        "a worktree away from the dir is the thing worth saying"
    );
}

#[test]
fn a_window_without_git_state_omits_the_branch() {
    let mut info = window(1, "/r/shop", "api", Runtime::Claude);
    info.worktree = Some("/r/shop".into());
    let mut app = app(vec![info]);

    assert!(
        !pairs(&inspect_key(&app, &NodeKey::Window(1)))
            .iter()
            .any(|(label, _)| *label == "branch"),
        "absent git state omits the field rather than showing it empty"
    );

    let mut state = clean_git();
    state.dirty = 2;
    state.ahead = 1;
    state.behind = 3;
    state.operation = Some(GitOperation::Rebase);
    app.git.insert("/r/shop".into(), state);
    assert_eq!(
        pairs(&inspect_key(&app, &NodeKey::Window(1)))
            .into_iter()
            .find(|(label, _)| *label == "branch"),
        Some(("branch", "main ●2 ⇡1⇣3 rebase")),
        "present git state carries the dirty and ahead/behind counts with it"
    );
}

#[test]
fn a_window_counts_its_subagents_and_how_many_run() {
    let mut info = window(1, "/r/shop", "api", Runtime::Claude);
    let mut done = subagent("a3", "tests", Some("run the unit suite"));
    done.state = SubagentState::Done;
    done.ended_secs = Some(0);
    info.subagents = vec![
        subagent("a1", "Explore", Some("map routes")),
        subagent("a2", "general-purpose", Some("grep handlers")),
        done,
    ];
    let app = app(vec![info]);

    assert_eq!(
        pairs(&inspect_key(&app, &NodeKey::Window(1)))
            .into_iter()
            .find(|(label, _)| *label == "sub-agents"),
        Some(("sub-agents", "3, 2 running"))
    );
}

#[test]
fn a_subagent_lists_its_kind_task_and_state() {
    let mut info = window(1, "/r/shop", "api-worker", Runtime::Claude);
    let mut explore = subagent("a1", "Explore", Some("map the routes"));
    explore.model = Some("opus".into());
    explore.tool = Some("Read".into());
    explore.started_secs = 90;
    info.subagents = vec![explore];
    let app = app(vec![info]);

    let inspection = inspect_key(
        &app,
        &NodeKey::Subagent {
            window_id: 1,
            id: "a1".into(),
        },
    );

    assert_eq!(inspection.name, "Explore: map the routes");
    assert_eq!(
        pairs(&inspection),
        vec![
            ("kind", "Explore"),
            ("task", "map the routes"),
            ("model", "opus"),
            ("state", "running · Read"),
            ("for", "1m"),
            ("spawned by", "1 api-worker"),
            ("depth", "1"),
        ]
    );
}

#[test]
fn a_subagent_without_a_label_omits_the_task() {
    let mut info = window(1, "/r/shop", "api-worker", Runtime::Claude);
    info.subagents = vec![subagent("a1", "Explore", None)];
    let app = app(vec![info]);

    let inspection = inspect_key(
        &app,
        &NodeKey::Subagent {
            window_id: 1,
            id: "a1".into(),
        },
    );

    assert!(
        !pairs(&inspection).iter().any(|(label, _)| *label == "task"),
        "there is no task to show, and an empty box would say less than none"
    );
}

#[test]
fn a_subagent_task_is_marked_for_wrapping() {
    let mut info = window(1, "/r/shop", "api-worker", Runtime::Claude);
    info.subagents = vec![subagent("a1", "Explore", Some("map the routes"))];
    let app = app(vec![info]);

    let inspection = inspect_key(
        &app,
        &NodeKey::Subagent {
            window_id: 1,
            id: "a1".into(),
        },
    );

    let wrapping: Vec<&str> = inspection
        .fields
        .iter()
        .filter(|field| field.wrap)
        .map(|field| field.label)
        .collect();
    assert_eq!(
        wrapping,
        vec!["task"],
        "the task is the one field the panel may not elide, and the only one"
    );
}

#[test]
fn spawned_by_names_the_parent_subagent() {
    let mut info = window(1, "/r/shop", "api-worker", Runtime::Claude);
    let mut child = subagent("a2", "general-purpose", Some("grep handlers"));
    child.parent_id = Some("a1".into());
    info.subagents = vec![subagent("a1", "Explore", Some("map the routes")), child];
    let app = app(vec![info]);

    let inspection = inspect_key(
        &app,
        &NodeKey::Subagent {
            window_id: 1,
            id: "a2".into(),
        },
    );

    assert_eq!(
        pairs(&inspection)
            .into_iter()
            .find(|(label, _)| *label == "spawned by"),
        Some(("spawned by", "Explore: map the routes")),
        "a nested sub-agent names the sub-agent above it, which the tree cannot say"
    );
    assert_eq!(
        pairs(&inspection)
            .into_iter()
            .find(|(label, _)| *label == "depth"),
        Some(("depth", "2")),
        "and sits one level deeper than its parent"
    );
}

#[test]
fn spawned_by_falls_back_to_the_window() {
    // Two projects, so the window's number is not simply its id: "spawned by"
    // must show the number the graph and the sidebar show for that window.
    let other = window(1, "/r/aaa", "other", Runtime::Shell);
    let mut info = window(2, "/r/shop", "api-worker", Runtime::Claude);
    let mut orphan = subagent("a2", "general-purpose", Some("grep handlers"));
    orphan.parent_id = Some("gone".into());
    info.subagents = vec![subagent("a1", "Explore", Some("map the routes")), orphan];
    let app = app(vec![other, info]);

    let rooted = inspect_key(
        &app,
        &NodeKey::Subagent {
            window_id: 2,
            id: "a1".into(),
        },
    );
    assert_eq!(
        pairs(&rooted)
            .into_iter()
            .find(|(label, _)| *label == "spawned by"),
        Some(("spawned by", "2 api-worker")),
        "no parent_id means the window spawned it"
    );

    let orphaned = inspect_key(
        &app,
        &NodeKey::Subagent {
            window_id: 2,
            id: "a2".into(),
        },
    );
    assert_eq!(
        pairs(&orphaned)
            .into_iter()
            .find(|(label, _)| *label == "spawned by"),
        Some(("spawned by", "2 api-worker")),
        "a parent_id naming a sub-agent that has gone falls back to the window too"
    );
}

#[test]
fn a_finished_subagent_shows_how_long_it_ran() {
    let mut info = window(1, "/r/shop", "api-worker", Runtime::Claude);
    let mut done = subagent("a1", "tests", Some("run the unit suite"));
    done.state = SubagentState::Done;
    // Both are ages: it started 60s ago and ended 15s ago, so it ran for 45s.
    done.started_secs = 60;
    done.ended_secs = Some(15);
    info.subagents = vec![done];
    let app = app(vec![info]);

    let inspection = inspect_key(
        &app,
        &NodeKey::Subagent {
            window_id: 1,
            id: "a1".into(),
        },
    );

    assert_eq!(
        pairs(&inspection)
            .into_iter()
            .filter(|(label, _)| *label == "state" || *label == "for")
            .collect::<Vec<_>>(),
        vec![("state", "done"), ("for", "45s")],
        "a finished sub-agent shows the run, not its age"
    );
}
