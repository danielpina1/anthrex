//! The followups' "a timed-out `append_once` loses a paid distill result": a distilled
//! answer the store cannot take is kept in `<repo_dir>/lessons.pending.json` and
//! appended later, once, under the store's lock, with bounded retries. The store is
//! made to fail by swapping it for a FIFO during the decider call (`append_once`'s
//! read refuses anything but a regular file, before any read), and given back by
//! swapping the regular file in again.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use proto::{DeciderMode, LessonLine};

use super::super::{DecideFn, ServiceTiming};
use super::{Rig, answered, attempt, distilled, is_distilled, lines, passes, queued, until, write};
use crate::lessons::LESSONS_FILE;

/// The pending file, as the worker writes it.
fn pending_path(repo: &Path) -> PathBuf {
    repo.join("lessons.pending.json")
}

/// The pending file's records, raw; none without a file.
fn pending(repo: &Path) -> Vec<serde_json::Value> {
    let Ok(text) = std::fs::read_to_string(pending_path(repo)) else {
        return Vec::new();
    };
    let file: serde_json::Value = serde_json::from_str(&text).unwrap();
    file["records"].as_array().cloned().unwrap_or_default()
}

/// A pending file holding one record of `lines` for run `id`'s entry.
fn write_pending(repo: &Path, id: &str, lines: &[LessonLine], tries: u32) {
    let record = serde_json::json!({
        "queued": format!("{id}/t1/distill"),
        "lines": lines,
        "tries": tries,
    });
    let file = serde_json::json!({ "v": 1, "records": [record] });
    std::fs::write(pending_path(repo), file.to_string()).unwrap();
}

fn mkfifo(path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let fifo = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: a plain mkfifo on a path the test owns.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
}

/// The store moved aside and a FIFO in its place.
fn stall_store(repo: &Path) {
    let store = repo.join(LESSONS_FILE);
    std::fs::rename(&store, repo.join("held.jsonl")).unwrap();
    mkfifo(&store);
}

/// The FIFO removed and the store back.
fn restore_store(repo: &Path) {
    let store = repo.join(LESSONS_FILE);
    std::fs::remove_file(&store).unwrap();
    std::fs::rename(repo.join("held.jsonl"), &store).unwrap();
}

/// A call that answers and, the first time, stalls `repo`'s store before it does; each
/// call counted in `calls`.
fn stalling(repo: &Path, calls: Arc<AtomicUsize>) -> DecideFn {
    let repo = repo.to_path_buf();
    Arc::new(move |_request, _project| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            stall_store(&repo);
        }
        Box::pin(async move { (None, answered()) })
    })
}

/// Short write retries, so no test waits for the default backoff.
fn fast(timing: &mut ServiceTiming) {
    timing.write_backoff = Duration::from_millis(10);
}

fn count(repo: &Path, what: impl Fn(&LessonLine) -> bool) -> usize {
    lines(repo).iter().filter(|l| what(l)).count()
}

#[tokio::test]
async fn a_stalled_store_keeps_the_distilled_result_and_appends_it_once() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    write(&repo, &[queued("r", super::T)]);
    let decide = stalling(&repo, rig.calls.clone());
    // Review m3: no give-up within the test's window, however slow its restore.
    let patient = |t: &mut ServiceTiming| {
        fast(t);
        t.write_tries = 1000;
    };
    let service = rig.start_seamed(20, (DeciderMode::Claude, None), decide, patient);
    until("the pending result on disk", || pending(&repo).len() == 1).await;
    let record = &pending(&repo)[0];
    assert_eq!(record["queued"], "r/t1/distill");
    let kinds: Vec<&str> = (record["lines"].as_array().unwrap().iter())
        .map(|l| l["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["attempt", "distilled"], "{record}");

    restore_store(&repo);
    until("the distilled line", || is_distilled(&repo, "r")).await;
    until("the pending file removed", || !pending_path(&repo).exists()).await;
    let before = service.passes();
    service.wake(repo.clone());
    passes(&service, before + 1).await;
    assert_eq!(rig.calls(), 1, "the paid result was asked for again");
    assert_eq!(count(&repo, |l| matches!(l, LessonLine::Distilled(_))), 1);
    assert_eq!(count(&repo, |l| matches!(l, LessonLine::Attempt(_))), 1);
    service.stop();
}

#[tokio::test]
async fn a_restart_appends_the_pending_result_once() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    write(&repo, &[queued("r", super::T)]);
    let decide = stalling(&repo, rig.calls.clone());
    // The first daemon never retries: an hour's backoff.
    let slow = |t: &mut ServiceTiming| t.write_backoff = Duration::from_secs(3600);
    let first = rig.start_seamed(20, (DeciderMode::Claude, None), decide, slow);
    let tried = || (pending(&repo).first()).is_some_and(|r| r["tries"].as_u64() >= Some(1));
    until("the first failed write on disk", tried).await;
    // Its first pass ends at that failure: the worker now waits out the hour.
    passes(&first, 1).await;
    first.stop();
    restore_store(&repo);

    let second = rig.start(20, DeciderMode::Claude);
    until("the distilled line", || is_distilled(&repo, "r")).await;
    until("the pending file removed", || !pending_path(&repo).exists()).await;
    let before = second.passes();
    second.wake(repo.clone());
    passes(&second, before + 1).await;
    assert_eq!(rig.calls(), 1, "the paid result was asked for again");
    assert_eq!(count(&repo, |l| matches!(l, LessonLine::Distilled(_))), 1);
    second.stop();
}

