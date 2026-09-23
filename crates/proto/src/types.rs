use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

use crate::run::RunRef;

/// Which program a window runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Claude,
    Codex,
    Shell,
}

impl Runtime {
    pub fn label(self) -> &'static str {
        match self {
            Runtime::Claude => "claude",
            Runtime::Codex => "codex",
            Runtime::Shell => "shell",
        }
    }
}

impl FromStr for Runtime {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "claude" => Ok(Runtime::Claude),
            "codex" => Ok(Runtime::Codex),
            "shell" => Ok(Runtime::Shell),
            other => Err(format!(
                "unknown runtime '{other}' (expected claude, codex or shell)"
            )),
        }
    }
}

impl std::fmt::Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Sidebar status of a window. See spec section 3.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Starting,
    Working,
    Idle,
    Attention,
    Done,
    Exited,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Starting => "starting",
            Status::Working => "working",
            Status::Idle => "idle",
            Status::Attention => "attention",
            Status::Done => "done",
            Status::Exited => "exited",
        }
    }
}

/// How a window's child ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub reason: String,
}

/// What the client asks for when creating a window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSpec {
    pub name: Option<String>,
    pub runtime: Runtime,
    pub cwd: PathBuf,
    pub worktree_branch: Option<String>,
    pub model: Option<String>,
    pub initial_prompt: Option<String>,
}

/// One sub-agent inside a window's session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub state: SubagentState,
    pub tool: Option<String>,
    pub started_secs: u64,
    pub ended_secs: Option<u64>,
    pub needs_permission: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubagentState {
    Running,
    Done,
    Failed,
}

/// Which program a window runs under the hood: an interactive PTY, or a headless agent
/// session run by the orchestration engine (from M8a.17).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowKind {
    #[default]
    Pty,
    Headless,
}

/// The display projection of a window, sent to clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub name: String,
    pub runtime: Runtime,
    pub cwd: PathBuf,
    pub project: PathBuf,
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
    pub status: Status,
    pub tool: Option<String>,
    pub since_secs: u64,
    pub last_output_secs: u64,
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub subagents: Vec<SubagentInfo>,
    pub exit: Option<ExitInfo>,
    #[serde(default)]
    pub kind: WindowKind,
    #[serde(default)]
    pub run: Option<RunRef>,
}

/// Where a window's `HEAD` points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Head {
    Branch(String),
    Detached(String), // short oid
    Unborn(String),   // the branch name that does not exist yet
}

/// A git operation in progress in a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOperation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
}

/// The git status of one worktree, as last probed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitState {
    pub head: Head,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub dirty: u32,
    pub untracked: u32,
    pub conflicts: u32,
    pub operation: Option<GitOperation>,
    pub stale: bool,
}

impl GitState {
    /// True when there is nothing uncommitted, no divergence and no operation in
    /// progress.
    ///
    /// This is the *single* definition of "clean": the TUI's bottom bar asks this
    /// rather than deciding a second time from the parts it happens to be about to
    /// render. An in-progress operation counts, which is the part that is easy to
    /// leave out — a worktree halfway through a rebase with no dirty files is not a
    /// worktree anyone should be told is clean.
    pub fn is_clean(&self) -> bool {
        self.dirty == 0
            && self.untracked == 0
            && self.conflicts == 0
            && self.ahead == 0
            && self.behind == 0
            && self.operation.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    Tui,
    Cli,
    Hook,
    Mcp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_parses_and_labels() {
        assert_eq!("claude".parse::<Runtime>().unwrap(), Runtime::Claude);
        assert_eq!("CODEX".parse::<Runtime>().unwrap(), Runtime::Codex);
        assert_eq!(Runtime::Shell.label(), "shell");
        assert!("perl".parse::<Runtime>().is_err());
    }

    #[test]
    fn status_labels_are_lowercase_words() {
        assert_eq!(Status::Attention.label(), "attention");
        assert_eq!(Status::Exited.label(), "exited");
    }

    #[test]
    fn window_info_round_trips_through_json() {
        let info = WindowInfo {
            id: 7,
            name: "api".into(),
            runtime: Runtime::Claude,
            cwd: "/tmp/repo".into(),
            project: "/tmp/repo".into(),
            worktree: Some("/tmp/repo".into()),
            branch: Some("feat/api".into()),
            status: Status::Working,
            tool: Some("Bash".into()),
            since_secs: 12,
            last_output_secs: 1,
            session_id: Some("s1".into()),
            model: Some("opus".into()),
            subagents: vec![SubagentInfo {
                id: "agent-1".into(),
                parent_id: Some("parent-1".into()),
                kind: "explore".into(),
                label: Some("Find call sites".into()),
                model: Some("haiku".into()),
                state: SubagentState::Running,
                tool: Some("Read".into()),
                started_secs: 9,
                ended_secs: Some(11),
                needs_permission: true,
            }],
            exit: None,
            kind: WindowKind::Pty,
            run: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"project\":\"/tmp/repo\""));
        assert!(json.contains("\"runtime\":\"claude\""));
        assert!(json.contains("\"state\":\"running\""));
        assert!(!json.contains("has_session"));
        assert_eq!(serde_json::from_str::<WindowInfo>(&json).unwrap(), info);
    }

