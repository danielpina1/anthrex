//! Task M9.15.5: decision 19's requests over real temporary data directories.

use std::path::Path;

use proto::{
    DistillCause, DistilledLine, LESSONS_VERSION, LessonDraft, LessonLayer, LessonLine,
    LessonStatus, LessonsRequest, QueuedLine, RunReply, VerdictLine,
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

#[tokio::test]
async fn graph_is_refused_until_m9_15_13() {
    let dir = tempfile::tempdir().unwrap();
    let at = LessonsAt {
        git: "git".into(),
        data_dir: dir.path().to_path_buf(),
        git_timeout: Duration::from_secs(5),
        cfg: cfg(),
    };
    let req = LessonsRequest::Graph {
        dir: dir.path().to_path_buf(),
    };
    let reply = answer(&LessonLocks::default(), at, req, NOW).await;
    assert_eq!(reply, RunReply::refused(request::LESSONS, GRAPH_NOT_YET));
}
