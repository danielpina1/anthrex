//! M8a.8: the review worktree and the reviewer's clamped diff (decision 35, ruling Q4),
//! including diffs whose cuts land inside a character and a diff larger than any
//! capture cap.

mod support;

use daemon::run::contract::{DIFF_CUT_MARKER, REVIEW_DIFF_MAX};
use daemon::run::git::{diff_so_far, prepare_review, prepare_worktree, sync};
use support::run_git::{
    T, commit_file, head, real_git, repo, try_git, worktree_block, write, wt_dir,
};

/// A worker's commit in the task worktree `task`, recorded on the task's branch by the
/// engine (final fix batch F1b: the worker commits on a detached `HEAD`, and the done
/// check that precedes every review records it).
fn task_commit(task: &std::path::Path, path: &str, content: &str, message: &str) -> String {
    let commit = commit_file(task, path, content, message);
    assert_eq!(sync(real_git(), task, T).unwrap(), commit);
    commit
}

#[test]
fn review_worktree_is_detached_at_the_task_head_and_replaced() {
    let repo = repo();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/r7/t1");
    let branch = "anthrex/r7/t1";
    prepare_worktree(real_git(), &repo.root, branch, &base, &task, T).unwrap();
    let small = task_commit(&task, "src/lib.rs", "pub fn f() {}\n", "small");
    let review = wt.join("runs/r7/t1.review");

    let (b, h, patch) = prepare_review(real_git(), &repo.root, branch, &base, &review, T).unwrap();

    assert_eq!((b.as_str(), h.as_str()), (base.as_str(), small.as_str()));
    let expected =
        String::from_utf8(try_git(&repo.root, &["diff", &format!("{base}..{small}")]).stdout)
            .unwrap();
    assert_eq!(patch, expected);
    assert_eq!(head(&review), small);
    let block = worktree_block(&repo.root, &review).unwrap();
    assert!(block.lines().any(|l| l == "detached"), "{block}");

    // The next round replaces the worktree, and a diff over the limit is clamped.
    write(&review, "leftover.txt", "from round one\n");
    let big_body: String = "世世世世\n".repeat(6000);
    let big = task_commit(&task, "src/big.txt", &big_body, "big");

    let (b, h, patch) = prepare_review(real_git(), &repo.root, branch, &base, &review, T).unwrap();

    assert_eq!((b.as_str(), h.as_str()), (base.as_str(), big.as_str()));
    assert_eq!(head(&review), big);
    assert!(
        !review.join("leftover.txt").exists(),
        "the old round's worktree is gone"
    );
    assert!(
        worktree_block(&repo.root, &review)
            .unwrap()
            .lines()
            .any(|l| l == "detached")
    );
    assert!(patch.len() <= REVIEW_DIFF_MAX, "{}", patch.len());
    assert!(patch.len() >= REVIEW_DIFF_MAX - 3, "{}", patch.len());
    // `src/big.txt` sorts before `src/lib.rs`: the head is the big file's, the tail the
    // small change's.
    assert!(
        patch.starts_with("diff --git a/src/big.txt"),
        "the head of the diff is kept"
    );
    assert!(
        patch.ends_with("+pub fn f() {}\n"),
        "the tail of the diff is kept"
    );
}

fn full_diff(root: &std::path::Path, base: &str, head: &str) -> String {
    String::from_utf8(try_git(root, &["diff", "--no-color", &format!("{base}..{head}")]).stdout)
        .unwrap()
}

/// The clamp contract, checked against the whole diff: at most `REVIEW_DIFF_MAX` bytes
/// and at most 3 short, one marker, a head that is a prefix of the diff and a tail that
/// is a suffix of it, and no replacement character from a cut inside a character.
fn assert_clamped(patch: &str, full: &str) {
    assert!(patch.len() <= REVIEW_DIFF_MAX, "{}", patch.len());
    assert!(patch.len() >= REVIEW_DIFF_MAX - 3, "{}", patch.len());
    assert!(!patch.contains('\u{FFFD}'), "a character was cut");
    assert_eq!(patch.matches(DIFF_CUT_MARKER).count(), 1);
    let (head_part, tail_part) = patch.split_once(DIFF_CUT_MARKER).unwrap();
    assert!(
        full.starts_with(head_part),
        "the head is not the diff's start"
    );
    assert!(full.ends_with(tail_part), "the tail is not the diff's end");
}