    #[test]
    fn window_info_carries_the_worktree() {
        let mut info = WindowInfo {
            id: 7,
            name: "api".into(),
            runtime: Runtime::Claude,
            cwd: "/tmp/repo".into(),
            project: "/tmp/repo".into(),
            worktree: Some("/tmp/repo".into()),
            branch: None,
            status: Status::Working,
            tool: None,
            since_secs: 12,
            last_output_secs: 1,
            session_id: None,
            model: None,
            subagents: vec![],
            exit: None,
            kind: WindowKind::Pty,
            run: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"worktree\":\"/tmp/repo\""));
        assert_eq!(serde_json::from_str::<WindowInfo>(&json).unwrap(), info);

        info.worktree = None;
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"worktree\":null"));
        assert_eq!(serde_json::from_str::<WindowInfo>(&json).unwrap(), info);
    }

    #[test]
    fn git_state_round_trips_through_messagepack() {
        let full = GitState {
            head: Head::Branch("main".into()),
            upstream: Some("origin/main".into()),
            ahead: 2,
            behind: 1,
            dirty: 3,
            untracked: 1,
            conflicts: 0,
            operation: Some(GitOperation::Rebase),
            stale: false,
        };
        let packed = rmp_serde::to_vec_named(&full).unwrap();
        let back: GitState = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(back, full);

        let minimal = GitState {
            head: Head::Detached("a1b2c3d".into()),
            upstream: None,
            ahead: 0,
            behind: 0,
            dirty: 0,
            untracked: 0,
            conflicts: 0,
            operation: None,
            stale: true,
        };
        let packed = rmp_serde::to_vec_named(&minimal).unwrap();
        let back: GitState = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(back, minimal);
    }

    #[test]
    fn head_variants_serialize_snake_case() {
        assert_eq!(
            serde_json::to_value(Head::Branch("main".into())).unwrap(),
            serde_json::json!({"branch": "main"})
        );
        assert_eq!(
            serde_json::to_value(Head::Detached("a1b2c3d".into())).unwrap(),
            serde_json::json!({"detached": "a1b2c3d"})
        );
        assert_eq!(
            serde_json::to_value(Head::Unborn("main".into())).unwrap(),
            serde_json::json!({"unborn": "main"})
        );
    }

    #[test]
    fn is_clean_is_true_only_when_nothing_is_pending() {
        let clean = GitState {
            head: Head::Branch("main".into()),
            upstream: Some("origin/main".into()),
            ahead: 0,
            behind: 0,
            dirty: 0,
            untracked: 0,
            conflicts: 0,
            operation: None,
            stale: false,
        };
        assert!(clean.is_clean());

        let mut dirty = clean.clone();
        dirty.dirty = 1;
        assert!(!dirty.is_clean());

        let mut untracked = clean.clone();
        untracked.untracked = 1;
        assert!(!untracked.is_clean());

        let mut conflicts = clean.clone();
        conflicts.conflicts = 1;
        assert!(!conflicts.is_clean());

        let mut ahead = clean.clone();
        ahead.ahead = 1;
        assert!(!ahead.is_clean());

        let mut behind = clean.clone();
        behind.behind = 1;
        assert!(!behind.is_clean());

        // A worktree mid-rebase with a spotless tree is the case this used to get
        // wrong: every count is zero, but there is an operation in progress, and the
        // bottom bar must not be able to derive a tick from it.
        let mut rebasing = clean.clone();
        rebasing.operation = Some(GitOperation::Rebase);
        assert!(!rebasing.is_clean());
    }

    #[test]
    fn subagent_state_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&SubagentState::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&SubagentState::Done).unwrap(),
            "\"done\""
        );
        assert_eq!(
            serde_json::to_string(&SubagentState::Failed).unwrap(),
            "\"failed\""
        );
    }

    fn base_window_json() -> serde_json::Value {
        serde_json::json!({
            "id": 7, "name": "api", "runtime": "claude",
            "cwd": "/tmp/repo", "project": "/tmp/repo", "worktree": null, "branch": null,
            "status": "working", "tool": null, "since_secs": 12,
            "last_output_secs": 1, "session_id": null, "model": null,
            "subagents": [], "exit": null
        })
    }

    #[test]
    fn window_info_run_defaults_to_none() {
        let info: WindowInfo = serde_json::from_value(base_window_json()).unwrap();
        assert_eq!(info.run, None);
    }

    #[test]
    fn window_info_kind_defaults_to_pty() {
        let info: WindowInfo = serde_json::from_value(base_window_json()).unwrap();
        assert_eq!(info.kind, WindowKind::Pty);

        let json = serde_json::to_string(&WindowKind::Headless).unwrap();
        assert_eq!(json, "\"headless\"");
        assert_eq!(
            serde_json::from_str::<WindowKind>(&json).unwrap(),
            WindowKind::Headless
        );
    }
}
