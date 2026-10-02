//! Task M9.2.10's fix round (the controller's ruling on concern 2): the journal never
//! carries what `run.json` refuses. A `PrViewed` answer is cut to the same caps before
//! it is journaled: each thread's newest comments only, each comment cut, a stage's
//! review text within its budget, every id kept, and anthrex's marker kept at the end
//! of a cut body (so a reply anthrex sent is still known by it).

use super::view_trim::{CUT_MARK, journal_line, journaled};
use crate::host::{
    Author, IssueComment, Mergeable, PrState, PrView, Review, ReviewState, ReviewThread,
    ThreadComment,
};
use crate::run::delivery::ops::HostResult;
use crate::run::engine::OpResult;
use crate::run::engine::delivery::review_limits::{
    COMMENT_KEPT_CHARS, STAGE_TEXT_CHARS, THREAD_COMMENTS_KEPT,
};
use crate::run::journal::{COMPACT_AFTER_BYTES, JournalLine};

fn by(login: &str) -> Author {
    Author {
        login: login.into(),
        bot: false,
    }
}

const MARKER: &str = "<!-- anthrex:reply r1a2b 7:t1 1a2b3c4 -->";

/// The largest view `gh` can answer (decision 23's pages): 100 threads of 50 comments,
/// 100 conversation comments and 100 reviews, every body 8 000 two-byte characters.
fn huge() -> PrView {
    let body: String = std::iter::repeat_n('é', 8_000).collect();
    let hunk: String = std::iter::repeat_n('x', 8_000).collect();
    let mut id = 0;
    let mut next = || {
        id += 1;
        id
    };
    let threads = (0..100)
        .map(|_| ReviewThread {
            resolved: false,
            path: Some("src/lib.rs".into()),
            line: Some(1),
            comments: (0..50)
                .map(|_| ThreadComment {
                    id: next(),
                    author: by("alice"),
                    body: body.clone(),
                    diff_hunk: hunk.clone(),
                })
                .collect(),
        })
        .collect();
    let comments = (0..100)
        .map(|_| IssueComment {
            id: next(),
            author: by("bob"),
            body: body.clone(),
        })
        .collect();
    let reviews = (0..100)
        .map(|_| Review {
            id: next(),
            author: by("carol"),
            state: ReviewState::ChangesRequested,
            body: body.clone(),
        })
        .collect();
    PrView {
        number: 7,
        state: PrState::Open,
        merged_at: None,
        merge_commit: None,
        base_ref: "main".into(),
        head_oid: "a".repeat(40),
        mergeable: Mergeable::Mergeable,
        review_decision: None,
        checks: Vec::new(),
        reviews,
        comments,
        threads,
    }
}

fn ids(view: &PrView) -> Vec<u64> {
    let threads = view.threads.iter().flat_map(|t| &t.comments).map(|c| c.id);
    let comments = view.comments.iter().map(|c| c.id);
    threads
        .chain(comments)
        .chain(view.reviews.iter().map(|r| r.id))
        .collect()
}

fn trimmed(view: PrView) -> PrView {
    let result = journaled(OpResult::Host(HostResult::PrViewed(Box::new(view))));
    match result {
        OpResult::Host(HostResult::PrViewed(view)) => *view,
        other => panic!("not a view: {other:?}"),
    }
}

