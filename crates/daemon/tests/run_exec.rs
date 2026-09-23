//! M8a.10: the engine's own shells (decisions 33 and 34) — `check`'s bounded tail, its
//! process-group timeout, and the fail-to-pass test proof in a real scratch worktree.
//! The environment test is alone in `run_exec_env.rs`.

mod support;

use daemon::manager::KILL_GRACE;
use daemon::run::exec::{
    CHECK_SUMMARY_LINES, CHECK_TAIL_LINES, OUTPUT_GRACE, ShellOutcome, run_shell, summary,
};
use daemon::run::proof::{ProofError, ProofOp, ProofRuns, proof_command, proof_pattern, run_proof};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use support::TempRepo;
use support::run_git::{T, commit_file, real_git, repo, wt_dir};

/// Generous: these tests assert what a command did, not how long it took, except the
/// one timeout test, which derives its own bound.
const LONG: Duration = Duration::from_secs(60);

fn shell(command: &str) -> ShellOutcome {
    let dir = tempfile::tempdir().unwrap();
    run_shell(dir.path(), command, &[], LONG)
}

#[test]
fn check_captures_the_last_200_lines_and_exit_code() {
    let outcome = shell("seq 1 500; exit 3");
    assert!(!outcome.ok);
    assert_eq!(outcome.code, Some(3));
    assert!(!outcome.timed_out);
    let lines: Vec<&str> = outcome.tail.lines().collect();
    assert_eq!(lines.len(), CHECK_TAIL_LINES);
    assert_eq!(lines.first(), Some(&"301"));
    assert_eq!(lines.last(), Some(&"500"));

    let green = shell("echo fine");
    assert!(green.ok);
    assert_eq!(green.code, Some(0));
    assert_eq!(green.tail, "fine");
}

#[test]
fn summary_is_the_last_40_lines() {
    let outcome = shell("seq 1 100");
    let last = summary(&outcome.tail);
    let lines: Vec<&str> = last.lines().collect();
    assert_eq!(lines.len(), CHECK_SUMMARY_LINES);
    assert_eq!(lines.first(), Some(&"61"));
    assert_eq!(lines.last(), Some(&"100"));
    assert_eq!(summary("a\nb"), "a\nb", "a short tail is its own summary");
}

