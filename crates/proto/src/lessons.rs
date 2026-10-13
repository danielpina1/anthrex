//! Milestone 9.15 (decision 32): what a lesson looks like on the wire, the requests on
//! a repository's lessons, and the knowledge graph's data, carried by protocol 24's
//! `RunRequest::Lessons` and `RunReply::{Lessons, LessonsGraph}` (task M9.15.5).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A lesson's layer (decision 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonLayer {
    /// What happened here and why.
    Episodic,
    /// A lasting fact about the repository.
    Semantic,
    /// How work must be done here.
    Procedural,
}

impl LessonLayer {
    /// Its wire name: `episodic`, `semantic`, `procedural`.
    pub fn name(self) -> &'static str {
        match self {
            LessonLayer::Episodic => "episodic",
            LessonLayer::Semantic => "semantic",
            LessonLayer::Procedural => "procedural",
        }
    }
}

/// A lesson's status (decision 18). Active (selectable and drawn): `on_trial`, `kept`,
/// `approved`, `known`. `proposed` is drawn and listed but never selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonStatus {
    OnTrial,
    Kept,
    Proposed,
    Approved,
    Rejected,
    Retired,
    Known,
}

impl LessonStatus {
    /// Its wire name: `on_trial`, `kept`, `proposed`, `approved`, `rejected`,
    /// `retired`, `known` (`anthrex lessons list` and the request refusals).
    pub fn name(self) -> &'static str {
        match self {
            LessonStatus::OnTrial => "on_trial",
            LessonStatus::Kept => "kept",
            LessonStatus::Proposed => "proposed",
            LessonStatus::Approved => "approved",
            LessonStatus::Rejected => "rejected",
            LessonStatus::Retired => "retired",
            LessonStatus::Known => "known",
        }
    }
}

/// Where a lesson came from: a distiller's answer, or a recurring failure signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonOrigin {
    Distilled,
    Signature,
}

/// One lesson as `anthrex lessons list`, the Lessons page and the graph's inspect panel
/// show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonInfo {
    pub id: String,
    pub layer: LessonLayer,
    pub status: LessonStatus,
    pub origin: LessonOrigin,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub files: Vec<String>,
    pub created_at: u64,
    pub last_seen: u64,
    /// At most 10, newest last.
    #[serde(default)]
    pub runs: Vec<String>,
    #[serde(default)]
    pub green: u32,
    #[serde(default)]
    pub outcomes: u32,
    #[serde(default)]
    pub reconfirmed: u32,
    /// A known failure's signature headlines (at most 3).
    #[serde(default)]
    pub headlines: Vec<String>,
    #[serde(default)]
    pub retired_reason: Option<String>,
}

/// A request on the lessons of the repository at `dir` (decision 19).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LessonsRequest {
    List {
        dir: PathBuf,
        #[serde(default)]
        all: bool,
    },
    Decide {
        dir: PathBuf,
        ids: Vec<String>,
        approve: bool,
    },
    Forget {
        dir: PathBuf,
        ids: Vec<String>,
    },
    Graph {
        dir: PathBuf,
    },
}

/// A graph node's kind (decision 42).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeKind {
    Module,
    Root,
    Lesson,
}

/// A graph edge's kind (decision 42).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeKind {
    Touches,
    SharesFiles,
    FromSignature,
}

/// Where the graph's modules came from (decision 41).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleSource {
    CargoCache,
    Profile,
    #[default]
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub kind: GraphNodeKind,
    pub label: String,
    #[serde(default)]
    pub status: Option<LessonStatus>,
    #[serde(default)]
    pub layer: Option<LessonLayer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: GraphEdgeKind,
}

/// The knowledge graph of one repository (decisions 41 to 43).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonGraph {
    pub project: PathBuf,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    #[serde(default)]
    pub lessons: Vec<LessonInfo>,
    #[serde(default)]
    pub omitted: u32,
    #[serde(default)]
    pub module_source: ModuleSource,
}

/// The most nodes a graph holds (decision 43).
pub const GRAPH_NODES_MAX: usize = 300;
/// The longest discard reason kept (decision 26).
pub const DISCARD_REASON_MAX: usize = 500;
