mod support;

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{RunningCommand, TestDaemon, tempdir};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

struct GitFixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    repo_sub: PathBuf,
    worktree: PathBuf,
    plain: PathBuf,
}

impl GitFixture {
    fn new() -> Self {
        let temp = tempdir();
        let repo = temp.path().join("repo");
        let repo_sub = repo.join("sub");
        let worktree = temp.path().join("wt");
        let plain = temp.path().join("plain");

        run_git(&[OsStr::new("init"), repo.as_os_str()]);
        run_git(&[
            OsStr::new("-C"),
            repo.as_os_str(),
            OsStr::new("commit"),
            OsStr::new("--allow-empty"),
            OsStr::new("-m"),
            OsStr::new("init"),
        ]);
        std::fs::create_dir_all(&repo_sub).unwrap();
        std::fs::create_dir_all(&plain).unwrap();
        run_git(&[
            OsStr::new("-C"),
            repo.as_os_str(),
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-b"),
            OsStr::new("feature"),
            worktree.as_os_str(),
        ]);

        Self {
            repo: repo.canonicalize().unwrap(),
            repo_sub: repo_sub.canonicalize().unwrap(),
            worktree: worktree.canonicalize().unwrap(),
            plain: plain.canonicalize().unwrap(),
            _temp: temp,
        }
    }
}

fn run_git(args: &[&OsStr]) {
    let args = args
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect::<Vec<OsString>>();
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(&args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    let output = RunningCommand::start(&mut command).finish(GIT_TIMEOUT);
    assert!(
        output.status.success(),
        "git {args:?} failed with {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn agent_script() -> Vec<Value> {
    vec![
        json!({"hook":"SessionStart","payload":{}}),
        json!({"hook":"PreToolUse","payload":{"tool_name":"Agent","tool_input":{"subagent_type":"Explore","prompt":"map routes\nin detail"}}}),
        json!({"hook":"SubagentStart","payload":{"agent_id":"a1","agent_type":"Explore"}}),
        json!({"hook":"PreToolUse","payload":{"agent_id":"a1","tool_name":"Agent","tool_input":{"subagent_type":"general-purpose","prompt":"grep handlers"}}}),
        json!({"hook":"SubagentStart","payload":{"agent_id":"a2","agent_type":"general-purpose"}}),
        json!({"hook":"PreToolUse","payload":{"agent_id":"a1","tool_name":"Read","tool_input":{}}}),
        json!({"read_line":true}),
    ]
}

fn create_windows(daemon: &TestDaemon, fixture: &GitFixture) {
    let ids = [
        create_window(daemon, "claude", "main-agent", &fixture.repo_sub),
        create_window(daemon, "claude", "wt-agent", &fixture.worktree),
        create_window(daemon, "shell", "loose", &fixture.plain),
    ];
    assert_eq!(
        ids.iter().copied().collect::<BTreeSet<_>>().len(),
        3,
        "window creation returned duplicate ids: {ids:?}",
    );
}

fn create_window(daemon: &TestDaemon, runtime: &str, name: &str, dir: &Path) -> u32 {
    let output = daemon.anthrex(&[
        "new",
        "--runtime",
        runtime,
        "--name",
        name,
        "--dir",
        dir.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "creating {name} failed with {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.stderr.is_empty(),
        "creating {name} wrote stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    assert_eq!(
        stdout.lines().count(),
        1,
        "creating {name} did not print exactly one id: {stdout:?}",
    );
    stdout
        .trim()
        .parse::<u32>()
        .unwrap_or_else(|error| panic!("creating {name} printed an invalid id {stdout:?}: {error}"))
}

fn tree_json(daemon: &TestDaemon) -> Value {
    let output = daemon.anthrex(&["tree", "--json"]);
    assert!(
        output.status.success(),
        "tree --json failed with {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.stderr.is_empty(),
        "tree --json wrote stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "tree --json returned invalid JSON: {error}; stdout: {}",
            String::from_utf8_lossy(&output.stdout),
        )
    })
}

fn wait_for_tree(daemon: &TestDaemon, description: &str, ready: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + READY_TIMEOUT;
    let mut last = tree_json(daemon);
    loop {
        if ready(&last) {
            return last;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}; last tree JSON: {last:#}",
        );
        std::thread::sleep(POLL_INTERVAL);
        last = tree_json(daemon);
    }
}

fn projects(tree: &Value) -> &[Value] {
    tree["projects"]
        .as_array()
        .expect("tree projects must be an array")
}

fn project_with_root<'a>(tree: &'a Value, root: &Path) -> &'a Value {
    let root = root.display().to_string();
    projects(tree)
        .iter()
        .find(|project| project["root"] == root)
        .unwrap_or_else(|| panic!("missing project root {root:?} in {tree:#}"))
}