#[test]
fn check_merges_stderr() {
    let outcome = shell("echo out; echo err >&2; echo out2 1>&2; echo last");
    assert_eq!(outcome.tail, "out\nerr\nout2\nlast");
    // The shell's own complaint about the command (it goes to the process's stderr,
    // before any redirection applies) is in the same tail.
    let broken = shell("no-such-command-anthrex-m8a10");
    assert!(!broken.ok);
    assert_eq!(broken.code, Some(127));
    assert!(
        broken.tail.contains("no-such-command-anthrex-m8a10"),
        "tail: {:?}",
        broken.tail
    );
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists and may be signalled.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[test]
fn check_timeout_kills_the_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let bg = dir.path().join("bg");
    let timeout = Duration::from_secs(1);
    let started = Instant::now();
    let outcome = run_shell(
        dir.path(),
        &format!("sleep 30 & echo $! > '{}'; wait", bg.display()),
        &[],
        timeout,
    );
    let elapsed = started.elapsed();
    assert!(outcome.timed_out, "{outcome:?}");
    assert!(!outcome.ok);
    assert_eq!(outcome.code, None);
    // The brief's 3 s: the 1 s timeout, plus `OUTPUT_GRACE` (1 s) for the output still
    // in flight after the kill, plus 1 s for spawning and reaping on a loaded machine.
    let bound = Duration::from_secs(3);
    assert!(timeout + OUTPUT_GRACE < bound);
    assert!(elapsed < bound, "timed out after {elapsed:?}");

    let pid: i32 = std::fs::read_to_string(&bg)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let deadline = Instant::now() + KILL_GRACE + Duration::from_secs(1);
    while alive(pid) {
        assert!(
            Instant::now() < deadline,
            "the background sleep {pid} outlived the check's timeout"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_check_that_exits_leaves_no_background_process_behind() {
    let dir = tempfile::tempdir().unwrap();
    let bg = dir.path().join("bg");
    // The background sleep holds the output pipe open after the shell exits.
    let started = Instant::now();
    let outcome = run_shell(
        dir.path(),
        &format!("sleep 120 & echo $! > '{}'; echo done", bg.display()),
        &[],
        LONG,
    );
    assert!(outcome.ok, "{outcome:?}");
    assert!(!outcome.timed_out);
    assert_eq!(outcome.tail, "done");
    assert!(
        started.elapsed() < LONG / 2,
        "waited on the background process's pipe: {:?}",
        started.elapsed()
    );
    let pid: i32 = std::fs::read_to_string(&bg)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let deadline = Instant::now() + KILL_GRACE + Duration::from_secs(1);
    while alive(pid) {
        assert!(
            Instant::now() < deadline,
            "the background sleep {pid} was left running"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn check_tail_survives_invalid_utf8_and_huge_lines() {
    let dir = tempfile::tempdir().unwrap();
    let ascii = dir.path().join("ascii");
    let wide = dir.path().join("wide");
    let bad = dir.path().join("bad");
    let mut long = vec![b'a'; 1 << 20];
    long.push(b'\n');
    std::fs::write(&ascii, &long).unwrap();
    std::fs::write(&wide, format!("{}\n", "世".repeat(1000))).unwrap();
    std::fs::write(&bad, [b'x', 0xff, 0xfe, b'y', b'\n']).unwrap();
    let outcome = run_shell(
        dir.path(),
        "cat ascii; cat wide; cat bad; echo end",
        &[],
        LONG,
    );
    assert!(outcome.ok, "{outcome:?}");
    let lines: Vec<&str> = outcome.tail.split('\n').collect();
    assert_eq!(
        lines.len(),
        4,
        "{:?}",
        &outcome.tail[..outcome.tail.len().min(200)]
    );
    assert_eq!(lines[0].chars().count(), 300);
    assert_eq!(lines[0].len(), 300);
    assert_eq!(lines[1].chars().count(), 300);
    assert_eq!(lines[1].len(), 900);
    assert_eq!(lines[1], "世".repeat(300));
    assert_eq!(lines[2], "x\u{fffd}\u{fffd}y");
    assert_eq!(lines[3], "end");
}

// ---- the test proof ----

const SINGLE_TEST: &str = "sh tests/{test}.sh";
const TEST_PASSED: &str = "PASS {test}";

struct ProofRepo {
    repo: TempRepo,
    _wt: tempfile::TempDir,
    wt: PathBuf,
}

fn proof_repo() -> ProofRepo {
    let repo = repo();
    commit_file(&repo.root, "README", "base\n", "base");
    let (_wt, wt) = wt_dir();
    ProofRepo { repo, _wt, wt }
}

const RESET_TEST: &str = "grep -q reset impl.txt || exit 1\necho 'PASS t_reset'\n";

fn op(p: &ProofRepo, name: &str, red: &str, head: &str, test: &str) -> ProofOp {
    ProofOp {
        root: p.repo.root.clone(),
        path: p.wt.join(format!("runs/r1/{name}.proof")),
        red: red.to_string(),
        head: head.to_string(),
        command: proof_command(SINGLE_TEST, test),
        passed: proof_pattern(TEST_PASSED, test),
        timeout_secs: 60,
        setup: None,
        env: Vec::new(),
    }
}

fn prove(op: &ProofOp) -> ProofRuns {
    run_proof(real_git(), op, T).unwrap_or_else(|err| panic!("run_proof failed: {err:?}"))
}

#[test]
fn proof_accepts_a_real_red_then_green() {
    let p = proof_repo();
    let red = commit_file(&p.repo.root, "tests/t_reset.sh", RESET_TEST, "red");
    let green = commit_file(&p.repo.root, "impl.txt", "fn reset() {}\n", "green");
    let runs = prove(&op(&p, "t1", &red, &green, "t_reset"));
    assert!(runs.red_failed, "{runs:?}");
    assert!(runs.head_passed, "{runs:?}");
    assert!(runs.matched, "{runs:?}");
    assert_eq!(runs.head_tail, "PASS t_reset");
}

#[test]
fn proof_rejects_a_red_that_passes() {
    let p = proof_repo();
    commit_file(&p.repo.root, "impl.txt", "fn reset() {}\n", "impl first");
    let red = commit_file(&p.repo.root, "tests/t_reset.sh", RESET_TEST, "not red");
    let green = commit_file(&p.repo.root, "other.txt", "x\n", "head");
    let runs = prove(&op(&p, "t1", &red, &green, "t_reset"));
    assert!(!runs.red_failed, "{runs:?}");
    assert_eq!(runs.red_tail, "PASS t_reset");
    assert!(!runs.head_passed && !runs.matched, "{runs:?}");
}

#[test]
fn proof_rejects_a_head_that_fails() {
    let p = proof_repo();
    let red = commit_file(&p.repo.root, "tests/t_reset.sh", RESET_TEST, "red");
    let green = commit_file(&p.repo.root, "impl.txt", "no such word\n", "still red");
    let runs = prove(&op(&p, "t1", &red, &green, "t_reset"));
    assert!(runs.red_failed, "{runs:?}");
    assert!(!runs.head_passed, "{runs:?}");
    assert!(!runs.matched, "{runs:?}");
}

#[test]
fn proof_rejects_output_that_does_not_show_the_test() {
    let p = proof_repo();
    let red = commit_file(&p.repo.root, "tests/t_reset.sh", "exit 1\n", "red");
    let green = commit_file(&p.repo.root, "tests/t_reset.sh", "exit 0\n", "silent");
    let runs = prove(&op(&p, "t1", &red, &green, "t_reset"));
    assert!(runs.red_failed, "{runs:?}");
    assert!(runs.head_passed, "{runs:?}");
    assert!(!runs.matched, "{runs:?}");
    assert_eq!(runs.head_tail, "");
}

fn find_named(dir: &Path, name: &str, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name() == name {
            found.push(path.clone());
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            find_named(&path, name, found);
        }
    }
}

#[test]
fn proof_quotes_a_hostile_test_name() {
    let p = proof_repo();
    let red = commit_file(&p.repo.root, "tests/t_reset.sh", RESET_TEST, "red");
    let green = commit_file(&p.repo.root, "impl.txt", "fn reset() {}\n", "green");
    let hostile = "a; touch pwned";
    let proof = op(&p, "t1", &red, &green, hostile);
    assert_eq!(proof.command, "sh tests/'a; touch pwned'.sh");
    let runs = prove(&proof);
    assert!(runs.red_failed, "{runs:?}");
    assert!(!runs.head_passed, "{runs:?}");
    let mut found = Vec::new();
    find_named(&proof.path, "pwned", &mut found);
    find_named(&p.repo.root, "pwned", &mut found);
    assert!(
        found.is_empty(),
        "the test name ran as a command: {found:?}"
    );
    // A regex metacharacter in the name is matched literally.
    assert_eq!(proof_pattern("ok {test}$", "a.b(c"), "ok a\\.b\\(c$");
}

#[test]
fn proof_runs_setup_once_per_new_worktree() {
    let p = proof_repo();
    let red = commit_file(&p.repo.root, "tests/t_reset.sh", RESET_TEST, "red");
    let green = commit_file(&p.repo.root, "impl.txt", "fn reset() {}\n", "green");
    let log_dir = tempfile::tempdir().unwrap();
    let log = log_dir.path().join("setup.log");
    let mut proof = op(&p, "t1", &red, &green, "t_reset");
    proof.setup = Some(format!("echo \"$PROOF_MARK\" >> '{}'", log.display()));
    proof.env = vec![("PROOF_MARK".into(), "ran".into())];
    let count = || {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|line| *line == "ran")
            .count()
    };

    let first = prove(&proof);
    assert!(
        first.red_failed && first.head_passed && first.matched,
        "{first:?}"
    );
    assert_eq!(count(), 1);
    let again = prove(&proof);
    assert!(
        again.red_failed && again.head_passed && again.matched,
        "{again:?}"
    );
    assert_eq!(count(), 1, "setup ran again in a reused proof worktree");

    let mut other = op(&p, "t2", &red, &green, "t_reset");
    other.setup = proof.setup.clone();
    other.env = proof.env.clone();
    prove(&other);
    assert_eq!(count(), 2, "a new proof worktree runs setup");
}

#[test]
fn a_failing_setup_is_reported_and_runs_again_next_time() {
    let p = proof_repo();
    let red = commit_file(&p.repo.root, "tests/t_reset.sh", RESET_TEST, "red");
    let green = commit_file(&p.repo.root, "impl.txt", "fn reset() {}\n", "green");
    let mut proof = op(&p, "t1", &red, &green, "t_reset");
    proof.setup = Some("echo setup broke; exit 4".into());
    match run_proof(real_git(), &proof, T) {
        Err(ProofError::SetupFailed { output }) => assert_eq!(output, "setup broke"),
        other => panic!("expected SetupFailed, got {other:?}"),
    }
    let log_dir = tempfile::tempdir().unwrap();
    let log = log_dir.path().join("setup.log");
    proof.setup = Some(format!("echo fixed > '{}'", log.display()));
    let runs = prove(&proof);
    assert!(
        runs.red_failed && runs.head_passed && runs.matched,
        "{runs:?}"
    );
    assert!(
        log.exists(),
        "a worktree whose setup failed was reused without setup"
    );
}

#[test]
fn a_red_run_that_times_out_is_no_evidence_of_failure() {
    let p = proof_repo();
    let red = commit_file(&p.repo.root, "tests/t_reset.sh", "sleep 30\n", "hangs");
    let green = commit_file(&p.repo.root, "impl.txt", "fn reset() {}\n", "green");
    let mut proof = op(&p, "t1", &red, &green, "t_reset");
    proof.timeout_secs = 1;
    let runs = prove(&proof);
    assert!(!runs.red_failed, "{runs:?}");
    assert!(
        runs.red_tail.ends_with("[anthrex: timed out after 1 s]"),
        "{runs:?}"
    );
    assert!(!runs.head_passed && !runs.matched, "{runs:?}");
}
