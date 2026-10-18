use proto::{SubagentInfo, SubagentState, WindowInfo};
use std::borrow::Cow;
use std::fmt::Write;
use std::path::Path;
use tui::tree::{self, RowKind, RuntimeCounts, SubagentNode, TreeState};

/// Every sub-agent the daemon reports, however long ago it finished.
const KEEP_ALL: tree::Keep = tree::Keep {
    finished_secs: u64::MAX,
    listed_age_secs: 0,
};

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
    /// The window's place in tree order, counted from 1 across the projects. Not its
    /// `C-b <n>` number: the sidebar does not number an idle orchestrator's window,
    /// and `tree` lists it as a plain window (milestone 9.3's final fix wave, B-M7).
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
            // Milestone 9.19 decision 13: `#<position>` is the `C-b <n>` a client
            // uses, `id <id>` what `ls`, `kill` and `attach` take.
            RowKind::Window { info, position, .. } => writeln!(
                text,
                "  #{position}  id {}  {}  {}  {}  {} {}{}",
                info.id,
                info.name,
                info.runtime.label(),
                info.model.as_deref().unwrap_or("-"),
                info.status.label(),
                common::numbers::duration(info.since_secs),
                tool_suffix(info.tool.as_deref())
            ),
            RowKind::Subagent { info, .. } => writeln!(
                text,
                // Sub-agents sit under a fixed window indent here, so the
                // window's own guide level is dropped: two columns per level.
                "      {} {}{}  {}{}",
                row.guides.chars().skip(2).collect::<String>(),
                info.kind,
                info.label
                    .as_ref()
                    .map(|label| format!(": {label}"))
                    .unwrap_or_default(),
                subagent_time(info),
                tool_suffix(info.tool.as_deref())
            ),
            // `tree::build` lists plain windows only; run rows need the run snapshot.
            RowKind::Run { .. }
            | RowKind::Planner { .. }
            | RowKind::Scout { .. }
            | RowKind::DesignAgent { .. }
            | RowKind::Task { .. }
            | RowKind::Stage { .. }
            | RowKind::Round { .. }
            | RowKind::IdleOrchestrator { .. }
            | RowKind::AgentRound { .. } => Ok(()),
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
                    // `anthrex tree` reads no config (only `attach` does; see
                    // `tui::settings::UiSettings`), so it keeps its pre-M6.9 behaviour
                    // of listing every sub-agent the daemon reports, never hiding an
                    // old finished one the way the client's tree view now can.
                    subagents: tree::subagent_forest(&info.subagents, KEEP_ALL)
                        .into_iter()
                        .map(subagent_json)
                        .collect(),
                }),
            RowKind::Subagent { .. }
            | RowKind::Run { .. }
            | RowKind::Planner { .. }
            | RowKind::Scout { .. }
            | RowKind::DesignAgent { .. }
            | RowKind::Task { .. }
            | RowKind::Stage { .. }
            | RowKind::Round { .. }
            | RowKind::IdleOrchestrator { .. }
            | RowKind::AgentRound { .. } => {}
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

/// `claude 2 · codex 1 · shell 1`, named as `--json` names them; a zero is left out.
fn format_counts(counts: RuntimeCounts) -> String {
    [
        ("claude", counts.claude),
        ("codex", counts.codex),
        ("shell", counts.shell),
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

/// Milestone 9.19 decision 13: `running 3m 10s`, `ran 12s` for a done one, `failed
/// 12s`, `permission 3m 10s`; decision 9: a stopped one with no end reads `done -`.
fn subagent_time(info: &SubagentInfo) -> String {
    let time = subagent_duration(info).map(common::numbers::duration);
    let word = match (info.needs_permission, info.state, &time) {
        (true, ..) => "permission",
        (false, SubagentState::Done, Some(_)) => "ran",
        (false, state, _) => state_label(state),
    };
    format!("{word} {}", time.as_deref().unwrap_or("-"))
}

/// How long a sub-agent has run: its age while running, else its start's age less
/// its end's; `None` for a stopped one whose end never arrived (unknown, not zero).
fn subagent_duration(info: &SubagentInfo) -> Option<u64> {
    match info.state {
        SubagentState::Running => Some(info.started_secs),
        SubagentState::Done | SubagentState::Failed => info
            .ended_secs
            .map(|ended| info.started_secs.saturating_sub(ended)),
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
#[path = "tree_cmd_tests.rs"]
mod tests;
