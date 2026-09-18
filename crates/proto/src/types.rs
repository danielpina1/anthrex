use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

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
            other => Err(format!("unknown runtime '{other}' (expected claude, codex or shell)")),
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

/// The display projection of a window, sent to clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub name: String,
    pub runtime: Runtime,
    pub cwd: PathBuf,
    pub branch: Option<String>,
    pub status: Status,
    pub tool: Option<String>,
    pub since_secs: u64,
    pub last_output_secs: u64,
    pub has_session: bool,
    pub exit: Option<ExitInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    Tui,
    Cli,
    Hook,
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
            branch: Some("feat/api".into()),
            status: Status::Working,
            tool: Some("Bash".into()),
            since_secs: 12,
            last_output_secs: 1,
            has_session: true,
            exit: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"runtime\":\"claude\""));
        assert_eq!(serde_json::from_str::<WindowInfo>(&json).unwrap(), info);
    }
}
