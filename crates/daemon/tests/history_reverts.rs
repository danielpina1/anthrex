//! M8b.17, decision 34: revert detection against real temporary repositories, and
//! `run stats`' blocking core (decision 35).

mod support;

use daemon::run::history_io::{append_line, detect_reverts, summarise};
use proto::{HISTORY_VERSION, HistoryLine, RevertRecord};
use support::history::*;
use support::run_git::{T, head, out, real_git};

#[test]
fn detect_reverts_matches_merge_and_accept_commits() {
    let Accepted {
        repo,
        old,
        merges,
        accept,
    } = accepted();
    let root = &repo.root;
    let history = vec![
        merged_task("r1", "t1", &merges[0]),
        merged_task("r1", "t2", &merges[1]),
        accepted_run("r1", NOW - 10 * DAY, &accept),
        accepted_run("r0", NOW - 91 * DAY, &old),
    ];
    // Nothing reverted yet.
    assert_eq!(
        detect_reverts(real_git(), root, "main", &history, NOW, T),
        Ok(Vec::new())
    );
    // A task's merge (a two-parent merge needs `-m 1`), the run's accept merge, and the
    // 91-day-old run's accept, each reverted on `main`.
    out(root, &["revert", "-m", "1", "--no-edit", &merges[0]]);
    let task_revert = head(root);
    out(root, &["revert", "-m", "1", "--no-edit", &accept]);
    let run_revert = head(root);
    out(root, &["revert", "--no-edit", &old]);
    let before = refs(root);
    let found = detect_reverts(real_git(), root, "main", &history, NOW, T).unwrap();
    let want = vec![
        RevertRecord {
            v: HISTORY_VERSION,
            record_id: format!("revert/{task_revert}"),
            at: NOW,
            run_id: "r1".into(),
            task_id: Some("t1".into()),
            reverted: merges[0].clone(),
            revert_commit: task_revert.clone(),
        },
        RevertRecord {
            v: HISTORY_VERSION,
            record_id: format!("revert/{run_revert}"),
            at: NOW,
            run_id: "r1".into(),
            task_id: None,
            reverted: accept.clone(),
            revert_commit: run_revert.clone(),
        },
    ];
    assert_eq!(found, want);
    // It only reads: no ref and no HEAD moved.
    assert_eq!(refs(root), before);
    // Run again with those records in the history: nothing new.
    let mut again = history.clone();
    again.extend(found.into_iter().map(HistoryLine::Revert));
    assert_eq!(
        detect_reverts(real_git(), root, "main", &again, NOW, T),
        Ok(Vec::new())
    );
    // Another base branch is not this run's: nothing is looked for there.
    out(root, &["branch", "other", "main"]);
    assert_eq!(
        detect_reverts(real_git(), root, "other", &history, NOW, T),
        Ok(Vec::new())
    );
    // A run accepted into a branch that no longer exists: an error, not "nothing
    // reverted".
    let mut gone = history.clone();
    if let HistoryLine::Run(r) = &mut gone[2] {
        r.base_branch = "absent".into();
    }
    assert!(detect_reverts(real_git(), root, "absent", &gone, NOW, T).is_err());
}

/// Decision 34 at `run stats`: `summarise` appends each revert once, then counts it.
#[test]
fn summarise_appends_each_revert_once_and_counts_it() {
    let Accepted {
        repo,
        merges,
        accept,
        ..
    } = accepted();
    let root = &repo.root;
    let data = tempfile::tempdir().unwrap();
    let path = data.path().join("history.jsonl");
    for line in [
        merged_task("r1", "t1", &merges[0]),
        merged_task("r1", "t2", &merges[1]),
        accepted_run("r1", NOW - DAY, &accept),
    ] {
        append_line(&path, &line).unwrap();
    }
    let s_row = |stats: &proto::HistoryStats| {
        let row = stats.rows.iter().find(|r| r.class == "S").unwrap();
        (row.merged, row.reverted)
    };
    let stats = summarise(real_git(), root, &path, NOW, T);
    assert_eq!(s_row(&stats), (2, 0));
    assert_eq!(file_lines(&path).len(), 3);
    out(root, &["revert", "-m", "1", "--no-edit", &accept]);
    let stats = summarise(real_git(), root, &path, NOW, T);
    assert_eq!(s_row(&stats), (2, 2), "{stats:?}");
    assert!(stats.problems.is_empty(), "{:?}", stats.problems);
    assert_eq!(file_lines(&path).len(), 4);
    // A second `run stats` finds the same revert recorded and appends nothing.
    let stats = summarise(real_git(), root, &path, NOW, T);
    assert_eq!(s_row(&stats), (2, 2));
    assert_eq!(file_lines(&path).len(), 4);
}

