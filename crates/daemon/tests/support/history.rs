//! M8b.16 and M8b.17: the history records and the accepted-run repository the
//! `history_io` and `history_reverts` tests share.

use std::path::Path;

use proto::{HISTORY_VERSION, HistoryLine, RevertRecord, RunRecord, TaskRecord};

use super::TempRepo;
use super::run_git::{commit_file, head, out, repo};

pub fn revert(id: &str, n: u64) -> HistoryLine {
    HistoryLine::Revert(RevertRecord {
        v: HISTORY_VERSION,
        record_id: id.into(),
        at: n,
        run_id: "r".into(),
        task_id: None,
        reverted: "a".repeat(40),
        revert_commit: "b".repeat(40),
    })
}

pub fn file_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

pub fn run_line(outcome: &str) -> HistoryLine {
    HistoryLine::Run(RunRecord {
        v: HISTORY_VERSION,
        record_id: "r".into(),
        at: 1,
        run_id: "r".into(),
        goal: "g".into(),
        path: None,
        triage: None,
        profile_source: None,
        outcome: outcome.into(),
        base_branch: "main".into(),
        accepted_commit: None,
        tasks: 1,
        usage: None,
    })
}

/// A merged task record of `run` whose merge commit is `merge`.
pub fn merged_task(run: &str, task: &str, merge: &str) -> HistoryLine {
    // Built through JSON so this test does not spell every field of the record.
    let line = serde_json::json!({
        "type": "task", "v": 1, "record_id": format!("{run}/{task}"), "at": 1,
        "run_id": run, "task_id": task, "path": null, "kind": "code", "hub": false,
        "test_mode": "tdd", "planned_size": "S", "final_size": "S", "size_check": null,
        "route": {"runtime": "claude", "model": "m", "strength": "standard", "effort": "medium"},
        "review_routes": [], "outcome": "merged", "block": null,
        "diff": {"files": 1, "hunks": 1, "added": 1, "removed": 0}, "tool_calls": 1,
        "worker_usage": {"input": 1, "output": 1, "cache_read": 0, "cache_write": 0},
        "reviewer_usage": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0},
        "decider_usage": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0},
        "phases": {"queued": 0, "preparing": 0, "working": 60, "proof": 0, "check": 0,
                   "review": 0, "merge": 0, "blocked": 0},
        "wall_secs": 60,
        "gates": {"proofs": 0, "proofs_failed": 0, "checks": 0, "checks_failed": 0,
                  "review_rounds": 0, "reviews_rejected": 0, "candidates_red": 0,
                  "generated_bounces": 0},
        "severities": {"critical": 0, "important": 0, "minor": 0},
        "bounces": {"done": 0, "proof": 0, "check": 0, "review": 0, "merge": 0},
        "failures": 0, "stalls": 0, "budget_exceeded": 0, "conflicts": 0, "max_rung": 0,
        "sessions": 1, "done_signal": null, "merge_commit": merge,
    });
    HistoryLine::Task(serde_json::from_value::<TaskRecord>(line).unwrap())
}

/// An accepted run record of `run`, `at`, whose accept merge is `accepted`.
pub fn accepted_run(run: &str, at: u64, accepted: &str) -> HistoryLine {
    HistoryLine::Run(RunRecord {
        record_id: run.into(),
        run_id: run.into(),
        at,
        accepted_commit: Some(accepted.into()),
        ..match run_line("accepted") {
            HistoryLine::Run(r) => r,
            _ => unreachable!(),
        }
    })
}

pub const DAY: u64 = 86_400;
/// The test's `now`.
pub const NOW: u64 = 2_000_000_000;

/// The repository an accepted two-task run leaves, and its commits.
pub struct Accepted {
    pub repo: TempRepo,
    pub old: String,
    pub merges: [String; 2],
    pub accept: String,
}

/// `main` with an old commit (the accept of a run 91 days old), then run `r1`: tasks
/// `t1` and `t2` merged into its branch with `--no-ff` (two-parent merges, as the
/// engine's `commit-tree` makes), and the branch merged into `main` with `--no-ff`
/// (as `run accept` does).
pub fn accepted() -> Accepted {
    let repo = repo();
    let root = repo.root.clone();
    out(&root, &["branch", "-M", "main"]);
    let old = commit_file(&root, "old.txt", "old\n", "an old run's work");
    out(&root, &["checkout", "-q", "-b", "anthrex/r1"]);
    let mut merges = Vec::new();
    for task in ["t1", "t2"] {
        out(&root, &["checkout", "-q", "-b", task, "anthrex/r1"]);
        commit_file(&root, &format!("{task}.txt"), "work\n", task);
        out(&root, &["checkout", "-q", "anthrex/r1"]);
        out(&root, &["merge", "-q", "--no-ff", "--no-edit", task]);
        merges.push(head(&root));
    }
    out(&root, &["checkout", "-q", "main"]);
    out(
        &root,
        &["merge", "-q", "--no-ff", "--no-edit", "anthrex/r1"],
    );
    let accept = head(&root);
    Accepted {
        repo,
        old,
        merges: [merges[0].clone(), merges[1].clone()],
        accept,
    }
}

/// Every ref and what it points at, and `HEAD`.
pub fn refs(root: &Path) -> String {
    format!(
        "{}\n{}",
        out(root, &["for-each-ref", "--format=%(refname) %(objectname)"]),
        head(root)
    )
}