#[test]
fn a_viewed_pr_is_journaled_within_run_jsons_caps() {
    let mut view = huge();
    // anthrex's own reply, long, the newest comment of the newest thread; another as an
    // old comment of the oldest thread.
    let long = format!("{}\n\n{MARKER}", "y".repeat(COMMENT_KEPT_CHARS + 10));
    view.threads[99].comments[49].body = long;
    view.threads[0].comments[0].body = format!("old\n\n{MARKER}");
    let before = ids(&view);
    let view = trimmed(view);
    assert_eq!(ids(&view), before, "every id is kept");
    let line = JournalLine::Done {
        op: 1,
        result: OpResult::Host(HostResult::PrViewed(Box::new(view.clone()))),
    };
    let bytes = serde_json::to_vec(&line).unwrap().len() as u64;
    assert!(bytes < COMPACT_AFTER_BYTES, "{bytes} bytes");
    // Each thread keeps the text of its newest comments only; the first comment's
    // hunk is the only one kept (the engine reads no other).
    for t in &view.threads {
        let (old, new) = t.comments.split_at(50 - THREAD_COMMENTS_KEPT);
        assert!(
            old.iter()
                .all(|c| c.body.is_empty() || c.body.ends_with(MARKER))
        );
        assert!(new.iter().all(|c| !c.body.is_empty()));
        assert!(t.comments[1..].iter().all(|c| c.diff_hunk.is_empty()));
        assert!(t.comments[0].diff_hunk.chars().count() <= COMMENT_KEPT_CHARS);
    }
    let text = |s: &str| s.strip_suffix(MARKER).unwrap_or(s).chars().count();
    let all = (view.reviews.iter().map(|r| r.body.as_str()))
        .chain(view.comments.iter().map(|c| c.body.as_str()))
        .chain(
            view.threads
                .iter()
                .flat_map(|t| &t.comments)
                .map(|c| c.body.as_str()),
        )
        .filter(|b| *b != CUT_MARK)
        .map(text)
        .sum::<usize>();
    let hunks: usize = (view.threads.iter())
        .map(|t| t.comments[0].diff_hunk.chars().count())
        .sum();
    assert!(all + hunks <= STAGE_TEXT_CHARS + 50 * 4, "{all} + {hunks}");
    // A body cut by the budget is never empty, so a changes-requested review still
    // reads as one; one within it is cut to the comment cap.
    assert!(view.reviews.iter().all(|r| !r.body.is_empty()));
    assert_eq!(view.reviews[99].body.chars().count(), COMMENT_KEPT_CHARS);
    assert!(view.comments.iter().any(|c| c.body == CUT_MARK));
    // anthrex's marker survives a cut, and an old comment keeps nothing else.
    let newest = &view.threads[99].comments[49].body;
    assert!(newest.ends_with(MARKER), "{}", &newest[newest.len() - 80..]);
    assert!(newest.chars().count() <= COMMENT_KEPT_CHARS + 2 + MARKER.len());
    assert_eq!(view.threads[0].comments[0].body, MARKER);
}

#[test]
fn a_small_view_and_other_results_are_journaled_as_they_are() {
    let mut view = huge();
    view.threads.truncate(1);
    view.threads[0].comments.truncate(2);
    view.threads[0].comments[1].body = format!("short\n\n{MARKER}");
    view.comments.truncate(1);
    view.reviews.clear();
    for c in &mut view.threads[0].comments {
        c.diff_hunk = "@@ -1 +1 @@".into();
    }
    view.threads[0].comments[0].body = "a".into();
    view.comments[0].body = "b".into();
    let mut want = view.clone();
    want.threads[0].comments[1].diff_hunk.clear();
    assert_eq!(trimmed(view), want);
    let other = OpResult::Failed {
        message: "x".repeat(10_000),
    };
    assert_eq!(journaled(other.clone()), other);
}

/// The final fix wave's A4: a `FailedLogs` answer's journal line keeps the file's path
/// and size, never its text; anything else is journaled as it is handed over.
#[test]
fn a_ci_logs_journal_line_carries_no_tail() {
    let file = crate::host::LogFile {
        path: "/tmp/data/delivery/ci-28000000001.log".into(),
        bytes: 48_000,
        truncated: true,
        tail: "--- FAIL: a::works".into(),
    };
    let result = OpResult::Host(HostResult::Logs(file.clone()));
    let line = journal_line(&result);
    let OpResult::Host(HostResult::Logs(kept)) = &line else {
        panic!("{line:?}")
    };
    assert_eq!(kept.tail, "");
    assert_eq!(
        (&kept.path, kept.bytes, kept.truncated),
        (&file.path, 48_000, true)
    );
    let text = serde_json::to_string(&JournalLine::Done {
        op: 3,
        result: line,
    })
    .unwrap();
    assert!(!text.contains("a::works"), "{text}");
    let other = OpResult::Host(HostResult::Rerun);
    assert_eq!(journal_line(&other), other);
}
