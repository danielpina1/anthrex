//! Task M9.15.6: decision 21's freeze over real temporary data directories.

use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use proto::{
    DistillCause, DistilledLine, LESSONS_VERSION, LessonDraft, LessonLayer, LessonLine,
    LessonStatus, QueuedLine, VerdictLine,
};

use super::*;
use crate::lessons::fold::title_fp;
use crate::run::engine::HISTORY_FILE;
use crate::run::history_io::append_json;

const NOW: u64 = 1_800_000_000;
const DAY: u64 = 86_400;
const FP: &str = "0123456789abcdef";

fn cfg() -> config::Lessons {
    config::Lessons::default()
}

fn id(title: &str) -> String {
    format!("l-{}", title_fp(title))
}

/// One distill entry of run `n` whose answer holds `items` (episodic → on trial, else
/// proposed).
fn distil(dir: &Path, n: u32, items: &[(LessonLayer, String)]) {
    let queued = format!("run{n}-{n:04x}/t1/distill");
    let path = dir.join(crate::lessons::LESSONS_FILE);
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
                title: title.clone(),
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

fn verdict(dir: &Path, title: &str, approved: bool) {
    let line = LessonLine::Verdict(VerdictLine {
        v: LESSONS_VERSION,
        record_id: format!("{}/verdict/{}", id(title), NOW - DAY),
        at: NOW - DAY,
        id: id(title),
        approved,
    });
    append_json(&dir.join(crate::lessons::LESSONS_FILE), &line).unwrap();
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

fn titles(n: usize, prefix: &str) -> Vec<String> {
    (0..n).map(|i| format!("{prefix} lesson {i}")).collect()
}

#[tokio::test]
async fn lessons_for_start_freezes_active_only() {
    let dir = tempfile::tempdir().unwrap();
    let [trial, second, proposed, approved, rejected] = [
        "on trial",
        "also on trial",
        "proposed",
        "approved",
        "rejected",
    ]
    .map(String::from);
    distil(
        dir.path(),
        1,
        &[
            (LessonLayer::Episodic, trial.clone()),
            (LessonLayer::Episodic, second.clone()),
            (LessonLayer::Semantic, proposed.clone()),
            (LessonLayer::Semantic, approved.clone()),
            (LessonLayer::Procedural, rejected.clone()),
        ],
    );
    verdict(dir.path(), &approved, true);
    verdict(dir.path(), &rejected, false);
    let history = dir.path().join(HISTORY_FILE);
    append_json(&history, &signed_task(1, NOW - 2 * DAY)).unwrap();
    append_json(&history, &signed_task(2, NOW - DAY)).unwrap();

    let (lessons, line) = lessons_for_start(dir.path(), &cfg(), NOW).await;
    let got: Vec<(String, LessonStatus)> = (lessons.items.iter())
        .map(|l| (l.id.clone(), l.status))
        .collect();
    let mut on_trial = [id(&trial), id(&second)];
    on_trial.sort();
    let want = [
        (format!("kf-{FP}"), LessonStatus::Known),
        (id(&approved), LessonStatus::Approved),
        (on_trial[0].clone(), LessonStatus::OnTrial),
        (on_trial[1].clone(), LessonStatus::OnTrial),
    ];
    assert_eq!(got, want);
    assert_eq!(line, "lessons: 4 active (1 known failures)");
    assert_eq!(lessons.frozen_at, NOW);
    assert_eq!(lessons.limits, crate::run::model::LessonLimits::default());
    let kf = &lessons.items[0];
    assert_eq!(kf.files, ["crates/a/**"]);
    assert_eq!(kf.usefulness, None);
}

#[tokio::test]
async fn frozen_max_is_fifty() {
    let dir = tempfile::tempdir().unwrap();
    let items: Vec<(LessonLayer, String)> = (titles(60, "episodic").into_iter())
        .map(|t| (LessonLayer::Episodic, t))
        .collect();
    distil(dir.path(), 1, &items);
    let (lessons, line) = lessons_for_start(dir.path(), &cfg(), NOW).await;
    assert_eq!(lessons.items.len(), FROZEN_MAX);
    assert_eq!(FROZEN_MAX, 50);
    // Ranked as decision 21 says: equal status and `last_seen`, so by id.
    let ids: Vec<&str> = lessons.items.iter().map(|l| l.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    assert_eq!(line, "lessons: 60 active (0 known failures)");
}

#[tokio::test]
async fn an_empty_store_logs_none() {
    let dir = tempfile::tempdir().unwrap();
    let (lessons, line) = lessons_for_start(dir.path(), &cfg(), NOW).await;
    assert!(lessons.items.is_empty());
    assert_eq!(line, "lessons: none");
}

#[tokio::test]
async fn off_logs_off() {
    let dir = tempfile::tempdir().unwrap();
    distil(
        dir.path(),
        1,
        &[(LessonLayer::Episodic, "on trial".to_string())],
    );
    let off = config::Lessons {
        enabled: false,
        ..cfg()
    };
    let (lessons, line) = lessons_for_start(dir.path(), &off, NOW).await;
    assert_eq!(line, "lessons: off");
    assert!(lessons.items.is_empty());
    assert!(!lessons.limits.enabled);
    // `risk_note` is independent of `enabled` (decision 31), so it is still frozen.
    assert!(lessons.limits.risk_note);
}

/// Ruling C15: a store whose read cannot finish (a FIFO no writer opens) gives no
/// lessons at the bound; the writer thread then releases the blocked read so the
/// runtime can drop.
#[tokio::test(flavor = "multi_thread")]
async fn a_slow_store_starts_with_none() {
    const BOUND: Duration = Duration::from_millis(50);
    // `tuning_io.rs`'s guard: bound × 100.
    const GUARD: Duration = Duration::from_secs(5);
    const HOLD: Duration = Duration::from_secs(9);
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join(crate::lessons::LESSONS_FILE);
    let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let held = fifo.clone();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let writer = std::thread::spawn(move || {
        // Wait for the signal first, so the read stays blocked past the bound.
        let _ = released.recv_timeout(HOLD);
        let deadline = Instant::now() + Duration::from_secs(10);
        let file = loop {
            let opened = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&held);
            match opened {
                Ok(file) => break file,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => {
                    let mut rdwr = std::fs::OpenOptions::new();
                    rdwr.read(true).write(true);
                    break rdwr.open(&held).expect("a FIFO opens O_RDWR");
                }
            }
        };
        std::thread::sleep(Duration::from_millis(200));
        let _ = std::fs::remove_file(&held);
        drop(file);
    });
    let started = Instant::now();
    let cfg = cfg();
    let read = lessons_within(dir.path(), &cfg, NOW, BOUND);
    let (lessons, line) = tokio::time::timeout(GUARD, read)
        .await
        .expect("the read is bounded");
    let took = started.elapsed();
    let _ = release.send(());
    tokio::task::spawn_blocking(move || writer.join().unwrap())
        .await
        .unwrap();
    assert_eq!(line, LESSONS_SLOW);
    assert_eq!(
        line,
        "lessons: none (the lessons store could not be read in time)"
    );
    assert!(lessons.items.is_empty());
    assert!(lessons.limits.enabled);
    assert!(took >= BOUND, "{took:?}");
}