#[tokio::test]
async fn the_retry_cap_closes_the_entry_with_its_reason() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    write(&repo, &[queued("r", super::T)]);
    let decide = stalling(&repo, rig.calls.clone());
    let service = rig.start_seamed(20, (DeciderMode::Claude, None), decide, fast);
    let closed = || (pending(&repo).first()).is_some_and(|r| r["gave_up"] == true);
    until("the pending record given up", closed).await;
    let record = &pending(&repo)[0];
    assert_eq!(
        record["tries"].as_u64(),
        Some(super::super::DISTILL_WRITE_TRIES.into())
    );
    let kinds: Vec<&str> = (record["lines"].as_array().unwrap().iter())
        .map(|l| l["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["attempt", "gave_up"], "{record}");

    restore_store(&repo);
    let reason = || {
        lines(&repo).into_iter().find_map(|l| match l {
            LessonLine::GaveUp(g) => Some(g.reason),
            _ => None,
        })
    };
    until("the gave_up line", || reason().is_some()).await;
    let reason = reason().unwrap();
    assert!(
        reason
            .starts_with("the distilled result could not be written to the store after 5 tries: "),
        "{reason}"
    );
    assert!(reason.contains("is not a regular file"), "{reason}");
    until("the pending file removed", || !pending_path(&repo).exists()).await;
    let before = service.passes();
    service.wake(repo.clone());
    passes(&service, before + 1).await;
    assert_eq!(rig.calls(), 1);
    assert_eq!(count(&repo, |l| matches!(l, LessonLine::Distilled(_))), 0);
    service.stop();
}

/// A crash after the append but before the pending record was removed: the re-append
/// is a no-op (`append_once`'s `record_id` check) and the record goes.
#[tokio::test]
async fn a_pending_result_already_in_the_store_is_not_appended_again() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    let held = [attempt("r", 1, super::T, None), distilled("r", super::T)];
    write(&repo, &[queued("r", super::T)]);
    write(&repo, &held);
    write_pending(&repo, "r", &held, 1);
    let before = std::fs::read(repo.join(LESSONS_FILE)).unwrap();
    let service = rig.start(20, DeciderMode::Claude);
    until("the pending file removed", || !pending_path(&repo).exists()).await;
    passes(&service, 1).await;
    assert_eq!(std::fs::read(repo.join(LESSONS_FILE)).unwrap(), before);
    assert_eq!(rig.calls(), 0);
    service.stop();
}

/// Review m1: a stale snapshot of the pending file (a save that outlived its bound and
/// renamed late) holds `distilled` for an entry the store already closed `gave_up`:
/// the record is dropped, and the entry is never closed twice.
#[tokio::test]
async fn a_pending_result_of_an_entry_the_store_closed_is_dropped() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    let closed = LessonLine::GaveUp(proto::GaveUpLine {
        v: proto::LESSONS_VERSION,
        record_id: "r/t1/distill/gave_up".into(),
        at: super::T,
        queued: "r/t1/distill".into(),
        reason: "the distilled result could not be written to the store after 5 tries: x".into(),
    });
    let tried = attempt("r", 1, super::T, None);
    write(&repo, &[queued("r", super::T), tried.clone(), closed]);
    write_pending(&repo, "r", &[tried, distilled("r", super::T)], 3);
    let before = std::fs::read_to_string(repo.join(LESSONS_FILE)).unwrap();
    let service = rig.start(20, DeciderMode::Claude);
    until("the pending file removed", || !pending_path(&repo).exists()).await;
    passes(&service, 1).await;
    assert_eq!(
        std::fs::read_to_string(repo.join(LESSONS_FILE)).unwrap(),
        before
    );
    assert_eq!(rig.calls(), 0);
    service.stop();
}
