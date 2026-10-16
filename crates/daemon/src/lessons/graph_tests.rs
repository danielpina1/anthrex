//! Task M9.15.13: decisions 41 to 43, the knowledge graph's nodes and edges, as table
//! tests over constructed stores and consolidated lessons (no I/O, no clock).

use std::collections::BTreeMap;

use proto::{
    DistillCause, GRAPH_NODES_MAX, GraphEdgeKind, GraphNodeKind, LessonLayer, LessonOrigin,
    LessonStatus, ModuleSource,
};

use super::*;
use crate::lessons::fold::{Lesson, Queued, Store};

const NOW: u64 = 1_800_000_000;

/// A lesson's files, the modules, and the hubs it touches in file order.
type TouchCase<'a> = (&'a [&'a str], &'a [(String, Option<String>)], &'a [&'a str]);
/// Expected `(from, to)` edges.
type Expected<'a> = &'a [(&'a str, &'a str)];

/// A distilled lesson `id` (`l-…`) or a known failure (`kf-…`) with `files`.
fn lesson(id: &str, status: LessonStatus, files: &[&str]) -> Lesson {
    let known = id.starts_with("kf-");
    Lesson {
        id: id.to_string(),
        layer: LessonLayer::Episodic,
        status,
        origin: if known {
            LessonOrigin::Signature
        } else {
            LessonOrigin::Distilled
        },
        title: format!("title of {id}"),
        body: format!("body of {id}"),
        files: files.iter().map(|f| f.to_string()).collect(),
        created_at: NOW,
        last_seen: NOW,
        trial_start: None,
        runs: Vec::new(),
        reconfirmed: 0,
        retired_reason: None,
        queued: None,
        green: 0,
        outcomes: 0,
        headlines: Vec::new(),
    }
}

fn modules(pairs: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
    (pairs.iter())
        .map(|(name, dir)| (name.to_string(), dir.map(str::to_string)))
        .collect()
}

fn tui_and_daemon() -> Vec<(String, Option<String>)> {
    modules(&[
        ("tui", Some("crates/tui")),
        ("daemon", Some("crates/daemon")),
    ])
}

fn graph_of(lessons: &[Lesson], mods: &[(String, Option<String>)]) -> LessonGraph {
    build(&Store::default(), lessons, mods, ModuleSource::CargoCache)
}

