//! Lessons requests never block for good: a store or history that is not a regular
//! file is refused before any read, every request's blocking work is awaited under a
//! bound, and `Decide`/`Forget` never hold the repository's `LessonLocks` entry across
//! a read that has not ended (the fix for fifo-report.md's product finding).
//!
//! No test here waits without a deadline: each request runs on a thread of its own,
//! with a runtime of its own, and is awaited with `recv_timeout`; the thread is joined
//! only once it has sent. A stuck read is a closure blocked on a gate the test drops on
//! every exit, so no blocking thread outlives the test. Every `TempDir` is held by the
//! test thread, so it drops on every exit, a failed wait's panic included.

use std::sync::mpsc;

use super::*;
use crate::lessons::store::{append_once, append_once_reading};

/// The bound a stuck read is given here (docs/timing-budgets.md).
const STUCK_BOUND: Duration = Duration::from_millis(200);
/// The hang guard over one request: a correct build answers a refusal in milliseconds,
/// and a stuck read within `STUCK_BOUND`. Never reached by a correct build.
const GUARD: Duration = Duration::from_secs(20);

/// `work` run on a thread of its own, on a runtime of its own, awaited for at most
/// `GUARD`. A wait that times out panics and leaves the thread behind (it is never
/// joined), so a request that never answers fails the test instead of hanging it.
fn replied<T, F>(work: impl FnOnce() -> F + Send + 'static) -> T
where
    T: Send + 'static,
    F: std::future::Future<Output = T>,
{
    let (tx, rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let _ = tx.send(runtime.block_on(work()));
    });
    let got = (rx.recv_timeout(GUARD)).unwrap_or_else(|_| panic!("no reply within {GUARD:?}"));
    // It has sent, so the join cannot wait.
    thread.join().unwrap();
    got
}

/// A read that blocks until the returned sender drops (or a minute passes), then fails.
/// The sender is dropped inside the request's runtime, before it ends: a runtime's
/// drop waits for its blocking threads, this read's among them.
fn stuck<T>() -> (
    mpsc::Sender<()>,
    impl FnOnce() -> Result<T, String> + Send + 'static,
) {
    let (release, gate) = mpsc::channel::<()>();
    let read = move || {
        let _ = gate.recv_timeout(Duration::from_secs(60));
        Err("released".to_string())
    };
    (release, read)
}

fn mkfifo(path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let fifo = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: a plain mkfifo on a path the test owns.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
}

fn at(tmp: &Path) -> LessonsAt {
    LessonsAt {
        git: "git".into(),
        data_dir: tmp.join("data"),
        git_timeout: T,
        cfg: cfg(),
    }
}

/// The four requests on `checkout`.
fn requests(checkout: &Path) -> [LessonsRequest; 4] {
    let dir = checkout.to_path_buf();
    let ids = vec![format!("kf-{FP}")];
    [
        LessonsRequest::List {
            dir: dir.clone(),
            all: true,
        },
        LessonsRequest::Graph { dir: dir.clone() },
        LessonsRequest::Decide {
            dir: dir.clone(),
            ids: ids.clone(),
            approve: true,
        },
        LessonsRequest::Forget { dir, ids },
    ]
}

/// `request(n)` (an index into [`requests`]) answered with a FIFO at `file` in the
/// repository's data directory: refused, naming the FIFO, well within the bound.
fn refuses_a_fifo_at(file: &str, n: usize) {
    let (tmp, checkout, _project, repo_dir) = git_repo();
    let fifo = repo_dir.join(file);
    mkfifo(&fifo);
    let (at, req) = (at(tmp.path()), requests(&checkout)[n].clone());
    let started = std::time::Instant::now();
    let reply = replied(move || async move { answer(&LessonLocks::default(), at, req, NOW).await });
    let message = format!("{} is not a regular file", fifo.display());
    assert_eq!(reply, RunReply::refused(request::LESSONS, message));
    assert!(started.elapsed() < GUARD, "{:?}", started.elapsed());
}

#[test]
fn list_refuses_a_fifo_store_and_history() {
    refuses_a_fifo_at(LESSONS_FILE, 0);
    refuses_a_fifo_at(HISTORY_FILE, 0);
}

#[test]
fn graph_refuses_a_fifo_store_and_history() {
    refuses_a_fifo_at(LESSONS_FILE, 1);
    refuses_a_fifo_at(HISTORY_FILE, 1);
}

#[test]
fn decide_refuses_a_fifo_store_and_history() {
    refuses_a_fifo_at(LESSONS_FILE, 2);
    refuses_a_fifo_at(HISTORY_FILE, 2);
}

#[test]
fn forget_refuses_a_fifo_store_and_history() {
    refuses_a_fifo_at(LESSONS_FILE, 3);
    refuses_a_fifo_at(HISTORY_FILE, 3);
}

#[test]
fn a_stuck_list_or_graph_read_times_out() {
    let (release, read) = stuck::<()>();
    let started = std::time::Instant::now();
    let got = replied(move || async move {
        let got = bounded(STUCK_BOUND, read).await;
        // Released before the runtime drops, which waits for its blocking threads.
        drop(release);
        got
    });
    let took = started.elapsed();
    assert_eq!(
        got,
        Err("the lessons store could not be read within 0.2 s".to_string())
    );
    assert!(took >= STUCK_BOUND, "{took:?}");
}

/// A distilled line for `append_once`.
fn retired_line() -> LessonLine {
    LessonLine::Retired(proto::RetiredLine {
        v: LESSONS_VERSION,
        record_id: format!("{}/retired", id(GAMMA)),
        at: NOW,
        id: id(GAMMA),
        reason: "test".into(),
    })
}

