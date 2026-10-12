//! Milestone 9.13 decision 12 (task M9.13.6): before every gate command the run's pinned
//! gate scripts are hashed in the checkout, and a script that differs from its blob at
//! the base commit (rewritten, a symlink, missing) is never run. A temporary repository
//! and a real shell; no PTY, no daemon.

use crate::support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use daemon::run::driver::tier_step::{StepCommand, run_isolated};
use daemon::run::gate_words::PinnedFile;
use daemon::run::proof::{ProofError, ProofOp, direct, run_proof};
use support::TempRepo;
use support::run_git::{T, commit_file, out, real_git, repo, write, wt_dir};

const SCRIPT: &str = "scripts/check.sh";
/// Passes, unless `red.txt` is there (the proof's red commit adds it).
const ORIGINAL: &str = "echo checked\ntest ! -f red.txt\n";

/// A repository whose base commit holds [`SCRIPT`]; the script pinned at that commit.
fn based() -> (TempRepo, PinnedFile) {
    let repo = repo();
    let base = commit_file(&repo.root, SCRIPT, ORIGINAL, "base");
    let blob = out(&repo.root, &["rev-parse", &format!("{base}:{SCRIPT}")]);
    let pinned = PinnedFile {
        path: SCRIPT.to_string(),
        blob,
    };
    (repo, pinned)
}

/// Where the rewritten script would leave its mark: outside the repository.
fn marker() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ran");
    (dir, path)
}

/// `command` in `checkout`, isolated under `scratch`, with `pinned`.
fn step(
    checkout: &Path,
    scratch: &Path,
    command: &str,
    marker: &Path,
    pinned: &[PinnedFile],
) -> StepCommand {
    StepCommand {
        dir: checkout.to_path_buf(),
        command: command.to_string(),
        env: vec![("MARKER".to_string(), marker.display().to_string())],
        slots: Vec::new(),
        timeout: Duration::from_secs(60),
        confine: None,
        common: checkout.join(".git"),
        base: scratch.join("base"),
        name: "s1-1".to_string(),
        collect: false,
        pinned: pinned.to_vec(),
    }
}

fn rewrite(root: &Path) {
    write(root, SCRIPT, "touch \"$MARKER\"\necho checked\n");
}

#[test]
fn a_matching_gate_script_runs() {
    let (repo, pinned) = based();
    let (scratch, marker) = marker();
    let ran = run_isolated(&step(
        &repo.root,
        scratch.path(),
        "sh scripts/check.sh",
        &marker,
        &[pinned],
    ));
    assert!(ran.outcome.ok, "{:?}", ran.outcome);
    assert_eq!(ran.outcome.gate_refused, None);
    assert_eq!(ran.outcome.tail, "checked");
}

#[test]
fn a_rewritten_gate_script_is_never_run() {
    let (repo, pinned) = based();
    let (scratch, marker) = marker();
    rewrite(&repo.root);
    let ran = run_isolated(&step(
        &repo.root,
        scratch.path(),
        "sh scripts/check.sh",
        &marker,
        &[pinned],
    ));
    assert_eq!(ran.outcome.gate_refused.as_deref(), Some(SCRIPT));
    assert!(!ran.outcome.ok);
    assert!(!marker.exists(), "the rewritten script ran");
}

#[test]
fn a_setup_that_rewrites_the_check_script_is_caught() {
    let (repo, pinned) = based();
    let (scratch, marker) = marker();
    let setup = format!("printf 'touch \"$MARKER\"\\n' > {SCRIPT}");
    let pins = [pinned];
    let first = run_isolated(&step(&repo.root, scratch.path(), &setup, &marker, &pins));
    assert!(first.outcome.ok, "{:?}", first.outcome);
    assert_eq!(first.outcome.gate_refused, None);
    let text = std::fs::read_to_string(repo.root.join(SCRIPT)).unwrap();
    assert!(text.contains("touch"), "setup rewrote the script: {text}");
    let check = run_isolated(&step(
        &repo.root,
        scratch.path(),
        "sh scripts/check.sh",
        &marker,
        &pins,
    ));
    assert_eq!(check.outcome.gate_refused.as_deref(), Some(SCRIPT));
    assert!(!marker.exists(), "the rewritten script ran");
}

