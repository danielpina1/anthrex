//! M8a final fix batch F1c (re-review 4, I2): the engine's check and proof commands run
//! code a worker wrote, so when workers are sandboxed they run confined: they may write
//! their checkout, its own object store, their temporary directory and the profile's
//! cache directories, and nothing else — not the user's `.git`, not `$HOME`.
//!
//! Every payload is harmless and aimed inside this test's own temporary directories.

#![cfg(target_os = "macos")]

mod support;

use daemon::run::confine::ConfineSpec;
use daemon::run::exec::{OUTPUT_GRACE, run_confined};
use daemon::run::git::{checkout_repo_dir, prepare_task_worktree};
use daemon::run::proof::{ProofOp, direct, proof_command, proof_pattern, run_proof};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use support::TempRepo;
use support::run_git::{T, commit_file, real_git, repo, wt_dir};

const LONG: Duration = Duration::from_secs(60);
/// Spawning `sandbox-exec`, killing and reaping on a loaded machine, beyond the legal
/// worst case the code derives from `OUTPUT_GRACE` (docs/timing-budgets.md rule 1; the
/// same allowance as `run_exec.rs`'s `SLACK`).
const SLACK: Duration = Duration::from_secs(2);

struct World {
    repo: TempRepo,
    _wt: tempfile::TempDir,
    wt: PathBuf,
    _data: tempfile::TempDir,
    data: PathBuf,
    _outside: tempfile::TempDir,
    /// A stand-in for the user's `$HOME` and for a cache directory, outside everything.
    outside: PathBuf,
}

fn world() -> World {
    let repo = repo();
    commit_file(&repo.root, "README", "base\n", "base");
    let (wt, wt_path) = wt_dir();
    let data = tempfile::tempdir().unwrap();
    let data_path = data.path().canonicalize().unwrap().join("runs/cf01");
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().canonicalize().unwrap();
    World {
        repo,
        _wt: wt,
        wt: wt_path,
        _data: data,
        data: data_path,
        _outside: outside,
        outside: outside_path,
    }
}

impl World {
    fn common(&self) -> PathBuf {
        self.repo.root.join(".git").canonicalize().unwrap()
    }

    fn spec(&self, cache_dirs: &[&Path]) -> ConfineSpec {
        ConfineSpec {
            data_dir: self.data.clone(),
            common_dir: self.common(),
            cache_dirs: cache_dirs.iter().map(|p| p.display().to_string()).collect(),
        }
    }

    fn task(&self) -> PathBuf {
        let path = self.wt.join("runs/cf01/t1");
        let base = support::run_git::head(&self.repo.root);
        prepare_task_worktree(
            real_git(),
            &self.repo.root,
            "anthrex/cf01/t1",
            &base,
            &path,
            &checkout_repo_dir(&self.data, &path),
            T,
        )
        .unwrap();
        path
    }
}

/// Writes that must be denied: the user's object store, the user's index, a file in
/// `$HOME`. Each is attempted, and the command still exits 0.
fn attacks(common: &Path) -> String {
    format!(
        "printf x > '{c}/objects/pwned'; printf x >> '{c}/index'; printf x > \"$HOME/pwned\"; ",
        c = common.display()
    )
}

#[test]
fn a_confined_check_writes_only_its_checkout_tmp_objects_and_caches() {
    let w = world();
    let task = w.task();
    let cache = w.outside.join("cache");
    let home = w.outside.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let index_before = std::fs::read(w.common().join("index")).unwrap();
    let confinement = w.spec(&[&cache]).for_checkout(&task).unwrap();
    let objects = checkout_repo_dir(&w.data, &task).join("git/objects");
    let command = format!(
        "{attacks}printf ok > built.txt && printf ok > \"$TMPDIR/scratch\" && mkdir -p '{cache}' && printf ok > '{cache}/c' && printf ok > '{objects}/allowed' && echo allowed",
        attacks = attacks(&w.common()),
        cache = cache.display(),
        objects = objects.display(),
    );
    let env = vec![("HOME".to_string(), home.display().to_string())];

    let outcome = run_confined(&task, &command, &env, LONG, Some(&confinement));
    assert!(outcome.ok, "{outcome:?}");
    assert!(outcome.tail.ends_with("allowed"), "{}", outcome.tail);
    assert!(
        outcome.tail.contains("Operation not permitted"),
        "{}",
        outcome.tail
    );
    // Denied: nothing reached the user's repository or `$HOME`.
    assert!(!w.common().join("objects/pwned").exists());
    assert_eq!(
        std::fs::read(w.common().join("index")).unwrap(),
        index_before
    );
    assert!(!home.join("pwned").exists());
    // Allowed: the checkout, the temporary directory, the cache, its own objects.
    assert!(task.join("built.txt").is_file());
    assert!(confinement.tmp().join("scratch").is_file());
    assert!(
        confinement
            .tmp()
            .starts_with(checkout_repo_dir(&w.data, &task))
    );
    assert!(cache.join("c").is_file());
    assert!(objects.join("allowed").is_file());

    // Unconfined, the same command reaches them: the test's payloads are live.
    let outcome = run_confined(&task, &attacks(&w.common()), &env, LONG, None);
    assert!(outcome.ok, "{outcome:?}");
    assert!(w.common().join("objects/pwned").exists());
    assert!(home.join("pwned").exists());
}

