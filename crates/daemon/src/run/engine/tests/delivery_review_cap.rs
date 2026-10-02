//! The final fix wave's B m-7: what `run.json` keeps of a stage's threads is bounded in
//! count as well as in text. A stage keeps at most [`THREADS_KEPT`] thread records: the
//! oldest resolved ones (replied or ignored) drop first, then the oldest others that no
//! task holds and no open batch lists. A thread keeps the ids of its newest
//! [`THREAD_IDS_KEPT`] comments. GitHub's view shows a PR's newest 100 threads, each
//! with its first 50 comments, so neither cut can make a view show a dropped record
//! again as new.

use crate::run::delivery::{Batch, SeenComment, ThreadRecord, ThreadState};
use crate::run::engine::delivery::review_limits::{
    COMMENT_KEPT_CHARS, STAGE_TEXT_CHARS, THREAD_IDS_KEPT, THREADS_KEPT,
};

use super::delivery_watch::{poll_with, view, watched};
use super::merge::commit;

const LOGIN: &str = "a-login-of-the-longest-length-github-ok"; // 39 characters

fn thread(k: u64, state: ThreadState, comments: u64) -> ThreadRecord {
    ThreadRecord {
        key: format!("t{k}"),
        author: LOGIN.into(),
        path: Some(format!("src/{}.rs", "p".repeat(200))),
        line: Some(1),
        diff_hunk: "@@ -1 +1 @@".repeat(500),
        text: "x".repeat(COMMENT_KEPT_CHARS * 2),
        state,
        seen_at: 1_000 + k,
        last_comment_id: k * 1_000 + comments,
        comments: (1..=comments)
            .map(|c| SeenComment {
                id: k * 1_000 + c,
                author: LOGIN.into(),
                text: "y".repeat(COMMENT_KEPT_CHARS * 2),
            })
            .collect(),
        candidates: Vec::new(),
        counted: true,
        batch: 1,
        waiting_since: 0,
        replies: 0,
    }
}

fn replied() -> ThreadState {
    ThreadState::Replied { comment_id: 1 }
}

#[test]
fn a_stages_threads_are_capped_oldest_resolved_first() {
    let mut fx = watched();
    // 700 records, oldest first: 100 new, 200 replied, 100 ignored, 100 tasked, 200
    // new; the first of the newer new ones is in the open batch.
    let mut threads = Vec::new();
    for k in 1..=700u64 {
        let state = match k {
            101..=300 => replied(),
            301..=400 => ThreadState::Ignored {
                reason: "a resolved thread".into(),
            },
            401..=500 => ThreadState::Tasked {
                task: format!("fix{k}"),
            },
            _ => ThreadState::New,
        };
        threads.push(thread(k, state, if k == 700 { 300 } else { 2 }));
    }
    let stage = &mut fx.run_mut().delivery.stages[0];
    stage.threads = threads;
    stage.batch = Some(Batch {
        threads: vec!["t501".into()],
        ..Batch::default()
    });
    poll_with(&mut fx, view(&commit(1)));
    let stage = fx.run().delivery.stage(1).unwrap();
    assert_eq!(stage.threads.len(), THREADS_KEPT);
    let keys: Vec<&str> = stage.threads.iter().map(|t| t.key.as_str()).collect();
    // 200 go: the 200 oldest resolved ones (every replied one), not the older new
    // ones, and the order holds.
    assert_eq!(keys[0], "t1", "{:?}", &keys[..3]);
    assert_eq!(keys[100], "t301", "{:?}", &keys[99..102]);
    assert_eq!(keys[THREADS_KEPT - 1], "t700");
    assert!(keys.windows(2).all(|w| w[0] != w[1]));
    // The newest thread keeps its newest ids.
    let newest = stage.threads.last().unwrap();
    let ids: Vec<u64> = newest.comments.iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), THREAD_IDS_KEPT);
    assert_eq!(ids[0], 700_000 + 300 - THREAD_IDS_KEPT as u64 + 1);
    assert_eq!(*ids.last().unwrap(), 700_300);
    assert_eq!(newest.last_comment_id, 700_300);
}

#[test]
fn past_the_resolved_ones_only_unheld_threads_drop() {
    let mut fx = watched();
    // 600 records, none resolved: 550 tasked, 50 new (the oldest new in the batch).
    let mut threads = Vec::new();
    for k in 1..=600u64 {
        let state = match k {
            1..=50 => ThreadState::New,
            _ => ThreadState::Tasked {
                task: format!("fix{k}"),
            },
        };
        threads.push(thread(k, state, 1));
    }
    let stage = &mut fx.run_mut().delivery.stages[0];
    stage.threads = threads;
    stage.batch = Some(Batch {
        threads: vec!["t1".into()],
        ..Batch::default()
    });
    poll_with(&mut fx, view(&commit(1)));
    let stage = fx.run().delivery.stage(1).unwrap();
    let keys: Vec<&str> = stage.threads.iter().map(|t| t.key.as_str()).collect();
    // Only the 49 unbatched new ones may go; the tasked ones and t1 stay.
    assert_eq!(keys.len(), 600 - 49);
    assert_eq!(keys[0], "t1");
    assert_eq!(keys[1], "t51");
}

/// The size bound: whatever the views brought, a stage's threads in `run.json` are at
/// most `THREADS_KEPT` records of `THREAD_IDS_KEPT` ids, with the stage's text budget.
#[test]
fn a_stages_threads_have_a_size_bound() {
    let mut fx = watched();
    let threads: Vec<ThreadRecord> = (1..=800u64).map(|k| thread(k, replied(), 400)).collect();
    fx.run_mut().delivery.stages[0].threads = threads;
    poll_with(&mut fx, view(&commit(1)));
    let stage = fx.run().delivery.stage(1).unwrap();
    let bytes = serde_json::to_string(&stage.threads).unwrap().len();
    // A record: its key, author, path (cut to 200), line, state and counters, about
    // 600 bytes; an id: `{"id":…,"author":…,"text":""}`, about 90 bytes; text: at
    // most 4 bytes a character.
    let bound = THREADS_KEPT * (600 + THREAD_IDS_KEPT * 90) + 4 * STAGE_TEXT_CHARS;
    assert!(bytes <= bound, "{bytes} > {bound}");
    assert!(stage.threads.len() <= THREADS_KEPT);
}