fn edges(graph: &LessonGraph, kind: GraphEdgeKind) -> Vec<(String, String)> {
    (graph.edges.iter())
        .filter(|e| e.kind == kind)
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    (list.iter())
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

fn lesson_ids(graph: &LessonGraph) -> Vec<String> {
    (graph.nodes.iter())
        .filter(|n| n.kind == GraphNodeKind::Lesson)
        .map(|n| n.id.clone())
        .collect()
}

#[test]
fn lesson_spanning_two_modules_links_to_both() {
    let nested = modules(&[
        ("daemon", Some("crates/daemon")),
        ("sub", Some("crates/daemon/sub")),
    ]);
    // (files, modules, the touched hubs in file order)
    let table: [TouchCase<'_>; 5] = [
        (
            &["crates/tui/src/a.rs", "crates/daemon/**"],
            &tui_and_daemon(),
            &["m:tui", "m:daemon"],
        ),
        (
            &["crates/tui/src/a.rs", "crates/tui/**"],
            &tui_and_daemon(),
            &["m:tui"],
        ),
        // The longest module directory wins.
        (&["crates/daemon/sub/x.rs"], &nested, &["m:sub"]),
        (&["crates/daemon/src/x.rs"], &nested, &["m:daemon"]),
        // A directory is matched by whole components, not by text.
        (&["crates/tuix/a.rs"], &tui_and_daemon(), &[ROOT_ID]),
    ];
    for (files, mods, want) in table {
        let graph = graph_of(&[lesson("l-a", LessonStatus::Kept, files)], mods);
        let got = edges(&graph, GraphEdgeKind::Touches);
        let want: Vec<(String, String)> =
            want.iter().map(|m| ("l-a".into(), m.to_string())).collect();
        assert_eq!(got, want, "files {files:?}");
    }
}

#[test]
fn no_files_lesson_links_to_root() {
    let graph = graph_of(&[lesson("l-a", LessonStatus::Kept, &[])], &tui_and_daemon());
    assert_eq!(
        edges(&graph, GraphEdgeKind::Touches),
        pairs(&[("l-a", ROOT_ID)])
    );
}

#[test]
fn files_in_no_module_link_to_root() {
    let table: [(&[&str], Expected<'_>); 4] = [
        (&["docs/a.md"], &[("l-a", ROOT_ID)]),
        (&["**/*.rs"], &[("l-a", ROOT_ID)]),
        (&["docs/a.md", "README.md"], &[("l-a", ROOT_ID)]),
        (
            &["docs/a.md", "crates/tui/a.rs"],
            &[("l-a", ROOT_ID), ("l-a", "m:tui")],
        ),
    ];
    for (files, want) in table {
        let graph = graph_of(
            &[lesson("l-a", LessonStatus::Kept, files)],
            &tui_and_daemon(),
        );
        assert_eq!(
            edges(&graph, GraphEdgeKind::Touches),
            pairs(want),
            "{files:?}"
        );
    }
    // A module whose directory is unknown has a hub and no files.
    let mods = modules(&[("tui", None)]);
    let graph = graph_of(
        &[lesson("l-a", LessonStatus::Kept, &["crates/tui/a.rs"])],
        &mods,
    );
    assert!(graph.nodes.iter().any(|n| n.id == "m:tui"));
    assert_eq!(
        edges(&graph, GraphEdgeKind::Touches),
        pairs(&[("l-a", ROOT_ID)])
    );
}

#[test]
fn known_failure_count() {
    let lessons = [
        lesson(
            "kf-00000000000000aa",
            LessonStatus::Known,
            &["crates/tui/**"],
        ),
        lesson("kf-00000000000000bb", LessonStatus::Known, &[]),
        lesson("l-a", LessonStatus::Kept, &[]),
    ];
    let graph = graph_of(&lessons, &tui_and_daemon());
    let known: Vec<&GraphNode> = (graph.nodes.iter())
        .filter(|n| n.status == Some(LessonStatus::Known))
        .collect();
    assert_eq!(known.len(), 2);
    assert!(known.iter().all(|n| n.kind == GraphNodeKind::Lesson));
    assert!(known.iter().all(|n| n.layer == Some(LessonLayer::Episodic)));
}

#[test]
fn dismissed_fp_absent() {
    let mut store = Store::default();
    store.dismissed.insert("00000000000000aa".to_string());
    let lessons = [
        lesson("kf-00000000000000aa", LessonStatus::Known, &[]),
        lesson("kf-00000000000000bb", LessonStatus::Known, &[]),
    ];
    let graph = build(&store, &lessons, &[], ModuleSource::None);
    assert_eq!(lesson_ids(&graph), ["kf-00000000000000bb"]);
    assert_eq!(graph.lessons.len(), 1);
    assert!(graph.edges.iter().all(|e| e.from != "kf-00000000000000aa"));
}

#[test]
fn retired_and_rejected_absent() {
    let lessons = [
        lesson("l-retired", LessonStatus::Retired, &["crates/tui/**"]),
        lesson("l-rejected", LessonStatus::Rejected, &["crates/tui/**"]),
        lesson("l-kept", LessonStatus::Kept, &["crates/tui/**"]),
    ];
    let graph = graph_of(&lessons, &tui_and_daemon());
    assert_eq!(lesson_ids(&graph), ["l-kept"]);
    let infos: Vec<&str> = graph.lessons.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(infos, ["l-kept"]);
    assert_eq!(
        graph.omitted, 0,
        "a gone lesson is not omitted, it is absent"
    );
    assert!(edges(&graph, GraphEdgeKind::SharesFiles).is_empty());
}

#[test]
fn proposed_present() {
    let table = [
        LessonStatus::Proposed,
        LessonStatus::OnTrial,
        LessonStatus::Kept,
        LessonStatus::Approved,
        LessonStatus::Known,
    ];
    for status in table {
        let id = if status == LessonStatus::Known {
            "kf-00000000000000aa"
        } else {
            "l-a"
        };
        let graph = graph_of(&[lesson(id, status, &[])], &[]);
        let node = graph.nodes.iter().find(|n| n.id == id);
        assert_eq!(node.and_then(|n| n.status), Some(status), "{status:?}");
        assert_eq!(graph.lessons[0].status, status);
    }
}

#[test]
fn shares_files_edges_once_with_from_lt_to() {
    let lessons = [
        lesson("l-b", LessonStatus::Kept, &["crates/a/**"]),
        lesson(
            "l-a",
            LessonStatus::Kept,
            &["crates/a/x.rs", "crates/a/y.rs"],
        ),
        lesson("l-c", LessonStatus::Kept, &["crates/z.rs"]),
        lesson("l-d", LessonStatus::Kept, &[]),
        lesson("kf-00000000000000aa", LessonStatus::Known, &["crates/z.rs"]),
    ];
    let graph = graph_of(&lessons, &[]);
    assert_eq!(
        edges(&graph, GraphEdgeKind::SharesFiles),
        pairs(&[("kf-00000000000000aa", "l-c"), ("l-a", "l-b")])
    );
}

#[test]
fn from_signature_edge() {
    let mut store = Store::default();
    store.queued.insert(
        "run1-0001/t1/distill".to_string(),
        Queued {
            record_id: "run1-0001/t1/distill".to_string(),
            at: NOW,
            run_id: "run1-0001".to_string(),
            task_id: Some("t1".to_string()),
            cause: DistillCause::TaskRecovered,
            fps: vec![
                "00000000000000aa".to_string(),
                "00000000000000bb".to_string(),
                "00000000000000cc".to_string(),
            ],
            open: false,
            attempts: 0,
        },
    );
    store.dismissed.insert("00000000000000cc".to_string());
    let mut distilled = lesson("l-x", LessonStatus::OnTrial, &[]);
    distilled.queued = Some("run1-0001/t1/distill".to_string());
    let mut stray = lesson("l-y", LessonStatus::OnTrial, &[]);
    stray.queued = Some("run9-0009/t1/distill".to_string());
    let lessons = [
        distilled,
        stray,
        // `aa` is active; `bb` ended (consolidation returned no `kf-bb`); `cc` was dismissed.
        lesson("kf-00000000000000aa", LessonStatus::Known, &[]),
        lesson("kf-00000000000000cc", LessonStatus::Known, &[]),
    ];
    let graph = build(&store, &lessons, &[], ModuleSource::None);
    assert_eq!(
        edges(&graph, GraphEdgeKind::FromSignature),
        pairs(&[("l-x", "kf-00000000000000aa")])
    );
}

#[test]
fn empty_store_gives_module_hubs_only() {
    let mods = modules(&[("tui", Some("crates/tui")), ("daemon", None)]);
    for source in [ModuleSource::CargoCache, ModuleSource::Profile] {
        let graph = build(&Store::default(), &[], &mods, source);
        let nodes: Vec<(&str, GraphNodeKind, &str)> = (graph.nodes.iter())
            .map(|n| (n.id.as_str(), n.kind, n.label.as_str()))
            .collect();
        assert_eq!(
            nodes,
            [
                (ROOT_ID, GraphNodeKind::Root, "(root)"),
                ("m:daemon", GraphNodeKind::Module, "daemon"),
                ("m:tui", GraphNodeKind::Module, "tui"),
            ]
        );
        assert!(graph.edges.is_empty());
        assert!(graph.lessons.is_empty());
        assert_eq!((graph.omitted, graph.module_source), (0, source));
    }
    let graph = build(&Store::default(), &[], &[], ModuleSource::None);
    let ids: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, [ROOT_ID], "the root hub is always present");
    assert_eq!(graph.module_source, ModuleSource::None);
}

#[test]
fn cap_keeps_hubs_and_counts_omitted() {
    let mods: Vec<(String, Option<String>)> = (0..5)
        .map(|i| (format!("mod{i}"), Some(format!("crates/mod{i}"))))
        .collect();
    let hubs = mods.len() + 1;
    let mut lessons: Vec<Lesson> = (0..448)
        .map(|i| {
            let mut l = lesson(&format!("l-{i:04}"), LessonStatus::OnTrial, &[]);
            l.last_seen = NOW - u64::from(i as u32);
            l
        })
        .collect();
    // Known failures last in the slice still rank first (decision 21's freeze order).
    lessons.push(lesson("kf-00000000000000aa", LessonStatus::Known, &[]));
    lessons.push(lesson("kf-00000000000000bb", LessonStatus::Known, &[]));
    let graph = graph_of(&lessons, &mods);
    assert_eq!(graph.nodes.len(), GRAPH_NODES_MAX);
    assert_eq!(graph.omitted as usize, 450 - (GRAPH_NODES_MAX - hubs));
    assert!(graph.nodes.iter().any(|n| n.id == ROOT_ID));
    for (name, _) in &mods {
        assert!(graph.nodes.iter().any(|n| n.id == format!("m:{name}")));
    }
    let drawn = lesson_ids(&graph);
    assert_eq!(drawn.len(), graph.lessons.len());
    assert_eq!(&drawn[..2], ["kf-00000000000000aa", "kf-00000000000000bb"]);
    // Then the newest first: l-0000 is the newest on-trial lesson, l-0447 the oldest.
    assert_eq!(drawn[2], "l-0000");
    assert!(!drawn.contains(&"l-0447".to_string()));
    // Every edge names a drawn node.
    let ids: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(
        (graph.edges.iter())
            .all(|e| ids.contains(&e.from.as_str()) && ids.contains(&e.to.as_str()))
    );
}

#[test]
fn module_hubs_capped_by_name() {
    let mods: Vec<(String, Option<String>)> = (0..150)
        .rev()
        .map(|i| (format!("mod{i:03}"), Some(format!("crates/mod{i:03}"))))
        .collect();
    let graph = graph_of(
        &[lesson("l-a", LessonStatus::Kept, &["crates/mod149/a.rs"])],
        &mods,
    );
    let hubs: Vec<&str> = (graph.nodes.iter())
        .filter(|n| n.kind == GraphNodeKind::Module)
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(hubs.len(), MODULE_HUBS_MAX);
    assert_eq!((hubs[0], hubs[99]), ("m:mod000", "m:mod099"));
    // A module without a hub is no module: its files go to the root hub.
    assert_eq!(
        edges(&graph, GraphEdgeKind::Touches),
        pairs(&[("l-a", ROOT_ID)])
    );
    // A module named twice has one hub.
    let twice = modules(&[("tui", Some("crates/tui")), ("tui", Some("crates/other"))]);
    let graph = graph_of(&[], &twice);
    assert_eq!(graph.nodes.len(), 2);
}

#[test]
fn shares_files_compares_drawn_lessons_only() {
    // Task M9.15.7's pattern: count the work, never time it. 450 lessons, of which at
    // most 299 are drawn: the pairwise step sees the drawn ones only.
    let lessons: Vec<Lesson> = (0..450)
        .map(|i| lesson(&format!("l-{i:04}"), LessonStatus::Kept, &["crates/a/**"]))
        .collect();
    PAIRS.with(|c| c.set(0));
    let graph = graph_of(&lessons, &[]);
    let drawn = (GRAPH_NODES_MAX - 1) as u64;
    assert_eq!(PAIRS.with(|c| c.get()), drawn * (drawn - 1) / 2);
    // The 44,551 intersecting pairs are capped (M9.15.13's edge-cap ruling).
    assert!(edges(&graph, GraphEdgeKind::SharesFiles).len() <= SHARES_MAX);
}

/// Each lesson's `shares_files` degree in `graph`.
fn share_degrees(graph: &LessonGraph) -> BTreeMap<String, usize> {
    let mut degrees = BTreeMap::new();
    for (from, to) in edges(graph, GraphEdgeKind::SharesFiles) {
        *degrees.entry(from).or_insert(0) += 1;
        *degrees.entry(to).or_insert(0) += 1;
    }
    degrees
}

#[test]
fn shares_files_worst_case_is_capped() {
    // The worst case: every drawn lesson (299, one hub) shares one broad glob, so all
    // 44,551 pairs intersect.
    let lessons: Vec<Lesson> = (0..GRAPH_NODES_MAX as u32 - 1)
        .map(|i| lesson(&format!("l-{i:04}"), LessonStatus::Kept, &["crates/a/**"]))
        .collect();
    let graph = graph_of(&lessons, &[]);
    let shares = edges(&graph, GraphEdgeKind::SharesFiles);
    assert!(shares.len() <= SHARES_MAX, "{} edges", shares.len());
    let degrees = share_degrees(&graph);
    assert!(
        degrees.values().all(|d| *d <= SHARES_PER_LESSON),
        "{degrees:?}"
    );
    // Equal strengths tie by id: the first lesson keeps its eight lowest-id partners.
    let first: Vec<&str> = (shares.iter())
        .filter(|(from, _)| from == "l-0000")
        .map(|(_, to)| to.as_str())
        .collect();
    assert_eq!(
        first,
        [
            "l-0001", "l-0002", "l-0003", "l-0004", "l-0005", "l-0006", "l-0007", "l-0008"
        ]
    );
    // Every lesson that shares files keeps at least one edge.
    assert_eq!(degrees.len(), lessons.len());
    // The rest of the graph is untouched: one `touches` edge per lesson.
    assert_eq!(edges(&graph, GraphEdgeKind::Touches).len(), lessons.len());
}

#[test]
fn shares_files_cap_keeps_the_strongest() {
    // `l-a` shares three files with `l-z` and one glob with each of nine `l-c<k>`:
    // ten candidates, so it keeps `l-z` (the strongest, though its id sorts last) and
    // the seven lowest-id `l-c<k>`.
    let mut lessons = vec![
        lesson(
            "l-a",
            LessonStatus::Kept,
            &[
                "crates/x/1.rs",
                "crates/x/2.rs",
                "crates/x/3.rs",
                "crates/y/**",
            ],
        ),
        lesson(
            "l-z",
            LessonStatus::Kept,
            &["crates/x/1.rs", "crates/x/2.rs", "crates/x/3.rs"],
        ),
    ];
    for k in 1..=9 {
        lessons.push(lesson(
            &format!("l-c{k}"),
            LessonStatus::Kept,
            &["crates/y/**"],
        ));
    }
    let graph = graph_of(&lessons, &[]);
    let of_a: Vec<String> = (edges(&graph, GraphEdgeKind::SharesFiles).into_iter())
        .filter(|(from, to)| from == "l-a" || to == "l-a")
        .map(|(from, to)| if from == "l-a" { to } else { from })
        .collect();
    let mut want: Vec<String> = (1..=7).map(|k| format!("l-c{k}")).collect();
    want.push("l-z".to_string());
    let mut got = of_a.clone();
    got.sort();
    assert_eq!(got, want, "{of_a:?}");
    assert!(
        share_degrees(&graph)
            .values()
            .all(|d| *d <= SHARES_PER_LESSON)
    );
}

#[test]
fn shares_files_cap_is_deterministic() {
    let mut lessons: Vec<Lesson> = (0..120)
        .map(|i| {
            let glob = format!("crates/m{}/**", i % 3);
            lesson(
                &format!("l-{i:04}"),
                LessonStatus::Kept,
                &[glob.as_str(), "crates/s/**"],
            )
        })
        .collect();
    let once = graph_of(&lessons, &[]);
    assert_eq!(graph_of(&lessons, &[]).edges, once.edges);
    lessons.reverse();
    lessons.rotate_left(37);
    assert_eq!(graph_of(&lessons, &[]).edges, once.edges);
    assert!(
        share_degrees(&once)
            .values()
            .all(|d| *d <= SHARES_PER_LESSON)
    );
}

#[test]
fn shares_files_small_graph_unchanged() {
    // Nine lessons on one glob: 36 pairs, each lesson at exactly the per-lesson cap.
    // Nothing is dropped and the edges keep their pairwise order.
    let lessons: Vec<Lesson> = (0..9)
        .map(|i| lesson(&format!("l-{i}"), LessonStatus::Kept, &["crates/a/**"]))
        .collect();
    let graph = graph_of(&lessons, &[]);
    let mut want = Vec::new();
    for i in 0..9 {
        for j in i + 1..9 {
            want.push((format!("l-{i}"), format!("l-{j}")));
        }
    }
    assert_eq!(edges(&graph, GraphEdgeKind::SharesFiles), want);
}

#[test]
fn shares_total_cap_binds_in_strength_then_id_order() {
    // At 300 nodes the per-lesson cap already bounds the total (299 × 8 / 2 = 1,196 <
    // 2,000), so the total cap is a backstop; pin it on the selection itself.
    let shares = [
        Share {
            from: "a",
            to: "b",
            strength: 1,
        },
        Share {
            from: "c",
            to: "d",
            strength: 2,
        },
        Share {
            from: "a",
            to: "c",
            strength: 1,
        },
        Share {
            from: "b",
            to: "d",
            strength: 1,
        },
    ];
    assert_eq!(strongest(&shares, 8, 2), [true, true, false, false]);
    assert_eq!(strongest(&shares, 1, 10), [true, true, false, false]);
    assert_eq!(strongest(&shares, 8, 10), [true; 4]);
}

#[test]
fn labels_are_plain_text() {
    let mut hostile = lesson("l-a", LessonStatus::Kept, &[]);
    hostile.title = "line one\nline two \u{1b}[31mred\u{202e}".to_string();
    let mods = modules(&[("we\u{1b}ird\nname", Some("crates/x"))]);
    let graph = graph_of(&[hostile], &mods);
    for node in &graph.nodes {
        assert!(
            !node
                .label
                .chars()
                .any(|c| c.is_control() || c == '\u{202e}'),
            "{:?}",
            node.label
        );
    }
    let by_id: BTreeMap<&str, &str> = (graph.nodes.iter())
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    assert_eq!(by_id["l-a"], "line one line two  [31mred");
}