#[test]
fn a_confined_integration_check_cannot_write_its_git_dir() {
    let w = world();
    let base = support::run_git::head(&w.repo.root);
    let integration = w.wt.join("runs/cf01/integration");
    daemon::run::git::create_run_branch(
        real_git(),
        &w.repo.root,
        "anthrex/cf01/integration",
        &base,
        &integration,
        T,
    )
    .unwrap();
    let admin = w.common().join("worktrees/integration");
    assert!(admin.is_dir());
    let confinement = w.spec(&[]).for_checkout(&integration).unwrap();
    let command = format!(
        "printf x > '{admin}/HEAD'; printf x > '{admin}/index'; printf ok > built.txt && echo done",
        admin = admin.display()
    );
    let head_before = std::fs::read(admin.join("HEAD")).unwrap();
    let outcome = run_confined(&integration, &command, &[], LONG, Some(&confinement));
    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(std::fs::read(admin.join("HEAD")).unwrap(), head_before);
    assert!(integration.join("built.txt").is_file());
}

#[test]
fn a_confined_proof_cannot_write_the_users_git() {
    let w = world();
    let script = format!(
        "{}grep -q reset impl.txt || exit 1\necho 'PASS t_reset'\n",
        attacks(&w.common())
    );
    let red = commit_file(&w.repo.root, "tests/t_reset.sh", &script, "red");
    let green = commit_file(&w.repo.root, "impl.txt", "fn reset() {}\n", "green");
    let home = w.outside.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let path = w.wt.join("runs/cf01/t1.proof");
    let op = ProofOp {
        root: w.repo.root.clone(),
        repo: checkout_repo_dir(&w.data, &path),
        path,
        red,
        head: green,
        command: proof_command("sh tests/{test}.sh", "t_reset"),
        passed: proof_pattern("PASS {test}", "t_reset"),
        timeout_secs: 60,
        setup: None,
        env: vec![("HOME".to_string(), home.display().to_string())],
        confine: Some(w.spec(&[])),
    };
    let index_before = std::fs::read(w.common().join("index")).unwrap();
    let runs = run_proof(real_git(), &op, T, &direct).unwrap();
    assert!(
        runs.red_failed && runs.head_passed && runs.matched,
        "{runs:?}"
    );
    assert!(!w.common().join("objects/pwned").exists());
    assert_eq!(
        std::fs::read(w.common().join("index")).unwrap(),
        index_before
    );
    assert!(!home.join("pwned").exists());
}

/// `sandbox-exec` `exec`s the shell, so the pid the engine times out and kills is the
/// shell's: a confined command that outlives its timeout, with a child of its own, is
/// ended as an unconfined one is.
#[test]
fn a_confined_command_that_times_out_is_killed_with_its_group() {
    let w = world();
    let task = w.task();
    let confinement = w.spec(&[]).for_checkout(&task).unwrap();
    let timeout = Duration::from_secs(1);
    let started = Instant::now();
    let outcome = run_confined(
        &task,
        "sleep 30 & sleep 30",
        &[],
        timeout,
        Some(&confinement),
    );
    let elapsed = started.elapsed();
    assert!(outcome.timed_out, "{outcome:?}");
    assert!(
        elapsed < timeout + OUTPUT_GRACE + SLACK,
        "a confined 1 s timeout returned after {elapsed:?}"
    );
}

/// A checkout the engine cannot confine is not run at all (fails closed).
#[test]
fn a_checkout_that_cannot_be_confined_is_refused() {
    let w = world();
    let cache = w.repo.root.clone();
    let task = w.task();
    let err = w.spec(&[&cache]).for_checkout(&task).unwrap_err();
    assert!(err.contains("overlaps the git common directory"), "{err}");
}
