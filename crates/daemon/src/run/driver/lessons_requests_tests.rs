//! Task M9.15.5: decision 19's requests over real temporary data directories.

use std::path::{Path, PathBuf};

use proto::{
    DistillCause, DistilledLine, LESSONS_VERSION, LessonDraft, LessonLayer, LessonLine,
    LessonStatus, LessonsRequest, ModuleSource, QueuedLine, RunReply, VerdictLine,
};

use super::*;
use crate::lessons::fold::title_fp;
use crate::run::history_io::append_json;

const NOW: u64 = 1_800_000_000;
const DAY: u64 = 86_400;
const FP: &str = "0123456789abcdef";
const ALPHA: &str = "the parser rejects trailing commas";
const BETA: &str = "run cargo fmt before claiming done";
const GAMMA: &str = "keep the lock order: manager, then engine";

fn cfg() -> config::Lessons {
    config::Lessons::default()
}

fn id(title: &str) -> String {
    format!("l-{}", title_fp(title))
}

fn store_path(dir: &Path) -> std::path::PathBuf {
    dir.join(LESSONS_FILE)
}

/// One distill entry whose answer holds `items` (episodic → on trial, else proposed).
fn distil(dir: &Path, n: u32, items: &[(LessonLayer, &str)]) {
    let queued = format!("run{n}-{n:04x}/t1/distill");
    let path = store_path(dir);
    let at = NOW - 3 * DAY + u64::from(n);
    let q = LessonLine::Queued(QueuedLine {
        v: LESSONS_VERSION,
        record_id: queued.clone(),
        at,
        run_id: format!("run{n}-{n:04x}"),
        task_id: Some("t1".into()),
        cause: DistillCause::TaskRecovered,
        dossier: "## outcome\nrecovered".into(),
        fps: Vec::new(),
    });
    let d = LessonLine::Distilled(DistilledLine {
        v: LESSONS_VERSION,
        record_id: format!("{queued}/distilled"),
        at,
        queued,
        items: (items.iter())
            .map(|(layer, title)| LessonDraft {
                layer: *layer,
                title: title.to_string(),
                body: format!("body of {title}"),
                files: vec!["crates/a/**".into()],
            })
            .collect(),
        dropped: 0,
        usage: None,
    });
    append_json(&path, &q).unwrap();
    append_json(&path, &d).unwrap();
}

/// The captured protocol-23 task record, re-keyed into run `n` at `at` and signed `FP`.
fn signed_task(n: u32, at: u64) -> serde_json::Value {
    let text = include_str!("../../../../proto/src/m914_history.jsonl");
    let line = (text.lines())
        .find(|l| l.contains("\"type\":\"task\""))
        .expect("the fixture has a task line");
    let mut task: serde_json::Value = serde_json::from_str(line).unwrap();
    let run = format!("run{n}-{n:04x}");
    task["record_id"] = format!("{run}/t1").into();
    task["run_id"] = run.into();
    task["at"] = at.into();
    task["owns"] = serde_json::json!(["crates/a/**"]);
    task["signatures"] = serde_json::json!([
        {"gate": "check", "fp": FP, "headline": "error: boom"}
    ]);
    task
}

/// Two runs a day apart hit `FP`: the known failure `kf-<FP>`.
fn known_failure(dir: &Path) {
    let path = dir.join(HISTORY_FILE);
    append_json(&path, &signed_task(1, NOW - 2 * DAY)).unwrap();
    append_json(&path, &signed_task(2, NOW - DAY)).unwrap();
}

/// Alpha on trial, Beta proposed, Gamma rejected, and the known failure.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    distil(
        dir.path(),
        1,
        &[
            (LessonLayer::Episodic, ALPHA),
            (LessonLayer::Semantic, BETA),
            (LessonLayer::Procedural, GAMMA),
        ],
    );
    let verdict = LessonLine::Verdict(VerdictLine {
        v: LESSONS_VERSION,
        record_id: format!("{}/verdict/{}", id(GAMMA), NOW - DAY),
        at: NOW - DAY,
        id: id(GAMMA),
        approved: false,
    });
    append_json(&store_path(dir.path()), &verdict).unwrap();
    known_failure(dir.path());
    dir
}

fn status_of(dir: &Path, wanted: &str) -> Option<LessonStatus> {
    (list_at(dir, true, NOW, &cfg()).into_iter())
        .find(|l| l.id == wanted)
        .map(|l| l.status)
}

fn store_bytes(dir: &Path) -> Vec<u8> {
    std::fs::read(store_path(dir)).unwrap()
}