fn windows(project: &Value) -> &[Value] {
    project["windows"]
        .as_array()
        .expect("project windows must be an array")
}

fn window<'a>(project: &'a Value, name: &str) -> &'a Value {
    windows(project)
        .iter()
        .find(|window| window["name"] == name)
        .unwrap_or_else(|| panic!("missing window {name:?} in {project:#}"))
}

fn window_names(project: &Value) -> BTreeSet<&str> {
    windows(project)
        .iter()
        .map(|window| {
            window["name"]
                .as_str()
                .expect("window name must be a string")
        })
        .collect()
}

fn all_window_names(tree: &Value) -> BTreeSet<&str> {
    projects(tree)
        .iter()
        .flat_map(windows)
        .map(|window| {
            window["name"]
                .as_str()
                .expect("window name must be a string")
        })
        .collect()
}

fn has_nested_agents(tree: &Value, name: &str) -> bool {
    projects(tree)
        .iter()
        .flat_map(windows)
        .find(|window| window["name"] == name)
        .and_then(|window| window["subagents"].as_array())
        .and_then(|agents| agents.iter().find(|agent| agent["id"] == "a1"))
        .filter(|agent| agent["state"] == "running" && agent["tool"] == "Read")
        .and_then(|agent| agent["children"].as_array())
        .is_some_and(|children| {
            children
                .iter()
                .any(|agent| agent["id"] == "a2" && agent["state"] == "running")
        })
}

fn assert_nested_agents(window: &Value) {
    let agents = window["subagents"]
        .as_array()
        .expect("window subagents must be an array");
    assert_eq!(agents.len(), 1, "unexpected top-level agents: {agents:#?}");

    let a1 = &agents[0];
    assert_eq!(a1["id"], "a1");
    assert!(a1["parent_id"].is_null());
    assert_eq!(a1["kind"], "Explore");
    assert_eq!(a1["label"], "map routes");
    assert_eq!(a1["state"], "running");
    assert_eq!(a1["tool"], "Read");

    let children = a1["children"]
        .as_array()
        .expect("a1 children must be an array");
    assert_eq!(children.len(), 1, "unexpected a1 children: {children:#?}");
    let a2 = &children[0];
    assert_eq!(a2["id"], "a2");
    assert_eq!(a2["parent_id"], "a1");
    assert_eq!(a2["kind"], "general-purpose");
    assert_eq!(a2["label"], "grep handlers");
    assert_eq!(a2["state"], "running");
    assert_eq!(a2["children"], json!([]));
}

#[test]
fn tree_json_groups_worktree_agents_and_nests_subagents() {
    let fixture = GitFixture::new();
    let daemon = TestDaemon::start(&agent_script());
    create_windows(&daemon, &fixture);

    let tree = wait_for_tree(&daemon, "both Claude nested-agent trees", |tree| {
        has_nested_agents(tree, "main-agent") && has_nested_agents(tree, "wt-agent")
    });

    assert_eq!(projects(&tree).len(), 2, "unexpected projects: {tree:#}");

    let repo_project = project_with_root(&tree, &fixture.repo);
    assert_eq!(
        window_names(repo_project),
        BTreeSet::from(["main-agent", "wt-agent"]),
    );
    assert_nested_agents(window(repo_project, "main-agent"));
    assert_nested_agents(window(repo_project, "wt-agent"));

    let plain_project = project_with_root(&tree, &fixture.plain);
    assert_eq!(window_names(plain_project), BTreeSet::from(["loose"]));

    drop(daemon);
}

#[test]
fn tree_text_and_project_filter() {
    let fixture = GitFixture::new();
    let daemon = TestDaemon::start(&agent_script());
    create_windows(&daemon, &fixture);

    wait_for_tree(&daemon, "all three windows", |tree| {
        all_window_names(tree) == BTreeSet::from(["loose", "main-agent", "wt-agent"])
    });

    let output = daemon.anthrex(&["tree", "--project", fixture.worktree.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "tree --project failed with {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.stderr.is_empty(),
        "tree --project wrote stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    assert!(
        stdout.contains(&format!("repo  {}  ", fixture.repo.display())),
        "tree did not print the canonical main repository: {stdout:?}",
    );
    assert!(
        stdout.contains("main-agent"),
        "missing main-agent: {stdout:?}"
    );
    assert!(stdout.contains("wt-agent"), "missing wt-agent: {stdout:?}");
    assert!(
        !stdout.contains("loose"),
        "included loose window: {stdout:?}"
    );
    assert_ne!(stdout, "no windows\n");

    drop(daemon);
}