#[test]
fn a_symlinked_gate_script_is_refused() {
    let (repo, pinned) = based();
    let (scratch, marker) = marker();
    let copy = repo.root.join("scripts/copy.sh");
    std::fs::write(&copy, ORIGINAL).unwrap();
    std::fs::remove_file(repo.root.join(SCRIPT)).unwrap();
    std::os::unix::fs::symlink("copy.sh", repo.root.join(SCRIPT)).unwrap();
    let ran = run_isolated(&step(
        &repo.root,
        scratch.path(),
        "sh scripts/check.sh",
        &marker,
        &[pinned],
    ));
    assert_eq!(ran.outcome.gate_refused.as_deref(), Some(SCRIPT));
    assert!(!ran.outcome.ok);
}

#[test]
fn a_missing_gate_script_is_refused() {
    let (repo, pinned) = based();
    let (scratch, marker) = marker();
    std::fs::remove_file(repo.root.join(SCRIPT)).unwrap();
    let ran = run_isolated(&step(
        &repo.root,
        scratch.path(),
        "echo ran",
        &marker,
        &[pinned],
    ));
    assert_eq!(ran.outcome.gate_refused.as_deref(), Some(SCRIPT));
    assert!(!ran.outcome.ok);
    assert!(!ran.outcome.tail.contains("ran\n") && ran.outcome.tail != "ran");
}

/// A proof of `sh scripts/check.sh` from `red` to `head`, with `pinned`.
fn proof(
    repo: &TempRepo,
    wt: &Path,
    (red, head): (String, String),
    env: Vec<(String, String)>,
    pinned: &[PinnedFile],
) -> ProofOp {
    ProofOp {
        root: repo.root.clone(),
        path: wt.join("runs/r1/t1.proof"),
        repo: wt.join("data/tasks/t1.proof"),
        red,
        head,
        command: "sh scripts/check.sh".into(),
        passed: "checked".into(),
        timeout_secs: 60,
        setup: None,
        env,
        confine: None,
        red_only: false,
        pinned: pinned.to_vec(),
    }
}

#[test]
fn a_proof_with_a_rewritten_gate_script_is_refused() {
    // The head rewrites the script: the red run happens, the head run never does.
    let (repo, pinned) = based();
    let red_script = "touch \"$RED_MARKER\"\nexit 1\n";
    let red = commit_file(&repo.root, "red.txt", "red\n", "red");
    let head = commit_file(
        &repo.root,
        SCRIPT,
        "touch \"$HEAD_MARKER\"\necho checked\n",
        "rewrite",
    );
    let (_wt, wt) = wt_dir();
    let (marks, _) = marker();
    let (red_mark, head_mark) = (marks.path().join("red"), marks.path().join("head"));
    let env = vec![
        ("RED_MARKER".to_string(), red_mark.display().to_string()),
        ("HEAD_MARKER".to_string(), head_mark.display().to_string()),
    ];
    let op = proof(
        &repo,
        &wt,
        (red.clone(), head.clone()),
        env.clone(),
        std::slice::from_ref(&pinned),
    );
    let err = run_proof(real_git(), &op, T, &direct).unwrap_err();
    assert_eq!(
        err,
        ProofError::GateRefused {
            path: SCRIPT.to_string()
        }
    );
    assert!(!head_mark.exists(), "the rewritten script ran at head");
    assert!(!red_mark.exists());

    // The red commit rewrites it: nothing runs at all.
    let rewritten = commit_file(&repo.root, SCRIPT, red_script, "red rewrite");
    let (_wt2, wt2) = wt_dir();
    let op = proof(&repo, &wt2, (rewritten, head), env, &[pinned]);
    let err = run_proof(real_git(), &op, T, &direct).unwrap_err();
    assert_eq!(
        err,
        ProofError::GateRefused {
            path: SCRIPT.to_string()
        }
    );
    assert!(!red_mark.exists(), "the rewritten script ran at red");
    assert!(!head_mark.exists());
}