#[test]
fn list_folds_store_and_history() {
    let dir = repo();
    let kf = format!("kf-{FP}");
    let ids = |all| {
        (list_at(dir.path(), all, NOW, &cfg()).into_iter())
            .map(|l| (l.id, l.status))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(false),
        [
            (kf.clone(), LessonStatus::Known),
            (id(ALPHA), LessonStatus::OnTrial),
            (id(BETA), LessonStatus::Proposed),
        ]
    );
    let all = ids(true);
    assert_eq!(all.len(), 4, "{all:?}");
    assert_eq!(all[3], (id(GAMMA), LessonStatus::Rejected));
    let empty = tempfile::tempdir().unwrap();
    assert!(list_at(empty.path(), true, NOW, &cfg()).is_empty());
}

#[test]
fn decide_unknown_id_refuses_everything() {
    let dir = repo();
    let before = store_bytes(dir.path());
    let ids = vec![id(BETA), "l-zz".to_string()];
    assert_eq!(
        decide_at(dir.path(), &ids, true, NOW, &cfg()),
        Err("unknown lesson l-zz".to_string())
    );
    assert_eq!(store_bytes(dir.path()), before, "nothing written");
    assert_eq!(
        status_of(dir.path(), &id(BETA)),
        Some(LessonStatus::Proposed)
    );
}

#[test]
fn decide_wrong_state_refuses_everything() {
    let dir = repo();
    let before = store_bytes(dir.path());
    for (other, status) in [(format!("kf-{FP}"), "known"), (id(GAMMA), "rejected")] {
        let ids = vec![id(ALPHA), other.clone()];
        assert_eq!(
            decide_at(dir.path(), &ids, false, NOW, &cfg()),
            Err(format!(
                "{other} is {status}; only a proposed or on-trial lesson can be approved or rejected"
            ))
        );
    }
    assert_eq!(store_bytes(dir.path()), before, "nothing written");
    // Both kinds a verdict applies to.
    let ids = vec![id(ALPHA), id(BETA), id(ALPHA)];
    assert_eq!(
        decide_at(dir.path(), &ids, true, NOW, &cfg()),
        Ok("approved 2".to_string())
    );
    assert_eq!(
        status_of(dir.path(), &id(ALPHA)),
        Some(LessonStatus::Approved)
    );
    assert_eq!(
        status_of(dir.path(), &id(BETA)),
        Some(LessonStatus::Approved)
    );
}

#[test]
fn forget_known_failure_writes_dismissed() {
    let dir = repo();
    let kf = format!("kf-{FP}");
    assert_eq!(
        forget_at(dir.path(), std::slice::from_ref(&kf), NOW, &cfg()),
        Ok("forgot 1".to_string())
    );
    let (lines, problems) = read_store(&store_path(dir.path()));
    assert!(problems.is_empty(), "{problems:?}");
    let dismissed = (lines.iter()).find_map(|l| match l {
        LessonLine::Dismissed(d) => Some(d),
        _ => None,
    });
    let dismissed = dismissed.expect("a dismissed line");
    assert_eq!(
        (
            dismissed.fp.as_str(),
            dismissed.record_id.as_str(),
            dismissed.at
        ),
        (FP, format!("kf-{FP}/dismissed").as_str(), NOW)
    );
    assert_eq!(status_of(dir.path(), &kf), None, "no longer listed");
    // Now unknown: an ended known failure is not listed, even with `all`.
    assert_eq!(
        forget_at(dir.path(), std::slice::from_ref(&kf), NOW, &cfg()),
        Err(format!("unknown lesson {kf}"))
    );
}

#[test]
fn forget_distilled_writes_retired() {
    let dir = repo();
    assert_eq!(
        forget_at(dir.path(), &[id(ALPHA), id(BETA)], NOW, &cfg()),
        Ok("forgot 2".to_string())
    );
    let lessons = list_at(dir.path(), true, NOW, &cfg());
    for title in [ALPHA, BETA] {
        let lesson = lessons.iter().find(|l| l.id == id(title)).unwrap();
        assert_eq!(lesson.status, LessonStatus::Retired);
        assert_eq!(lesson.retired_reason.as_deref(), Some(FORGOTTEN));
    }
    let before = store_bytes(dir.path());
    for (title, status) in [(ALPHA, "retired"), (GAMMA, "rejected")] {
        assert_eq!(
            forget_at(dir.path(), &[id(title)], NOW, &cfg()),
            Err(format!("{} is {status}; it is already gone", id(title)))
        );
    }
    assert_eq!(store_bytes(dir.path()), before, "nothing written");
}