/// Ruling 3 and 4: a `Decide` or a `Forget` whose read is stuck refuses at its bound
/// and releases the lock, so a later `Decide` and the distill worker's `append_once`
/// on the same repository go through.
#[test]
fn a_stuck_decide_or_forget_releases_the_lock() {
    for forget in [false, true] {
        let dir = repo();
        let repo_dir = dir.path().to_path_buf();
        let (release, read) = stuck();
        let (decided, later, appended) = replied(move || async move {
            let locks = LessonLocks::default();
            let stuck = if forget {
                forget_within(&locks, &repo_dir, vec![id(BETA)], NOW, STUCK_BOUND, read).await
            } else {
                let ids = vec![id(BETA)];
                decide_within(&locks, &repo_dir, ids, true, NOW, STUCK_BOUND, read).await
            };
            let later = decide(&locks, &repo_dir, vec![id(BETA)], true, NOW, cfg()).await;
            let appended = append_once(&locks, &repo_dir, retired_line()).await;
            drop(release);
            (stuck, later, appended.map_err(|e| e.to_string()))
        });
        assert_eq!(
            decided,
            Err("the lessons store could not be read within 0.2 s; nothing was written".into())
        );
        assert_eq!(later, Ok("approved 1".to_string()));
        assert_eq!(appended, Ok(true));
        assert_eq!(
            status_of(dir.path(), &id(BETA)),
            Some(LessonStatus::Approved)
        );
    }
}

/// A `Decide` that waits for the lock past its bound refuses, writing nothing.
#[test]
fn a_decide_that_cannot_take_the_lock_gives_up() {
    let dir = repo();
    let repo_dir = dir.path().to_path_buf();
    let got = replied(move || async move {
        let locks = LessonLocks::default();
        let _held = locks.lock(&repo_dir).await;
        let read = {
            let dir = repo_dir.clone();
            move || lessons_at(&dir, NOW, &cfg())
        };
        decide_within(
            &locks,
            &repo_dir,
            vec![id(BETA)],
            true,
            NOW,
            STUCK_BOUND,
            read,
        )
        .await
    });
    assert_eq!(
        got,
        Err(
            "the lessons store is busy: another change held it for over 0.2 s; nothing was written"
                .into()
        )
    );
    assert_eq!(
        status_of(dir.path(), &id(BETA)),
        Some(LessonStatus::Proposed)
    );
}

/// The distill worker's `append_once` gives up on a stuck read at its bound and
/// releases the lock; a FIFO store is an error at once.
#[test]
fn append_once_never_reads_unbounded_under_the_lock() {
    let dir = repo();
    let repo_dir = dir.path().to_path_buf();
    let (release, gate) = mpsc::channel::<()>();
    let holds = move |_: &Path, _: &str| {
        let _ = gate.recv_timeout(Duration::from_secs(60));
        Ok(false)
    };
    let (stuck, later) = replied(move || async move {
        let locks = LessonLocks::default();
        let stuck = append_once_reading(&locks, &repo_dir, retired_line(), STUCK_BOUND, holds);
        let stuck = stuck.await.map_err(|e| e.kind());
        let later = append_once(&locks, &repo_dir, retired_line()).await;
        drop(release);
        (stuck, later.map_err(|e| e.to_string()))
    });
    assert_eq!(stuck, Err(std::io::ErrorKind::TimedOut));
    assert_eq!(later, Ok(true));

    let fifo_dir = tempfile::tempdir().unwrap();
    let fifo = fifo_dir.path().join(LESSONS_FILE);
    mkfifo(&fifo);
    let repo_dir = fifo_dir.path().to_path_buf();
    let got = replied(move || async move {
        let appended = append_once(&LessonLocks::default(), &repo_dir, retired_line()).await;
        appended.map_err(|e| e.to_string())
    });
    assert_eq!(
        got,
        Err(format!("{} is not a regular file", fifo.display()))
    );
}

/// Ruling 6's other half: a regular store answers every request as before.
#[test]
fn a_regular_store_answers_every_request() {
    let (tmp, checkout, _project, repo_dir) = git_repo();
    let seeded = repo();
    for file in [LESSONS_FILE, HISTORY_FILE] {
        std::fs::copy(seeded.path().join(file), repo_dir.join(file)).unwrap();
    }
    let at = at(tmp.path());
    let dir = checkout.clone();
    let replies = replied(move || async move {
        let locks = LessonLocks::default();
        let mut replies = Vec::new();
        let decide = LessonsRequest::Decide {
            dir: dir.clone(),
            ids: vec![id(BETA)],
            approve: true,
        };
        let forget = LessonsRequest::Forget {
            dir: dir.clone(),
            ids: vec![format!("kf-{FP}")],
        };
        let list = LessonsRequest::List { dir, all: false };
        for req in [decide, forget, list] {
            replies.push(answer(&locks, at.clone(), req, NOW).await);
        }
        replies
    });
    assert_eq!(replies[0], RunReply::done(request::LESSONS, "approved 1"));
    assert_eq!(replies[1], RunReply::done(request::LESSONS, "forgot 1"));
    let RunReply::Lessons { lessons, .. } = &replies[2] else {
        panic!("not a list: {:?}", replies[2]);
    };
    let listed: Vec<(&str, LessonStatus)> =
        lessons.iter().map(|l| (l.id.as_str(), l.status)).collect();
    let (alpha, beta) = (id(ALPHA), id(BETA));
    assert_eq!(
        listed,
        [
            (beta.as_str(), LessonStatus::Approved),
            (alpha.as_str(), LessonStatus::OnTrial)
        ]
    );
}
