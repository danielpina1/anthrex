use proto::{SubagentInfo, SubagentState, WindowInfo};
use std::borrow::Cow;
use std::fmt::Write;
use std::path::Path;
use tui::tree::{self, RowKind, RuntimeCounts, SubagentNode, TreeState};

#[derive(Clone, Copy)]
pub struct ProjectQuery<'a> {
    requested: &'a Path,
    normalized: &'a Path,
}

impl<'a> ProjectQuery<'a> {
    pub fn new(requested: &'a Path, normalized: &'a Path) -> Self {
        Self {
            requested,
            normalized,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct TreeJson {
    pub projects: Vec<ProjectJson>,
}

#[derive(Debug, serde::Serialize)]
pub struct ProjectJson {
    pub root: String,
    pub name: String,
    pub status: String,
    pub counts: CountsJson,
    pub windows: Vec<WindowJson>,
}

#[derive(Debug, serde::Serialize)]
pub struct CountsJson {
    pub claude: usize,
    pub codex: usize,
    pub shell: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct WindowJson {
    pub id: u32,
    pub position: usize,
    pub name: String,
    pub runtime: String,
    pub model: Option<String>,
    pub status: String,
    pub since_secs: u64,
    pub tool: Option<String>,
    pub cwd: String,
    pub branch: Option<String>,
    pub session_id: Option<String>,
    pub subagents: Vec<SubagentJson>,
}

#[derive(Debug, serde::Serialize)]
pub struct SubagentJson {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub state: String,
    pub tool: Option<String>,
    pub started_secs: u64,
    pub ended_secs: Option<u64>,
    pub needs_permission: bool,
    pub children: Vec<SubagentJson>,
}

pub fn tree_text(windows: &[WindowInfo], project: Option<ProjectQuery<'_>>) -> String {
    let windows = selected_windows(windows, project);
    if windows.is_empty() {
        return "no windows\n".into();
    }
    let mut text = String::new();
    for row in tree::build(&windows, &TreeState::default()) {
        match row.kind {
            RowKind::Project {
                root,
                name,
                status,
                counts,
                ..
            } => writeln!(
                text,
                "{name}  {}  {}  {}",
                root.display(),
                status.label(),
                format_counts(counts)
            ),
            RowKind::Window { info, position, .. } => writeln!(
                text,
                "  {position:>2}  {}  {}  {}  {}  {}{}",
                info.name,
                info.runtime.label(),
                info.model.as_deref().unwrap_or("-"),
                info.status.label(),
                tree::format_elapsed(info.since_secs),
                tool_suffix(info.tool.as_deref())
            ),
            RowKind::Subagent { info, .. } => writeln!(
                text,
                // Sub-agents sit under a fixed window indent here, so the
                // window's own guide level is dropped: two columns per level.
                "      {} {}{}  {}  {}{}",
                row.guides.chars().skip(2).collect::<String>(),
                info.kind,
                info.label
                    .as_ref()
                    .map(|label| format!(": {label}"))
                    .unwrap_or_default(),
                if info.needs_permission {
                    "permission"
                } else {
                    state_label(info.state)
                },
                tree::format_elapsed(subagent_duration(info)),
                tool_suffix(info.tool.as_deref())
            ),
        }
        .expect("writing to a String cannot fail");
    }
    text
}

pub fn tree_json(windows: &[WindowInfo], project: Option<ProjectQuery<'_>>) -> TreeJson {
    let windows = selected_windows(windows, project);
    let mut result = TreeJson {
        projects: Vec::new(),
    };
    for row in tree::build(&windows, &TreeState::default()) {
        match row.kind {
            RowKind::Project {
                root,
                name,
                status,
                counts,
                ..
            } => result.projects.push(ProjectJson {
                root: root.display().to_string(),
                name,
                status: status.label().into(),
                counts: CountsJson {
                    claude: counts.claude,
                    codex: counts.codex,
                    shell: counts.shell,
                },
                windows: Vec::new(),
            }),
            RowKind::Window { info, position, .. } => result
                .projects
                .last_mut()
                .expect("tree windows follow their project")
                .windows
                .push(WindowJson {
                    id: info.id,
                    position,
                    name: info.name.clone(),
                    runtime: info.runtime.label().into(),
                    model: info.model.clone(),
                    status: info.status.label().into(),
                    since_secs: info.since_secs,
                    tool: info.tool.clone(),
                    cwd: info.cwd.display().to_string(),
                    branch: info.branch.clone(),
                    session_id: info.session_id.clone(),
                    subagents: tree::subagent_forest(&info.subagents)
                        .into_iter()
                        .map(subagent_json)
                        .collect(),
                }),
            RowKind::Subagent { .. } => {}
        }
    }
    result
}

fn selected_windows<'a>(
    windows: &'a [WindowInfo],
    project: Option<ProjectQuery<'_>>,
) -> Cow<'a, [WindowInfo]> {
    let Some(project) = project else {
        return Cow::Borrowed(windows);
    };
    let exact = |query: &Path| {
        windows
            .iter()
            .map(|window| window.project.as_path())
            .find(|root| *root == query)
    };
    let containing = |query: &Path| {
        windows
            .iter()
            .map(|window| window.project.as_path())
            .filter(|root| query.starts_with(root))
            .max_by_key(|root| root.components().count())
    };
    let root = exact(project.requested)
        .or_else(|| containing(project.requested))
        .or_else(|| exact(project.normalized))
        .or_else(|| containing(project.normalized));
    Cow::Owned(
        windows
            .iter()
            .filter(|window| Some(window.project.as_path()) == root)
            .cloned()
            .collect(),
    )
}

fn format_counts(counts: RuntimeCounts) -> String {
    [
        ("cl", counts.claude),
        ("cx", counts.codex),
        ("sh", counts.shell),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(runtime, count)| format!("{runtime} {count}"))
    .collect::<Vec<_>>()
    .join(" · ")
}

fn tool_suffix(tool: Option<&str>) -> String {
    tool.map(|tool| format!("  {tool}")).unwrap_or_default()
}

fn state_label(state: SubagentState) -> &'static str {
    match state {
        SubagentState::Running => "running",
        SubagentState::Done => "done",
        SubagentState::Failed => "failed",
    }
}

fn subagent_duration(info: &SubagentInfo) -> u64 {
    match info.state {
        SubagentState::Running => info.started_secs,
        SubagentState::Done | SubagentState::Failed => info
            .ended_secs
            .map(|ended| info.started_secs.saturating_sub(ended))
            .unwrap_or(0),
    }
}

fn subagent_json(node: SubagentNode<'_>) -> SubagentJson {
    let info = node.info;
    SubagentJson {
        id: info.id.clone(),
        parent_id: info.parent_id.clone(),
        kind: info.kind.clone(),
        label: info.label.clone(),
        model: info.model.clone(),
        state: state_label(info.state).into(),
        tool: info.tool.clone(),
        started_secs: info.started_secs,
        ended_secs: info.ended_secs,
        needs_permission: info.needs_permission,
        children: node.children.into_iter().map(subagent_json).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{Runtime, Status, SubagentInfo, SubagentState};
    use serde_json::{Value, json};

    fn window(id: u32, name: &str, project: &str) -> WindowInfo {
        WindowInfo {
            id,
            name: name.into(),
            runtime: Runtime::Claude,
            cwd: project.into(),
            project: project.into(),
            worktree: None,
            branch: None,
            status: Status::Working,
            tool: None,
            since_secs: 60,
            last_output_secs: 0,
            session_id: None,
            model: None,
            subagents: Vec::new(),
            exit: None,
        }
    }

    fn subagent(id: &str, kind: &str, label: &str, started_secs: u64) -> SubagentInfo {
        SubagentInfo {
            id: id.into(),
            parent_id: None,
            kind: kind.into(),
            label: Some(label.into()),
            model: None,
            state: SubagentState::Running,
            tool: None,
            started_secs,
            ended_secs: None,
            needs_permission: false,
        }
    }

    fn example() -> Vec<WindowInfo> {
        let mut api = window(1, "api-worker", "/r/shop");
        api.model = Some("claude-opus-5".into());
        api.tool = Some("Bash".into());
        api.since_secs = 120;
        api.session_id = Some("session-api".into());
        let mut explore = subagent("a1", "Explore", "map routes", 90);
        explore.tool = Some("Read".into());
        let mut child = subagent("a2", "general-purpose", "grep handlers", 20);
        child.parent_id = Some("a1".into());
        let mut tests = subagent("a3", "tests", "run unit suite", 60);
        tests.state = SubagentState::Done;
        tests.ended_secs = Some(15);
        api.subagents = vec![explore, child, tests];
        let mut billing = window(2, "billing", "/r/shop");
        billing.runtime = Runtime::Codex;
        billing.status = Status::Attention;
        billing.since_secs = 41;
        let mut frontend = window(4, "frontend", "/r/shop");
        frontend.model = Some("claude-sonnet-4-5".into());
        frontend.subagents = api.subagents.clone();
        let mut blog = window(8, "notes", "/r/blog");
        blog.status = Status::Idle;
        blog.since_secs = 30;
        vec![api, billing, frontend, blog]
    }

    fn query(path: &Path) -> ProjectQuery<'_> {
        ProjectQuery::new(path, path)
    }

    fn as_json(windows: &[WindowInfo], project: Option<ProjectQuery<'_>>) -> Value {
        serde_json::to_value(tree_json(windows, project)).unwrap()
    }

    #[test]
    fn text_lists_projects_windows_and_nested_subagents() {
        let text = tree_text(&example(), None);
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(
            lines.first().copied(),
            Some("shop  /r/shop  attention  cl 2 · cx 1")
        );
        assert!(lines.contains(&"   1  api-worker  claude  claude-opus-5  working  2m  Bash"));
        assert!(lines.contains(&"   2  billing  codex  -  attention  41s"));
        assert!(lines.contains(&"   3  frontend  claude  claude-sonnet-4-5  working  1m"));
        assert!(lines.contains(&"      ├─ Explore: map routes  running  1m  Read"));
        assert!(lines.contains(&"      │ └─ general-purpose: grep handlers  running  20s"));
        assert!(lines.contains(&"      └─ tests: run unit suite  done  45s"));
        assert!(lines.contains(&"blog  /r/blog  idle  cl 1"));
        assert_eq!(
            lines.last().copied(),
            Some("   4  notes  claude  -  idle  30s")
        );
        assert!(text.ends_with('\n'));
        assert!(!text.ends_with("\n\n"));
    }

    #[test]
    fn json_has_the_documented_shape() {
        let value = as_json(&example(), None);
        assert_eq!(value["projects"][0]["name"], "shop");
        assert_eq!(value.as_object().unwrap().len(), 1);
        let shop = &value["projects"][0];
        assert_eq!(shop["root"], "/r/shop");
        assert_eq!(shop["status"], "attention");
        assert_eq!(shop["counts"], json!({"claude": 2, "codex": 1, "shell": 0}));
        assert_eq!(shop["windows"][0]["position"], 1);
        assert_eq!(shop["windows"][2]["id"], 4);
        assert_eq!(shop["windows"][2]["position"], 3);
        let first = &shop["windows"][0];
        assert_eq!(
            first,
            &json!({
                "id": 1, "position": 1, "name": "api-worker", "runtime": "claude",
                "model": "claude-opus-5", "status": "working", "since_secs": 120,
                "tool": "Bash", "cwd": "/r/shop", "branch": null,
                "session_id": "session-api", "subagents": [
                    {
                        "id": "a1", "parent_id": null, "kind": "Explore",
                        "label": "map routes", "model": null, "state": "running",
                        "tool": "Read", "started_secs": 90, "ended_secs": null,
                        "needs_permission": false, "children": [{
                            "id": "a2", "parent_id": "a1", "kind": "general-purpose",
                            "label": "grep handlers", "model": null, "state": "running",
                            "tool": null, "started_secs": 20, "ended_secs": null,
                            "needs_permission": false, "children": []
                        }]
                    },
                    {
                        "id": "a3", "parent_id": null, "kind": "tests",
                        "label": "run unit suite", "model": null, "state": "done",
                        "tool": null, "started_secs": 60, "ended_secs": 15,
                        "needs_permission": false, "children": []
                    }
                ]
            })
        );
        let billing = &shop["windows"][1];
        for field in ["model", "tool", "branch", "session_id"] {
            assert_eq!(billing.get(field), Some(&Value::Null), "{field}");
        }
        assert_eq!(billing["subagents"], json!([]));
        assert_eq!(value["projects"][1]["name"], "blog");
        assert_eq!(value["projects"][1]["windows"][0]["position"], 4);
    }

    #[test]
    fn project_filter_picks_the_longest_containing_root() {
        let windows = vec![window(9, "root", "/r"), window(4, "shop", "/r/shop")];
        for (path, root, name) in [
            ("/r/shop/src", "/r/shop", "shop"),
            ("/r/shop", "/r/shop", "shop"),
            ("/r/other", "/r", "r"),
            ("/r/shopper", "/r", "r"),
        ] {
            let value = as_json(&windows, Some(query(Path::new(path))));
            assert_eq!(value["projects"][0]["root"], root, "{path}");
            assert_eq!(value["projects"].as_array().unwrap().len(), 1);
            assert_eq!(value["projects"][0]["windows"][0]["position"], 1);
            let text = tree_text(&windows, Some(query(Path::new(path))));
            assert_eq!(text.lines().count(), 2);
            assert_eq!(
                text.lines().next(),
                Some(format!("{name}  {root}  working  cl 1").as_str())
            );
        }
        assert_eq!(
            tree_text(&windows, Some(query(Path::new("/elsewhere")))),
            "no windows\n"
        );
        assert_eq!(
            as_json(&windows, Some(query(Path::new("/elsewhere")))),
            json!({"projects": []})
        );
    }

    #[test]
    fn project_filter_prefers_recorded_request_before_normalized_fallback() {
        let windows = vec![
            window(1, "immutable", "/late/sub"),
            window(2, "normalized", "/late"),
        ];

        let recorded = ProjectQuery::new(Path::new("/late/sub"), Path::new("/late"));
        let value = as_json(&windows, Some(recorded));
        assert_eq!(value["projects"].as_array().unwrap().len(), 1);
        assert_eq!(value["projects"][0]["root"], "/late/sub");
        assert_eq!(value["projects"][0]["windows"][0]["name"], "immutable");

        let normalized = ProjectQuery::new(Path::new("/external/wt"), Path::new("/late"));
        let value = as_json(&windows, Some(normalized));
        assert_eq!(value["projects"].as_array().unwrap().len(), 1);
        assert_eq!(value["projects"][0]["root"], "/late");
        assert_eq!(value["projects"][0]["windows"][0]["name"], "normalized");
    }

    #[test]
    fn empty_list_prints_no_windows() {
        assert_eq!(tree_text(&[], None), "no windows\n");
        assert_eq!(as_json(&[], None), json!({"projects": []}));
    }

    #[test]
    fn permission_is_printed_and_serialized() {
        let mut windows = example();
        windows[0].subagents[0].needs_permission = true;
        assert!(
            tree_text(&windows, None)
                .lines()
                .any(|line| { line == "      ├─ Explore: map routes  permission  1m  Read" })
        );
        let value = as_json(&windows, None);
        let agent = &value["projects"][0]["windows"][0]["subagents"][0];
        assert_eq!(agent["needs_permission"], true);
        assert_eq!(agent["state"], "running");
    }

    #[test]
    fn subagent_states_and_durations_use_the_cli_contract() {
        for (state, ended, label, elapsed) in [
            (SubagentState::Running, None, "running", "1m"),
            (SubagentState::Done, Some(15), "done", "45s"),
            (SubagentState::Failed, Some(10), "failed", "50s"),
            (SubagentState::Done, None, "done", "0s"),
            (SubagentState::Failed, Some(90), "failed", "0s"),
        ] {
            let mut window = window(1, "worker", "/r/shop");
            let mut agent = subagent("a1", "tests", "", 60);
            agent.label = None;
            agent.state = state;
            agent.ended_secs = ended;
            window.subagents = vec![agent];
            let windows = [window];
            let text = tree_text(&windows, None);
            assert_eq!(
                text.lines().last(),
                Some(format!("      └─ tests  {label}  {elapsed}").as_str())
            );
            let value = as_json(&windows, None);
            let agent = &value["projects"][0]["windows"][0]["subagents"][0];
            assert_eq!(agent["state"], label);
            assert_eq!(agent["started_secs"], 60);
            assert_eq!(agent["ended_secs"], json!(ended));
            assert_eq!(agent.get("label"), Some(&Value::Null));
            assert!(agent.get("duration").is_none());
        }
    }

    #[test]
    fn counts_omit_zero_runtimes_and_keep_runtime_order() {
        let mut windows = example();
        let mut shell = window(5, "terminal", "/r/shop");
        shell.runtime = Runtime::Shell;
        windows.push(shell);
        assert_eq!(
            tree_text(&windows, None).lines().next(),
            Some("shop  /r/shop  attention  cl 2 · cx 1 · sh 1")
        );
        assert_eq!(as_json(&windows, None)["projects"][0]["counts"]["shell"], 1);
    }
}