/// Two titles whose ids share their first six characters (`l-` and four hex digits).
fn colliding_titles() -> (String, String) {
    let mut seen = std::collections::HashMap::new();
    for n in 0.. {
        let title = format!("lesson number {n}");
        if let Some(other) = seen.insert(id(&title)[..PREFIX_MIN].to_string(), title.clone()) {
            return (other, title);
        }
    }
    unreachable!()
}

#[test]
fn prefix_resolves_or_is_ambiguous() {
    let dir = repo();
    let (one, two) = colliding_titles();
    distil(
        dir.path(),
        2,
        &[(LessonLayer::Semantic, &one), (LessonLayer::Semantic, &two)],
    );
    let prefix = id(&one)[..PREFIX_MIN].to_string();
    assert_eq!(
        decide_at(dir.path(), std::slice::from_ref(&prefix), true, NOW, &cfg()),
        Err(format!("{prefix} names 2 lessons"))
    );
    // Too short to name anything, even when only one id starts with it.
    let short = id(BETA)[..PREFIX_MIN - 1].to_string();
    assert_eq!(
        decide_at(dir.path(), std::slice::from_ref(&short), true, NOW, &cfg()),
        Err(format!("unknown lesson {short}"))
    );
    // Six characters naming one lesson, and a known failure's `kf-` prefix.
    assert_eq!(
        decide_at(dir.path(), &[id(BETA)[..8].to_string()], true, NOW, &cfg()),
        Ok("approved 1".to_string())
    );
    assert_eq!(
        status_of(dir.path(), &id(BETA)),
        Some(LessonStatus::Approved)
    );
    assert_eq!(
        forget_at(dir.path(), &["kf-0123".to_string()], NOW, &cfg()),
        Ok("forgot 1".to_string())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decide_rederives_under_the_lock() {
    let dir = repo();
    let locks = LessonLocks::default();
    let ids = || vec![id(BETA)];
    let (a, b) = tokio::join!(
        decide(&locks, dir.path(), ids(), true, NOW, cfg()),
        decide(&locks, dir.path(), ids(), true, NOW, cfg()),
    );
    let mut results = [a, b];
    results.sort();
    assert_eq!(results[0], Ok("approved 1".to_string()));
    let refusal = results[1].clone().unwrap_err();
    assert!(refusal.contains(" is approved"), "{refusal}");
    let (lines, _) = read_store(&store_path(dir.path()));
    let verdicts = (lines.iter())
        .filter(|l| matches!(l, LessonLine::Verdict(v) if v.id == id(BETA)))
        .count();
    assert_eq!(verdicts, 1, "one verdict written");
}

/// A git repository at `<tmp>/repo` with `crates/a` and `crates/b`, and its data
/// directory under `<tmp>/data`: `(tmp, checkout, project, repo_dir)`.
fn git_repo() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let tmp = tempfile::Builder::new()
        .prefix("ax-lessons-graph")
        .tempdir_in("/tmp")
        .unwrap();
    let checkout = tmp.path().join("repo");
    for module in ["a", "b"] {
        std::fs::create_dir_all(checkout.join("crates").join(module)).unwrap();
    }
    // AGENTS.md rule 11's scrubbed environment.
    let init = std::process::Command::new("git")
        .args(["--no-optional-locks", "init", "-q"])
        .current_dir(&checkout)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_LITERAL_PATHSPECS")
        .env_remove("GIT_GLOB_PATHSPECS")
        .env_remove("GIT_NOGLOB_PATHSPECS")
        .env_remove("GIT_ICASE_PATHSPECS")
        .status()
        .unwrap();
    assert!(init.success());
    let (root, project) = checkout_of(std::ffi::OsStr::new("git"), &checkout, T).unwrap();
    let repo_dir = crate::profile::repo_dir(&tmp.path().join("data"), &project);
    std::fs::create_dir_all(&repo_dir).unwrap();
    (tmp, root, project, repo_dir)
}

const T: Duration = Duration::from_secs(30);

fn write_profile(repo_dir: &Path, modules: &[&str]) {
    let profile = proto::RepoProfile {
        modules: modules.iter().map(|m| m.to_string()).collect(),
        ..proto::RepoProfile::default()
    };
    let text = toml::to_string(&profile).unwrap();
    let path = repo_dir.join(crate::profile::store::PROFILE_FILE);
    crate::profile::store::write_atomic(&path, text.as_bytes()).unwrap();
}

fn module_graph(modules: &[(&str, Option<&str>)]) -> crate::run::tiers::ModuleGraph {
    let mut graph = crate::run::tiers::ModuleGraph::default();
    for (name, dir) in modules {
        graph.modules.insert(
            name.to_string(),
            crate::run::tiers::ModuleInfo {
                dir: dir.map(str::to_string),
                deps: Default::default(),
            },
        );
    }
    graph
}

#[test]
fn module_source_prefers_cache_then_profile_then_none() {
    let (_tmp, checkout, _, repo_dir) = git_repo();
    let owned = |pairs: &[(&str, Option<&str>)]| -> Vec<(String, Option<String>)> {
        (pairs.iter())
            .map(|(n, d)| (n.to_string(), d.map(str::to_string)))
            .collect()
    };
    // Nothing: no modules.
    assert_eq!(
        modules_at(&repo_dir, &checkout),
        (Vec::new(), ModuleSource::None)
    );
    // A profile with no modules is still none.
    write_profile(&repo_dir, &[]);
    assert_eq!(
        modules_at(&repo_dir, &checkout),
        (Vec::new(), ModuleSource::None)
    );
    // The profile's patterns, matched in the checkout (no `cargo` is run).
    write_profile(&repo_dir, &["crates/*"]);
    assert_eq!(
        modules_at(&repo_dir, &checkout),
        (
            owned(&[("a", Some("crates/a")), ("b", Some("crates/b"))]),
            ModuleSource::Profile
        )
    );
    // The cache wins, and of its two entries the newest (the last `cache_put`).
    let cache = repo_dir.join(super::super::graph::GRAPH_CACHE_FILE);
    super::super::graph::cache_put(
        &cache,
        "0000000000000001",
        &module_graph(&[("old", Some("old"))]),
    );
    super::super::graph::cache_put(
        &cache,
        "0000000000000002",
        &module_graph(&[("anthrex-tui", Some("crates/tui")), ("gen", None)]),
    );
    assert_eq!(
        modules_at(&repo_dir, &checkout),
        (
            owned(&[("anthrex-tui", Some("crates/tui")), ("gen", None)]),
            ModuleSource::CargoCache
        )
    );
    // A corrupt cache reads as no cache.
    std::fs::write(&cache, b"{not json").unwrap();
    assert_eq!(modules_at(&repo_dir, &checkout).1, ModuleSource::Profile);
}

#[tokio::test]
async fn graph_is_answered() {
    let (tmp, checkout, project, repo_dir) = git_repo();
    let seeded = repo();
    for file in [LESSONS_FILE, HISTORY_FILE] {
        std::fs::copy(seeded.path().join(file), repo_dir.join(file)).unwrap();
    }
    write_profile(&repo_dir, &["crates/*"]);
    let at = LessonsAt {
        git: "git".into(),
        data_dir: tmp.path().join("data"),
        git_timeout: T,
        cfg: cfg(),
    };
    let req = LessonsRequest::Graph {
        dir: checkout.join("crates"),
    };
    let reply = answer(&LessonLocks::default(), at.clone(), req, NOW).await;
    let RunReply::LessonsGraph { graph, request_id } = reply else {
        panic!("not a graph: {reply:?}");
    };
    assert_eq!(request_id, None);
    assert_eq!(graph.project, project);
    assert_eq!(graph.module_source, ModuleSource::Profile);
    let ids: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let (kf, alpha, beta) = (format!("kf-{FP}"), id(ALPHA), id(BETA));
    assert_eq!(
        ids,
        [
            "m:(root)",
            "m:a",
            "m:b",
            kf.as_str(),
            alpha.as_str(),
            beta.as_str()
        ],
        "the rejected lesson is not drawn"
    );
    let drawn: Vec<&str> = graph.lessons.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(drawn, [kf.as_str(), alpha.as_str(), beta.as_str()]);
    let touches = (graph.edges.iter())
        .filter(|e| e.kind == proto::GraphEdgeKind::Touches && e.to == "m:a")
        .count();
    assert_eq!(touches, 3, "every lesson owns crates/a/**");
    assert_eq!(graph.omitted, 0);

    // Outside a repository: refused as `run stats` refuses it.
    let outside = tmp.path().join("plain");
    std::fs::create_dir_all(&outside).unwrap();
    let req = LessonsRequest::Graph {
        dir: outside.clone(),
    };
    let reply = answer(&LessonLocks::default(), at, req, NOW).await;
    assert_eq!(
        reply,
        RunReply::refused(
            request::LESSONS,
            format!("not a git repository: {}", outside.display())
        )
    );
}