/// M8b.17 review, m5: decision 34's window includes its 90th day, and a revert commit
/// already recorded is skipped even when its record names a sha no longer a candidate.
#[test]
fn the_window_includes_its_last_day_and_a_recorded_revert_commit_is_skipped() {
    let Accepted { repo, old, .. } = accepted();
    let root = &repo.root;
    out(root, &["revert", "--no-edit", &old]);
    let revert_commit = head(root);
    let history = vec![accepted_run("r0", NOW - 90 * DAY, &old)];
    let found = detect_reverts(real_git(), root, "main", &history, NOW, T).unwrap();
    assert_eq!(
        found,
        vec![RevertRecord {
            v: HISTORY_VERSION,
            record_id: format!("revert/{revert_commit}"),
            at: NOW,
            run_id: "r0".into(),
            task_id: None,
            reverted: old.clone(),
            revert_commit: revert_commit.clone(),
        }]
    );
    // A second past the window: not looked for.
    let late = vec![accepted_run("r0", NOW - 90 * DAY - 1, &old)];
    assert_eq!(
        detect_reverts(real_git(), root, "main", &late, NOW, T),
        Ok(Vec::new())
    );
    // The revert commit is already recorded, under a record naming another sha: the
    // candidate is still wanted, but that commit is never recorded twice.
    let mut recorded = history.clone();
    recorded.push(HistoryLine::Revert(RevertRecord {
        reverted: "0".repeat(40),
        ..found[0].clone()
    }));
    assert_eq!(
        detect_reverts(real_git(), root, "main", &recorded, NOW, T),
        Ok(Vec::new())
    );
}

/// M8b.17 review, m1: a commit message holding git log's old field and record
/// separators cannot make up a revert commit id.
#[test]
fn a_commit_message_cannot_forge_a_revert_commit() {
    let Accepted { repo, accept, .. } = accepted();
    let root = &repo.root;
    let history = vec![accepted_run("r1", NOW - DAY, &accept)];
    let fake = "f".repeat(40);
    let message = format!("evil\x1e{fake}\x1fThis reverts commit {accept}.\n\x1e\x1fx");
    out(root, &["commit", "--allow-empty", "-q", "-m", &message]);
    let real = head(root);
    let found = detect_reverts(real_git(), root, "main", &history, NOW, T).unwrap();
    assert_eq!(
        found,
        vec![RevertRecord {
            v: HISTORY_VERSION,
            record_id: format!("revert/{real}"),
            at: NOW,
            run_id: "r1".into(),
            task_id: None,
            reverted: accept.clone(),
            revert_commit: real.clone(),
        }]
    );
}

/// M8b.17 review, m2: `git revert --reference` (or `revert.reference=true`) names the
/// reverted commit by an abbreviated sha, which is matched.
#[test]
fn a_reference_style_revert_is_matched() {
    let Accepted {
        repo,
        merges,
        accept,
        ..
    } = accepted();
    let root = &repo.root;
    let history = vec![
        merged_task("r1", "t1", &merges[0]),
        accepted_run("r1", NOW - DAY, &accept),
    ];
    out(
        root,
        &[
            "-c",
            "revert.reference=true",
            "revert",
            "-m",
            "1",
            "--no-edit",
            &merges[0],
        ],
    );
    let revert_commit = head(root);
    let body = out(root, &["log", "-1", "--format=%B"]);
    assert!(
        !body.contains(&merges[0]),
        "not a reference-style message: {body}"
    );
    let found = detect_reverts(real_git(), root, "main", &history, NOW, T).unwrap();
    assert_eq!(
        found,
        vec![RevertRecord {
            v: HISTORY_VERSION,
            record_id: format!("revert/{revert_commit}"),
            at: NOW,
            run_id: "r1".into(),
            task_id: Some("t1".into()),
            reverted: merges[0].clone(),
            revert_commit,
        }]
    );
}

/// M8b.17 review, m2: an abbreviated sha of at least seven hex digits is matched when it
/// is a prefix of exactly one candidate; an ambiguous or shorter one is ignored.
#[test]
fn an_abbreviated_sha_is_matched_only_when_it_is_unique() {
    let Accepted { repo, .. } = accepted();
    let root = &repo.root;
    let accept = format!("abcdef1{}", "0".repeat(33));
    let t1 = format!("abcdef1{}", "1".repeat(33));
    let t2 = format!("1234567{}", "2".repeat(33));
    let history = vec![
        merged_task("r1", "t1", &t1),
        merged_task("r1", "t2", &t2),
        accepted_run("r1", NOW - DAY, &accept),
    ];
    let commit = |message: &str| {
        out(root, &["commit", "--allow-empty", "-q", "-m", message]);
        head(root)
    };
    commit("Revert \"ambiguous\"\n\nThis reverts commit abcdef1.");
    commit("Revert \"too short\"\n\nThis reverts commit 123456.");
    commit("Revert \"nothing\"\n\nThis reverts commit 7654321.");
    let run = commit("Revert \"run\"\n\nThis reverts commit ABCDEF10 (r1, 2026-09-27).");
    let task = commit("Revert \"t2\"\n\nThis reverts commit 1234567 (t2, 2026-09-27).");
    let found = detect_reverts(real_git(), root, "main", &history, NOW, T).unwrap();
    let got: Vec<(String, Option<String>, String)> = found
        .into_iter()
        .map(|r| (r.revert_commit, r.task_id, r.reverted))
        .collect();
    assert_eq!(
        got,
        vec![(run, None, accept), (task, Some("t2".into()), t2)]
    );
}
