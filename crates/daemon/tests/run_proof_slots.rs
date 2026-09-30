//! Milestone 9.1 ruling C-12b: the test proof asks for its slots before each of its
//! commands (`setup`, the red run, the head run), passes the variables it is given to
//! that command, and gives the slots back before the next git step. A real scratch
//! worktree and real shells; the hook stands in for the daemon's scheduler.

mod support;

use std::sync::{Arc, Mutex, MutexGuard};

use daemon::run::proof::{
    ProofOp, ProofStep, direct, proof_command, proof_pattern, run_proof_scheduled,
};
use support::run_git::{T, commit_file, real_git, repo, wt_dir};

/// The log, whatever a panicking holder left (AGENTS.md rule 3's recovery).
fn lock(log: &Mutex<Vec<String>>) -> MutexGuard<'_, Vec<String>> {
    log.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Drops by appending `released <step>` to the shared log.
struct Hold(Arc<Mutex<Vec<String>>>, ProofStep);

impl Drop for Hold {
    fn drop(&mut self) {
        lock(&self.0).push(format!("released {:?}", self.1));
    }
}

#[test]
fn the_proof_holds_a_slot_for_each_command_only() {
    let repo = repo();
    commit_file(&repo.root, "README", "base\n", "base");
    let test = "echo \"$SLOT_STEP\" >> \"$PROOF_LOG\"\ngrep -q reset impl.txt || exit 1\necho 'PASS t_reset'\n";
    let red = commit_file(&repo.root, "tests/t_reset.sh", test, "red");
    let green = commit_file(&repo.root, "impl.txt", "fn reset() {}\n", "green");
    let (_wt, wt) = wt_dir();
    let logs = tempfile::tempdir().unwrap();
    let log = logs.path().join("proof.log");
    let op = ProofOp {
        root: repo.root.clone(),
        path: wt.join("runs/r1/t1.proof"),
        repo: wt.join("data/tasks/t1.proof"),
        red,
        head: green,
        command: proof_command("sh tests/{test}.sh", "t_reset"),
        passed: proof_pattern("PASS {test}", "t_reset"),
        timeout_secs: 60,
        setup: Some("echo \"$SLOT_STEP\" >> \"$PROOF_LOG\"".into()),
        env: vec![("PROOF_LOG".into(), log.display().to_string())],
        confine: None,
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let slot = move |step: ProofStep| {
        lock(&seen).push(format!("granted {step:?}"));
        let extra = vec![("SLOT_STEP".to_string(), format!("{step:?}"))];
        Ok((extra, Box::new(Hold(seen.clone(), step)) as Box<dyn Send>))
    };
    let runs = run_proof_scheduled(real_git(), &op, T, &direct, &slot).unwrap();
    assert!(
        runs.red_failed && runs.head_passed && runs.matched,
        "{runs:?}"
    );
    let ran = std::fs::read_to_string(&log).unwrap();
    assert_eq!(ran, "Setup\nRed\nHead\n");
    assert_eq!(
        *lock(&events),
        vec![
            "granted Setup",
            "released Setup",
            "granted Red",
            "released Red",
            "granted Head",
            "released Head",
        ]
    );
}