#[test]
fn clamped_review_diffs_cut_on_character_boundaries_at_every_offset() {
    let repo = repo();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/cb01/t1");
    let branch = "anthrex/cb01/t1";
    prepare_worktree(real_git(), &repo.root, branch, &base, &task, T).unwrap();
    let review = wt.join("runs/cb01/t1.review");
    // 3- and 4-byte characters; each extra byte of file name shifts every cut by one.
    let body = "世𝄞世\n".repeat(8000);
    let budget = REVIEW_DIFF_MAX - DIFF_CUT_MARKER.len();
    let mut cut_inside_a_character = 0;
    for k in 0..6 {
        support::run_git::out(&task, &["reset", "-q", "--hard", &base]);
        let name = format!("src/big{}.txt", "x".repeat(k));
        let h = task_commit(&task, &name, &body, "big");

        let (_, _, patch) =
            prepare_review(real_git(), &repo.root, branch, &base, &review, T).unwrap();

        let full = full_diff(&repo.root, &base, &h);
        if !full.is_char_boundary(budget / 2) {
            cut_inside_a_character += 1;
        }
        assert_clamped(&patch, &full);
    }
    assert!(
        cut_inside_a_character > 0,
        "the fixture never put a cut inside a character"
    );
}

#[test]
fn a_diff_larger_than_any_capture_cap_is_clamped_not_failed() {
    let repo = repo();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/hg01/t1");
    let branch = "anthrex/hg01/t1";
    prepare_worktree(real_git(), &repo.root, branch, &base, &task, T).unwrap();
    // 700 000 lines of 100 bytes: a 70 MB file, a diff over 64 MiB.
    let mut body = String::with_capacity(70_000_000);
    for n in 0..700_000u32 {
        body.push_str(&format!("{n:0>99}\n"));
    }
    task_commit(&task, "data/big.txt", &body, "vendor a dataset");
    drop(body);

    let (_, _, patch) = prepare_review(
        real_git(),
        &repo.root,
        branch,
        &base,
        &wt.join("runs/hg01/t1.review"),
        T,
    )
    .unwrap();
    assert!(patch.len() <= REVIEW_DIFF_MAX && patch.len() >= REVIEW_DIFF_MAX - 3);
    assert!(
        patch.starts_with("diff --git a/data/big.txt b/data/big.txt"),
        "{}",
        &patch[..80]
    );
    assert!(patch.ends_with(&format!("+{:0>99}\n", 699_999)));
    assert_eq!(patch.matches(DIFF_CUT_MARKER).count(), 1);

    let (stat, patch) = diff_so_far(real_git(), &task, &base, &base, T).unwrap();
    assert!(stat.contains("data/big.txt"), "{stat}");
    assert!(patch.len() <= REVIEW_DIFF_MAX && patch.len() >= REVIEW_DIFF_MAX - 3);
    assert!(patch.ends_with(&format!("+{:0>99}\n", 699_999)));
}

#[test]
fn diffs_ignore_the_users_colour_prefix_and_textconv_config() {
    let repo = repo();
    let out = support::run_git::out;
    commit_file(
        &repo.root,
        ".gitattributes",
        "*.txt diff=shout\n",
        "attributes",
    );
    out(&repo.root, &["config", "color.ui", "always"]);
    out(&repo.root, &["config", "diff.noprefix", "true"]);
    out(
        &repo.root,
        &["config", "diff.shout.textconv", "sed s/two/TEXTCONV/"],
    );
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/cf01/t1");
    let branch = "anthrex/cf01/t1";
    prepare_worktree(real_git(), &repo.root, branch, &base, &task, T).unwrap();
    task_commit(&task, "src/a.txt", "one\ntwo\n", "a");

    let (_, _, review) = prepare_review(
        real_git(),
        &repo.root,
        branch,
        &base,
        &wt.join("runs/cf01/t1.review"),
        T,
    )
    .unwrap();
    let (stat, patch) = diff_so_far(real_git(), &task, &base, &base, T).unwrap();

    for text in [&review, &stat, &patch] {
        assert!(!text.contains('\u{1b}'), "colour escapes: {text:?}");
        assert!(!text.contains("TEXTCONV"), "textconv ran: {text:?}");
    }
    for diff in [&review, &patch] {
        assert!(
            diff.starts_with("diff --git a/src/a.txt b/src/a.txt\n"),
            "{diff:?}"
        );
        assert!(diff.contains("\n+two\n"), "{diff:?}");
    }
}
