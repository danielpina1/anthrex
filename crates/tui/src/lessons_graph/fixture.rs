//! The fixed eight-node graph the graph page's tests share (task M9.15.15): two module
//! hubs, the root hub, a known failure, an on-trial, a proposed, an approved and a kept
//! lesson.

use proto::{
    GraphEdge, GraphEdgeKind, GraphNode, GraphNodeKind, LessonGraph, LessonInfo, LessonLayer,
    LessonOrigin, LessonStatus, ModuleSource,
};

pub const BILLING: &str = "m:billing";
pub const WEB: &str = "m:web";
pub const ROOT: &str = "m:(root)";
pub const KNOWN: &str = "kf-0badf00d";
pub const TRIAL: &str = "l-11111111";
pub const PROPOSED: &str = "l-22222222";
pub const APPROVED: &str = "l-33333333";
pub const KEPT: &str = "l-44444444";

/// A lesson's details.
pub fn info(id: &str, status: LessonStatus, title: &str, files: &[&str]) -> LessonInfo {
    LessonInfo {
        id: id.into(),
        layer: LessonLayer::Procedural,
        status,
        origin: if status == LessonStatus::Known {
            LessonOrigin::Signature
        } else {
            LessonOrigin::Distilled
        },
        title: title.into(),
        body: format!("The body of {title}.\nIt has a second line."),
        files: files.iter().map(|f| f.to_string()).collect(),
        created_at: 1_760_000_000,
        // 2025-10-13 (UTC).
        last_seen: 1_760_313_600,
        runs: vec!["r-0001".into(), "r-0002".into()],
        green: 3,
        outcomes: 4,
        reconfirmed: 2,
        headlines: if status == LessonStatus::Known {
            vec!["error[E0308]: mismatched types".into()]
        } else {
            vec![]
        },
        retired_reason: None,
    }
}

fn node(id: &str, kind: GraphNodeKind, label: &str, status: Option<LessonStatus>) -> GraphNode {
    GraphNode {
        id: id.into(),
        kind,
        label: label.into(),
        status,
        layer: status.map(|_| LessonLayer::Procedural),
    }
}

fn edge(from: &str, to: &str, kind: GraphEdgeKind) -> GraphEdge {
    GraphEdge {
        from: from.into(),
        to: to.into(),
        kind,
    }
}

/// The eight-node graph.
pub fn graph() -> LessonGraph {
    use GraphEdgeKind::*;
    use GraphNodeKind::*;
    let lessons = vec![
        info(
            KNOWN,
            LessonStatus::Known,
            "cargo test fails on a type mismatch",
            &["crates/billing/src/**"],
        ),
        info(
            TRIAL,
            LessonStatus::OnTrial,
            "Run migrations before tests",
            &["crates/billing/migrations/*"],
        ),
        info(
            PROPOSED,
            LessonStatus::Proposed,
            "Prefer the Invoice builder",
            &["crates/billing/src/invoice.rs"],
        ),
        info(
            APPROVED,
            LessonStatus::Approved,
            "Templates live under web/templates",
            &["crates/web/templates/**"],
        ),
        info(KEPT, LessonStatus::Kept, "Keep the CHANGELOG sorted", &[]),
    ];
    let mut nodes = vec![
        node(ROOT, Root, "(root)", None),
        node(BILLING, Module, "billing", None),
        node(WEB, Module, "web", None),
    ];
    nodes.extend(
        lessons
            .iter()
            .map(|l| node(&l.id, Lesson, &l.title, Some(l.status))),
    );
    LessonGraph {
        project: "/p/shop".into(),
        nodes,
        edges: vec![
            edge(KNOWN, BILLING, Touches),
            edge(TRIAL, BILLING, Touches),
            edge(PROPOSED, BILLING, Touches),
            edge(APPROVED, WEB, Touches),
            edge(KEPT, ROOT, Touches),
            edge(TRIAL, PROPOSED, SharesFiles),
            edge(PROPOSED, KNOWN, FromSignature),
        ],
        lessons,
        omitted: 0,
        module_source: ModuleSource::CargoCache,
    }
}
