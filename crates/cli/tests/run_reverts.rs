//! M8b.17 review (I1): decision 34's revert detection runs at every `run start`, in the
//! background, not only at `run stats`. Both runtime commands are the harness's
//! `fake-agent` and the decider command a path that does not exist; the run stops at
//! its plan gate, so no session starts at all.

mod support;

use std::time::{SystemTime, UNIX_EPOCH};

use daemon::run::engine::HISTORY_FILE;
use proto::{HISTORY_VERSION, HistoryLine, RunRecord};
use support::run_harness::{RUN_WAIT, RunHarness, git_in};
use support::run_plans::{plan, task, until};

#[test]
fn run_start_records_a_revert_of_an_accepted_run() {
    let h = RunHarness::with_env(
        "",
        &[("ANTHREX_DECIDER_BIN", "/nonexistent/anthrex-test/decider")],
        true,
    );
    let repo = &h.repo;
    // An accepted run: its branch merged into `main` with `--no-ff`, as `run accept`
    // does, then that merge reverted (a two-parent merge needs `-m 1`).
    git_in(repo, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(repo.join("b.txt"), "b\n").unwrap();
    git_in(repo, &["add", "b.txt"]);
    git_in(repo, &["commit", "-q", "-m", "the run's work"]);
    git_in(repo, &["checkout", "-q", "main"]);
    git_in(repo, &["merge", "-q", "--no-ff", "--no-edit", "feature"]);
    let accept = git_in(repo, &["rev-parse", "HEAD"]);
    git_in(repo, &["revert", "-m", "1", "--no-edit", &accept]);
    let revert = git_in(repo, &["rev-parse", "HEAD"]);

    let project = repo.canonicalize().unwrap();
    let path = daemon::profile::repo_dir(&h.data(), &project).join(HISTORY_FILE);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let run = HistoryLine::Run(RunRecord {
        v: HISTORY_VERSION,
        record_id: "r1".into(),
        at: now,
        run_id: "r1".into(),
        goal: "g".into(),
        path: None,
        triage: None,
        profile_source: None,
        outcome: "accepted".into(),
        base_branch: "main".into(),
        accepted_commit: Some(accept.clone()),
        tasks: 1,
        usage: None,
    });
    std::fs::write(&path, format!("{}\n", serde_json::to_string(&run).unwrap())).unwrap();

    // A plain start, stopped at its plan gate; `run stats` is never asked.
    h.start(&plan("", &[task("t1", &["a.txt"], "")]), false);
    let line = until("the revert record", RUN_WAIT, || {
        let text = std::fs::read_to_string(&path).ok()?;
        text.lines()
            .find(|l| l.contains(&format!("\"record_id\":\"revert/{revert}\"")))
            .map(str::to_string)
    });
    let HistoryLine::Revert(record) = serde_json::from_str(&line).unwrap() else {
        panic!("not a revert record: {line}");
    };
    assert_eq!(
        (record.run_id.as_str(), record.task_id, record.reverted),
        ("r1", None, accept)
    );
    assert_eq!(record.revert_commit, revert);
}
